use serde_json::Value;

use crate::JobId;

/// Lifecycle status of an [`Event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStatus {
    /// Intermediate progress emitted by a running job.
    Progress,
    /// Final success; `payload` is the job's serialized `Output`.
    Done,
    /// Final failure; `payload` is `{ "error": "..." }`.
    Failed,
}

/// One message on the engine's result stream.
#[derive(Debug, Clone)]
pub struct Event {
    /// The job that produced this event.
    pub id: JobId,
    /// The job the UI started; everything in a chain shares it.
    pub root: JobId,
    /// Catalogue kind of the producing job.
    pub kind: String,
    pub payload: Value,
    pub status: EventStatus,
}