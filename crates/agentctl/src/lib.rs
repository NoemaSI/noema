//! Agent daemon for the Noema guest VM.
//!
//! Wire types ([`AgentRequest`], [`AgentEvent`]) plus two features:
//! - `server` (default, runs in the guest): a jobctl-style registry of typed
//!   [`server::AgentHandler`]s exposed over HTTP/SSE.
//! - `client` (host side): [`client::AgentClient`], a typed façade that hides
//!   the SSE framing from callers.

use serde::{Deserialize, Serialize};

/// Name of the primary goal handler, shared by client and server.
pub const GOAL_HANDLER: &str = "goal";

/// Task submitted to the agent server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub goal: String,
    pub criteria: Vec<String>,
}

/// One streamed agent event (SSE `data:` payload, JSON, one line).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Acceptance criteria derived by the agent from the goal.
    Criteria { items: Vec<String> },
    Done { summary: String },
    Failed { error: String },
}

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
pub mod handlers;
#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub use client::{AgentClient, AgentError, AgentRun};
#[cfg(feature = "server")]
pub use server::{AgentHandler, AgentServer, Emit};