//! Typed host-side client for the agent server. Hides the SSE wire format.

use std::collections::VecDeque;
use std::fmt;

use futures_util::StreamExt;

use crate::{AgentEvent, AgentRequest};

pub use crate::GOAL_HANDLER;

/// Client errors.
#[derive(Debug)]
pub enum AgentError {
    /// Transport-level failure (connect, TLS, body read).
    Request(reqwest::Error),
    /// Server answered with a non-success status code.
    Status(u16),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Request(e) => write!(f, "request failed: {e}"),
            AgentError::Status(status) => write!(f, "server answered {status}"),
        }
    }
}

impl std::error::Error for AgentError {}

/// Typed handle to one agent server, e.g. `AgentClient::new("http://127.0.0.1:3333")`.
pub struct AgentClient {
    base: String,
    http: reqwest::Client,
}

impl AgentClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Run the goal handler (rig + opencode agent; currently a stub).
    pub async fn run_goal(&self, request: AgentRequest) -> Result<AgentRun, AgentError> {
        self.run(GOAL_HANDLER, &request).await
    }

    /// Run any registered handler by name with a typed payload.
    pub async fn run<P: serde::Serialize>(
        &self,
        handler: &str,
        payload: &P,
    ) -> Result<AgentRun, AgentError> {
        let response = self
            .http
            .post(format!("{}/v1/agent/{handler}/run", self.base))
            .json(payload)
            .send()
            .await
            .map_err(AgentError::Request)?;
        if !response.status().is_success() {
            return Err(AgentError::Status(response.status().as_u16()));
        }
        Ok(AgentRun {
            inner: Box::pin(response.bytes_stream()),
            buffer: String::new(),
            queue: VecDeque::new(),
            summary: None,
            error: None,
            criteria: Vec::new(),
        })
    }
}

/// A running agent task, yielding [`AgentEvent`]s until the stream ends.
pub struct AgentRun {
    inner: futures_util::stream::BoxStream<'static, Result<bytes::Bytes, reqwest::Error>>,
    buffer: String,
    queue: VecDeque<AgentEvent>,
    summary: Option<String>,
    error: Option<String>,
    criteria: Vec<String>,
}

impl AgentRun {
    /// Next event, or `Ok(None)` at end of stream.
    pub async fn recv(&mut self) -> Result<Option<AgentEvent>, AgentError> {
        loop {
            if let Some(event) = self.queue.pop_front() {
                return Ok(Some(event));
            }
            let chunk = self
                .inner
                .next()
                .await
                .transpose()
                .map_err(AgentError::Request)?;
            let Some(chunk) = chunk else {
                return Ok(None);
            };
            self.buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..pos + 2).collect();
                for line in frame.lines().filter(|l| l.starts_with("data:")) {
                    let Ok(event) = serde_json::from_str(line["data:".len()..].trim()) else {
                        continue;
                    };
                    match &event {
                        AgentEvent::Done { summary } => self.summary = Some(summary.clone()),
                        AgentEvent::Criteria { items } => self.criteria = items.clone(),
                        AgentEvent::Failed { error } => self.error = Some(error.clone()),
                        _ => {}
                    }
                    self.queue.push_back(event);
                }
            }
        }
    }

    /// Summary carried by the terminal `Done` event, if seen.
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    /// Acceptance criteria emitted by the agent, if seen.
    pub fn criteria(&self) -> &[String] {
        &self.criteria
    }

    /// Error carried by the terminal `Failed` event, if seen.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}