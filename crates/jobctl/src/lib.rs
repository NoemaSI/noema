//! `jobctl` — a catalogue-driven job engine.
//!
//! Jobs are self-describing [`JobDefinition`]s registered in a catalogue.
//! The UI only ever talks to a typed façade (e.g. [`EngineHandle::analyze_intent`]);
//! internal jobs chain via [`JobCtx::run_child`]. Everything flows back on a
//! single [`Results`] stream of [`Event`]s tagged with `{ id, root, kind }`.
//!
//! ```ignore
//! let (engine, mut results) = engine()
//!     .register::<Intent>()
//!     .queue_capacity(64)
//!     .worker_threads(1)
//!     .spawn();
//!
//! // UI: submit through the typed façade
//! engine.analyze_intent("foo".into(), "characterize Kd".into())?;
//!
//! // UI: pump events inside gpui
//! cx.spawn(async move |this, cx| {
//!     while let Some(ev) = results.recv().await {
//!         // match (ev.status, ev.kind.as_str()) { ... }
//!     }
//! });
//! ```

mod catalog;
mod ctx;
mod engine;
mod events;
mod services;

pub mod jobs;

/// Wire protocol of the guest agent, re-exported so the UI can type-check
/// streamed events without depending on `agentctl` directly.
pub use agentctl as protocol;

pub use catalog::{JobDefinition, JobInfo};
pub use ctx::JobCtx;
pub use engine::{Results, engine, EngineBuilder, EngineHandle};
pub use events::{Event, EventStatus};
pub use services::Services;

pub type JobId = u64;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("job failed: {0}")]
    JobFailed(String),
    #[error("unknown job kind: {0}")]
    UnknownKind(String),
    #[error("job queue is full")]
    QueueFull,
    #[error("engine is shut down")]
    ShuttingDown,
    #[error("invalid payload: {0}")]
    Payload(#[from] serde_json::Error),
}
