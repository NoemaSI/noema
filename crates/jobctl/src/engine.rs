use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

use crate::catalog::{ErasedJob, JobAdapter, JobInfo};
use crate::ctx::JobCtx;
use crate::events::{Event, EventStatus};
use crate::services::Services;
use crate::{EngineError, JobId};

pub(crate) struct JobRequest {
    pub(crate) id: JobId,
    pub(crate) root: JobId,
    pub(crate) kind: String,
    pub(crate) payload: Value,
}

pub(crate) struct Shared {
    pub(crate) registry: HashMap<String, Arc<dyn ErasedJob>>,
    pub(crate) specs: Vec<JobInfo>,
    pub(crate) queue: async_channel::Sender<JobRequest>,
    pub(crate) results: async_channel::Sender<Event>,
    pub(crate) waiters: Mutex<HashMap<JobId, oneshot::Sender<Result<Value, EngineError>>>>,
    pub(crate) next_id: AtomicU64,
    pub(crate) services: Services,
    #[allow(dead_code)]
    pub(crate) runtime: Arc<Runtime>,
}

impl Shared {
    pub(crate) fn new_id(&self) -> JobId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

/// Start here: `engine().register::<...>().spawn()`.
pub fn engine() -> EngineBuilder {
    EngineBuilder::default()
}

#[derive(Default)]
pub struct EngineBuilder {
    jobs: HashMap<String, Arc<dyn ErasedJob>>,
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    queue_capacity: Option<usize>,
    worker_threads: Option<usize>,
}

impl EngineBuilder {
    pub fn register<D: crate::JobDefinition>(mut self) -> Self {
        self.jobs
            .insert(D::KIND.to_string(), Arc::new(JobAdapter::<D>::new()));
        self
    }

    pub fn service<T: Send + Sync + 'static>(mut self, service: T) -> Self {
        self.services.insert(TypeId::of::<T>(), Arc::new(service));
        self
    }

    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.queue_capacity = Some(capacity);
        self
    }

    pub fn worker_threads(mut self, threads: usize) -> Self {
        self.worker_threads = Some(threads);
        self
    }

    pub fn spawn(self) -> (EngineHandle, Results) {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(self.worker_threads.unwrap_or(2))
                .thread_name("jobctl-worker")
                .enable_all()
                .build()
                .expect("failed to start jobctl runtime"),
        );

        let (queue_tx, queue_rx) =
            async_channel::bounded::<JobRequest>(self.queue_capacity.unwrap_or(64));
        let (results_tx, results_rx) = async_channel::unbounded::<Event>();

        let specs = self.jobs.values().map(|job| job.info()).collect();
        let shared = Arc::new(Shared {
            registry: self.jobs,
            specs,
            queue: queue_tx,
            results: results_tx,
            waiters: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            services: Services::new(self.services),
            runtime: runtime.clone(),
        });

        {
            let shared = shared.clone();
            runtime.spawn(async move {
                while let Ok(request) = queue_rx.recv().await {
                    let shared = shared.clone();
                    tokio::spawn(async move {
                        run_job(shared, request).await;
                    });
                }
            });
        }

        (
            EngineHandle {
                shared: shared.clone(),
            },
            Results {
                rx: results_rx,
                runtime,
            },
        )
    }
}

/// Cloneable handle for submitting work and inspecting the catalogue.
#[derive(Clone)]
pub struct EngineHandle {
    shared: Arc<Shared>,
}

impl EngineHandle {
    /// All jobs registered in the catalogue.
    pub fn catalog(&self) -> Vec<JobInfo> {
        self.shared.specs.clone()
    }

    /// Enqueue a job by catalogue kind; the job becomes its own root.
    pub async fn submit(&self, kind: &str, payload: Value) -> Result<JobId, EngineError> {
        if !self.shared.registry.contains_key(kind) {
            return Err(EngineError::UnknownKind(kind.to_string()));
        }
        let id = self.shared.new_id();
        self.shared
            .queue
            .send(JobRequest {
                id,
                root: id,
                kind: kind.to_string(),
                payload,
            })
            .await
            .map_err(|_| EngineError::ShuttingDown)?;
        Ok(id)
    }

    /// Non-blocking variant of [`Self::submit`] for UI callbacks.
    pub fn try_submit(&self, kind: &str, payload: Value) -> Result<JobId, EngineError> {
        if !self.shared.registry.contains_key(kind) {
            return Err(EngineError::UnknownKind(kind.to_string()));
        }
        let id = self.shared.new_id();
        self.shared
            .queue
            .try_send(JobRequest {
                id,
                root: id,
                kind: kind.to_string(),
                payload,
            })
            .map_err(|e| match e {
                async_channel::TrySendError::Full(_) => EngineError::QueueFull,
                async_channel::TrySendError::Closed(_) => EngineError::ShuttingDown,
            })?;
        Ok(id)
    }
}

/// Receive engine events. Dropping it (together with the last
/// [`EngineHandle`]) lets the runtime wind down.
pub struct Results {
    rx: async_channel::Receiver<Event>,
    #[allow(dead_code)]
    runtime: Arc<Runtime>,
}

impl Results {
    /// Await the next event. Returns `None` once the engine is shut down and
    /// the queue is drained.
    pub async fn recv(&mut self) -> Option<Event> {
        self.rx.recv().await.ok()
    }
}

async fn run_job(shared: Arc<Shared>, request: JobRequest) {
    let outcome = match shared.registry.get(&request.kind) {
        Some(job) => {
            let ctx = JobCtx::new(request.id, request.root, request.kind.clone(), shared.clone());
            job.run(ctx, request.payload.clone(), shared.services.clone())
                .await
        }
        None => Err(EngineError::UnknownKind(request.kind.clone())),
    };

    let (payload, status, waiter) = match outcome {
        Ok(value) => (value.clone(), EventStatus::Done, Ok(value)),
        Err(err) => (
            serde_json::json!({ "error": err.to_string() }),
            EventStatus::Failed,
            Err(EngineError::JobFailed(err.to_string())),
        ),
    };

    let _ = shared
        .results
        .send(Event {
            id: request.id,
            root: request.root,
            kind: request.kind.clone(),
            payload,
            status,
        })
        .await;

    if let Some(tx) = shared.waiters.lock().unwrap().remove(&request.id) {
        let _ = tx.send(waiter);
    }
}