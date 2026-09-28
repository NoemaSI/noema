use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::engine::Shared;
use crate::events::{Event, EventStatus};
use crate::{EngineError, JobId};

/// Handle a running job uses to talk back to the engine: emit progress,
/// submit sibling jobs, or chain child jobs.
#[derive(Clone)]
pub struct JobCtx {
    id: JobId,
    root: JobId,
    kind: String,
    shared: Arc<Shared>,
}

impl JobCtx {
    pub(crate) fn new(id: JobId, root: JobId, kind: String, shared: Arc<Shared>) -> Self {
        Self {
            id,
            root,
            kind,
            shared,
        }
    }

    /// The id of this job.
    pub fn id(&self) -> JobId {
        self.id
    }

    /// The id of the job the UI started; everything in a chain shares it.
    pub fn root(&self) -> JobId {
        self.root
    }

    /// Stream a progress event to the UI.
    pub async fn emit<E: Serialize>(&self, event: &E) -> Result<(), EngineError> {
        let payload = serde_json::to_value(event)?;
        self.shared
            .results
            .send(Event {
                id: self.id,
                root: self.root,
                kind: self.kind.clone(),
                payload,
                status: EventStatus::Progress,
            })
            .await
            .map_err(|_| EngineError::ShuttingDown)
    }

    /// Enqueue another catalogue job; returns its id without waiting.
    pub async fn submit(&self, kind: &str, payload: Value) -> Result<JobId, EngineError> {
        let id = self.shared.new_id();
        self.send(id, kind, payload).await?;
        Ok(id)
    }

    /// Non-blocking variant of [`Self::submit`].
    pub fn try_submit(&self, kind: &str, payload: Value) -> Result<JobId, EngineError> {
        let id = self.shared.new_id();
        self.try_send(id, kind, payload)?;
        Ok(id)
    }

    /// Submit a child job and await its completion, returning its output.
    pub async fn run_child(&self, kind: &str, payload: Value) -> Result<Value, EngineError> {
        if !self.shared.registry.contains_key(kind) {
            return Err(EngineError::UnknownKind(kind.to_string()));
        }
        let id = self.shared.new_id();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.shared.waiters.lock().unwrap().insert(id, tx);
        self.send(id, kind, payload).await?;
        rx.await.map_err(|_| EngineError::ShuttingDown)?
    }

    async fn send(&self, id: JobId, kind: &str, payload: Value) -> Result<(), EngineError> {
        self.shared
            .queue
            .send(crate::engine::JobRequest {
                id,
                root: self.root,
                kind: kind.to_string(),
                payload,
            })
            .await
            .map_err(|_| EngineError::ShuttingDown)
    }

    fn try_send(&self, id: JobId, kind: &str, payload: Value) -> Result<(), EngineError> {
        self.shared
            .queue
            .try_send(crate::engine::JobRequest {
                id,
                root: self.root,
                kind: kind.to_string(),
                payload,
            })
            .map_err(|e| match e {
                async_channel::TrySendError::Full(_) => EngineError::QueueFull,
                async_channel::TrySendError::Closed(_) => EngineError::ShuttingDown,
            })
    }
}