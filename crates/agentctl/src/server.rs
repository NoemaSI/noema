//! Guest-side agent server: a jobctl-style registry of typed handlers
//! exposed over HTTP/SSE. All wire encoding lives here.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::{Path, State};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::AgentEvent;



/// Stream sink handed to [`AgentHandler::run`]; emits events to the caller.
#[derive(Clone)]
pub struct Emit(UnboundedSender<AgentEvent>);

impl Emit {
    pub fn emit(&self, event: AgentEvent) {
        let _ = self.0.send(event);
    }
}

/// A catalogue entry on the agent side, mirroring `jobctl::JobDefinition`.
///
/// The returned `String` becomes the terminal [`AgentEvent::Done`] summary;
/// an `Err` becomes [`AgentEvent::Failed`].
pub trait AgentHandler: Send + Sync + 'static {
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    type Payload: DeserializeOwned + Send + 'static;

    fn run(
        payload: Self::Payload,
        emit: Emit,
    ) -> impl Future<Output = Result<String, String>> + Send;
}

trait ErasedHandler: Send + Sync {
    fn start(&self, payload: serde_json::Value, emit: Emit);
}

struct Handler<H>(std::marker::PhantomData<H>);

impl<H: AgentHandler> ErasedHandler for Handler<H> {
    fn start(&self, payload: serde_json::Value, emit: Emit) {
        let payload = match serde_json::from_value::<H::Payload>(payload) {
            Ok(payload) => payload,
            Err(err) => {
                emit.emit(AgentEvent::Failed {
                    error: format!("invalid payload for {}: {err}", H::NAME),
                });
                return;
            }
        };
        let terminal = emit.clone();
        tokio::spawn(async move {
            match H::run(payload, emit).await {
                Ok(summary) => terminal.emit(AgentEvent::Done { summary }),
                Err(error) => terminal.emit(AgentEvent::Failed { error }),
            }
        });
    }
}

struct Entry {
    description: &'static str,
    handler: Arc<dyn ErasedHandler>,
}

/// Registry of handlers; build with [`AgentServer::builder`].
#[derive(Clone)]
pub struct AgentServer {
    entries: Arc<HashMap<&'static str, Entry>>,
}

pub struct AgentServerBuilder {
    entries: HashMap<&'static str, Entry>,
}

impl AgentServer {
    pub fn builder() -> AgentServerBuilder {
        AgentServerBuilder {
            entries: HashMap::new(),
        }
    }

    /// Serve until shutdown is requested.
    pub async fn serve(self, listen: &str) {
        let app = self.router();
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .unwrap_or_else(|e| panic!("bind {listen}: {e}"));
        eprintln!("agentctl serving on {listen}");
        axum::serve(listener, app).await.expect("server");
    }

    /// Bind `listen` and serve from a spawned task; returns once the
    /// listener is live. Lets embedders (jobctl's local agent mode) run the
    /// server in-process instead of spawning a binary.
    pub async fn spawn_serving(
        self,
        listen: &str,
    ) -> Result<tokio::task::JoinHandle<()>, std::io::Error> {
        let listener = tokio::net::TcpListener::bind(listen).await?;
        let app = self.router();
        Ok(tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("agent server error: {e}");
            }
        }))
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/healthz", get(healthz))
            .route("/v1/agent/catalogue", get(catalogue))
            .route("/v1/agent/{name}/run", post(run))
            .with_state(self)
    }
}

/// `ok <sha>` where `<sha>` is the source digest embedded at build time
/// ("unknown" for host builds); lets the host detect a stale running binary.
async fn healthz() -> impl IntoResponse {
    format!("ok {}", env!("AGENTCTL_SHA"))
}

impl AgentServerBuilder {
    pub fn register<H: AgentHandler>(mut self) -> Self {
        self.entries.insert(
            H::NAME,
            Entry {
                description: H::DESCRIPTION,
                handler: Arc::new(Handler::<H>(std::marker::PhantomData)),
            },
        );
        self
    }

    pub fn build(self) -> AgentServer {
        AgentServer {
            entries: Arc::new(self.entries),
        }
    }

    pub async fn serve(self, listen: &str) {
        self.build().serve(listen).await
    }

    pub async fn spawn_serving(
        self,
        listen: &str,
    ) -> Result<tokio::task::JoinHandle<()>, std::io::Error> {
        self.build().spawn_serving(listen).await
    }
}

async fn catalogue(State(server): State<AgentServer>) -> impl IntoResponse {
    let mut entries: Vec<_> = server
        .entries
        .iter()
        .map(|(name, e)| serde_json::json!({ "name": name, "description": e.description }))
        .collect();
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(entries)
}

async fn run(
    State(server): State<AgentServer>,
    Path(name): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> axum::response::Response {
    let Some(entry) = server.entries.get(name.as_str()) else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            format!("unknown agent handler {name}"),
        )
            .into_response();
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    entry.handler.start(payload, Emit(tx));
    Sse::new(EventStream(UnboundedReceiverStream::new(rx))).into_response()
}

/// Bridges the handler event channel to axum SSE frames.
struct EventStream(UnboundedReceiverStream<AgentEvent>);

impl tokio_stream::Stream for EventStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.0).poll_next(cx) {
            Poll::Ready(Some(event)) => Poll::Ready(Some(Ok(json_event(event)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

fn json_event(event: AgentEvent) -> Event {
    Event::default()
        .json_data(event)
        .unwrap_or_else(|_| Event::default().data(r#"{"type":"failed","error":"serialise"}"#))
}

