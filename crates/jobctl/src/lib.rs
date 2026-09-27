//! `jobctl` — a small job engine.
//!
//! The engine runs a tokio runtime on background threads. The UI submits jobs
//! through a [`Submitter`] (non-blocking, cheap to clone) and receives results
//! on a [`Results`] endpoint. The channels are `async-channel`, which is
//! executor-agnostic, so the UI side can `await` them inside `cx.spawn` in
//! gpui while the workers run on tokio.
//!
//! ```ignore
//! // app startup (main.rs)
//! let (submitter, mut results) = Engine::spawn(MyHandler, 64, 2);
//!
//! // somewhere in a gpui view: submit a job
//! let id = submitter.submit(MyJob::Run { name: "train".into() })?;
//!
//! // somewhere in a gpui view: pump results back into the view
//! cx.spawn(|this, mut cx| async move {
//!     while let Some((id, outcome)) = results.recv().await {
//!         this.update(&mut cx, |view, cx| {
//!             view.on_job_result(id, outcome);
//!             cx.notify();
//!         })
//!         .ok()?;
//!     }
//!     Some(())
//! })
//! .detach();
//! ```

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub mod intent;

pub type JobId = u64;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("invalid input provided: {0}")]
    InvalidInput(String),
    #[error("job failed: {0}")]
    JobFailed(String),
    #[error("job queue is full")]
    QueueFull,
    #[error("engine is shut down")]
    ShuttingDown,
}

/// The unit of work. Implement this for your own handler type.
pub trait Handler: Send + Sync + 'static {
    type Job: Send + 'static;
    type Output: Send + 'static;

    /// Runs one job. Spawned onto tokio, so it may be long-running and
    /// concurrent with other jobs.
    fn handle(
        &self,
        id: JobId,
        job: Self::Job,
    ) -> impl Future<Output = Result<Self::Output, EngineError>> + Send;
}

/// Owns the tokio runtime; dropping the last [`Submitter`] and [`Results`]
/// shuts it down.
pub struct Engine<H: Handler> {
    // keeps the tokio runtime alive while this endpoint exists
    #[allow(dead_code)]
    runtime: Arc<tokio::runtime::Runtime>,
    _marker: std::marker::PhantomData<H>,
}

/// Submit jobs to the engine. Clone freely; safe to call from the UI thread.
pub struct Submitter<H: Handler> {
    jobs_tx: async_channel::Sender<(JobId, H::Job)>,
    next_id: Arc<AtomicU64>,
    // keeps the tokio runtime alive while this endpoint exists
    #[allow(dead_code)]
    runtime: Arc<tokio::runtime::Runtime>,
}

impl<H: Handler> Clone for Submitter<H> {
    fn clone(&self) -> Self {
        Self {
            jobs_tx: self.jobs_tx.clone(),
            next_id: self.next_id.clone(),
            runtime: self.runtime.clone(),
        }
    }
}

impl<H: Handler> Submitter<H> {
    /// Enqueue a job; returns its id. Non-blocking: fails with
    /// [`EngineError::QueueFull`] if the bounded queue is saturated, or
    /// [`EngineError::ShuttingDown`] if the engine stopped.
    pub fn submit(&self, job: H::Job) -> Result<JobId, EngineError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.jobs_tx
            .try_send((id, job))
            .map_err(|e| match e {
                async_channel::TrySendError::Full(_) => EngineError::QueueFull,
                async_channel::TrySendError::Closed(_) => EngineError::ShuttingDown,
            })?;
        Ok(id)
    }
}

/// Receive completed jobs as `(id, outcome)` pairs.
pub struct Results<H: Handler> {
    results_rx: async_channel::Receiver<(JobId, Result<H::Output, EngineError>)>,
    // keeps the tokio runtime alive while this endpoint exists
    #[allow(dead_code)]
    runtime: Arc<tokio::runtime::Runtime>,
}

impl<H: Handler> Results<H> {
    /// Await the next result. Returns `None` once the engine is shut down and
    /// the queue is drained.
    pub async fn recv(&mut self) -> Option<(JobId, Result<H::Output, EngineError>)> {
        self.results_rx.recv().await.ok()
    }
}

impl<H: Handler> Engine<H> {
    /// Start the engine: `worker_threads` tokio workers, job queue bounded to
    /// `queue_capacity`. Returns the submit endpoint and the result stream.
    pub fn spawn(
        handler: H,
        queue_capacity: usize,
        worker_threads: usize,
    ) -> (Submitter<H>, Results<H>) {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(worker_threads)
                .thread_name("jobctl-worker")
                .enable_all()
                .build()
                .expect("failed to start jobctl runtime"),
        );

        let (jobs_tx, jobs_rx) = async_channel::bounded::<(JobId, H::Job)>(queue_capacity);
        let (results_tx, results_rx) =
            async_channel::unbounded::<(JobId, Result<H::Output, EngineError>)>();

        let handler = Arc::new(handler);
        let supervisor_runtime = runtime.clone();
        supervisor_runtime.spawn(async move {
            while let Ok((id, job)) = jobs_rx.recv().await {
                let handler = handler.clone();
                let results_tx = results_tx.clone();
                tokio::spawn(async move {
                    let outcome = handler.handle(id, job).await;
                    let _ = results_tx.send((id, outcome)).await;
                });
            }
        });

        let submitter = Submitter {
            jobs_tx,
            next_id: Arc::new(AtomicU64::new(1)),
            runtime: runtime.clone(),
        };
        let results = Results { results_rx, runtime };
        (submitter, results)
    }
}
