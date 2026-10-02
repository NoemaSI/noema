//! SSE debug viewer for the discovery loop's agent streams.
//!
//! A tiny embedded web server: `GET /` serves the viewer page,
//! `GET /events` streams every author-agent message (and a few
//! loop-lifecycle notes) as a server-sent event. The agent publishes
//! each stream item (text deltas, tool calls, reasoning, usage) into
//! a global broadcast channel; the SSE handler forwards them to any
//! connected browser.
//!
//! Feature-gated (`viewer`): no server, thread, or channel exists
//! without it, and [`crate::agent`] degrades to plain stdout logging.

use std::sync::OnceLock;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// One published message, rendered by the viewer page.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// "info" | "prompt" | "image" | "text" | "reasoning" |
    /// "reasoning_delta" | "tool_call" | "tool_call_delta" |
    /// "tool_done" | "usage" | "error" | "unknown"
    pub kind: &'static str,

    pub content: String,
}

fn sender() -> &'static broadcast::Sender<Event> {
    static SENDER: OnceLock<broadcast::Sender<Event>> = OnceLock::new();

    SENDER.get_or_init(|| broadcast::channel(4096).0)
}

/// Publish one message to every connected viewer. Cheap no-op when no
/// browser is watching.
pub fn publish(kind: &'static str, content: impl Into<String>) {
    let _ = sender().send(Event {
        kind,
        content: content.into(),
    });
}

/// The viewer's default address (`SKILL_VIEWER_ADDR` overrides).
pub fn default_addr() -> String {
    std::env::var("SKILL_VIEWER_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9935".to_string())
}

/// Start the viewer once per process on the default address, so the
/// authoring stages (`skill new`, `draft`) get a live feed without
/// the scientist starting `skill viewer` in a second terminal. A
/// second call (gate `redo`) is a no-op; a port conflict degrades to
/// a note, not a panic — the CLI itself must not die over a viewer.
pub fn serve_local() {
    static START: std::sync::Once = std::sync::Once::new();

    START.call_once(|| {
        let addr = default_addr();

        let probe = std::net::TcpListener::bind(&addr);

        match probe {
            Ok(_) => {
                drop(probe);

                publish("info", format!("skill viewer listening on {addr}"));
                serve(&addr);

                println!("skill: viewer on http://{addr}/");
            }
            Err(_) => println!("skill: viewer not started ({addr} busy) — use `skill viewer` elsewhere"),
        }
    });
}

/// Start the viewer server on a background thread (its own tokio
/// runtime). Returns immediately; the agent loop keeps publishing
/// regardless of whether anyone is connected.
pub fn serve(addr: &str) {
    let addr = addr.to_string();

    std::thread::Builder::new()
        .name("skill-viewer".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("building the viewer tokio runtime");

            runtime.block_on(async move {
                let listener = TcpListener::bind(&addr)
                    .await
                    .unwrap_or_else(|error| panic!("binding the viewer to {addr}: {error}"));

                eprintln!("[viewer] http://{addr}/");

                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        continue;
                    };

                    tokio::spawn(handle(stream));
                }
            });
        })
        .expect("spawning the viewer thread");
}

async fn handle(mut stream: TcpStream) {
    // Read until the end of the request headers (single small GET in
    // practice).
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        let Ok(n) = stream.read(&mut chunk).await else {
            return;
        };

        if n == 0 {
            return;
        }

        buffer.extend_from_slice(&chunk[..n]);

        if buffer.windows(4).any(|window| window == b"\r\n\r\n") || buffer.len() > 8192 {
            break;
        }
    }

    let request = String::from_utf8_lossy(&buffer);

    let path = request.split_whitespace().nth(1).unwrap_or("/");

    match path {
        "/events" => sse(stream).await,
        "/" | "/index.html" => page(stream).await,
        _ => {
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        }
    }
}

async fn sse(mut stream: TcpStream) {
    let mut receiver = sender().subscribe();

    let head = "HTTP/1.1 200 OK\r\n\
                Content-Type: text/event-stream\r\n\
                Cache-Control: no-cache\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: keep-alive\r\n\
                \r\n";

    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }

    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(15));

    loop {
        tokio::select! {
            item = receiver.recv() => match item {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_default();

                    if stream
                        .write_all(format!("data: {data}\n\n").as_bytes())
                        .await
                        .is_err()
                    {
                        return;
                    }

                    let _ = stream.flush().await;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            },

            _ = keepalive.tick() => {
                if stream.write_all(b": ping\n\n").await.is_err() {
                    return;
                }

                let _ = stream.flush().await;
            }
        }
    }
}

async fn page(mut stream: TcpStream) {
    let body = viewer_page();

    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n{}",
        body.len(),
        body,
    );

    let _ = stream.write_all(response.as_bytes()).await;
}

fn viewer_page() -> String {
    r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>skill discovery loop</title>
<style>
  body { background: #11151c; color: #d6deeb; font: 13px/1.45 ui-monospace, Menlo, Consolas, monospace; margin: 0; }
  h1 { font-size: 14px; padding: 10px 14px; margin: 0; background: #0d1017; border-bottom: 1px solid #1f2630; position: sticky; top: 0; }
  h1 small { color: #5c6a7e; font-weight: normal; }
  #events { padding: 10px 14px 40px; }
  .ev { margin: 4px 0; white-space: pre-wrap; word-break: break-word; }
  .ev .kind { color: #5c6a7e; font-size: 10px; text-transform: uppercase; margin-right: 8px; }
  .ev.info { color: #4dd0e1; }
  .ev.text { color: #d6deeb; }
  .ev.reasoning, .ev.reasoning_delta { color: #7e8695; font-style: italic; }
  .ev.tool_call { color: #82aaff; }
  .ev.tool_call_delta { color: #5f7ec9; }
  .ev.tool_done { color: #addb67; }
  .ev.usage { color: #e5c07b; }
  .ev.prompt { color: #c792ea; border-left: 2px solid #c792ea; padding-left: 8px; }
  .ev.error { color: #ef5350; }
  .ev.unknown { color: #ff5874; }
</style>
</head>
<body>
<h1>skill discovery loop <small id="count">0 events</small></h1>
<div id="events"></div>
<script>
var events = document.getElementById('events');
var count = document.getElementById('count');
var n = 0;
var last = null;  // coalesce consecutive small deltas into one node

function show(kind, content) {
  n += 1;
  count.textContent = n + ' events';

  if (kind === 'image') {
    var box = document.createElement('div');
    box.className = 'ev image';
    var img = document.createElement('img');
    img.src = 'data:image/png;base64,' + content;
    img.style.cssText = 'max-width:900px;width:100%;border:1px solid #1f2630;';
    box.append(img);
    events.append(box);
    last = null;
    var at = window.innerHeight + window.scrollY >= document.body.scrollHeight - 60;
    if (at) window.scrollTo(0, document.body.scrollHeight);
    return;
  }

  var coalesce = (kind === 'text' || kind === 'reasoning_delta' || kind === 'tool_call_delta') && last && last.kind === kind;

  if (coalesce) {
    last.pre.textContent += content;
  } else {
    var div = document.createElement('div');
    div.className = 'ev ' + kind;
    var tag = document.createElement('span');
    tag.className = 'kind';
    tag.textContent = kind;
    var pre = document.createElement('pre');
    pre.style.cssText = 'display:inline;margin:0;font:inherit;white-space:pre-wrap;';
    pre.textContent = content;
    div.append(tag, pre);
    events.append(div);
    last = { kind: kind, pre: pre };
  }

  var atBottom = window.innerHeight + window.scrollY >= document.body.scrollHeight - 60;
  if (atBottom) window.scrollTo(0, document.body.scrollHeight);
}

var source = new EventSource('/events');
source.onmessage = function (e) {
  var ev = JSON.parse(e.data);
  show(ev.kind, ev.content);
};
</script>
</body>
</html>
"#.to_string()
}