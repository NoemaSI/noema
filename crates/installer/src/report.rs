//! Progress reporting.
//!
//! Human mode writes readable lines to stderr. `--json` mode emits one
//! JSON object per line to stdout so the Noema UI (or a CI script) can
//! follow the setup without parsing prose.

use std::io::Write;

#[derive(Debug, Clone, Copy)]
pub struct Report {
    json: bool,
}

impl Report {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    pub fn json(&self) -> bool {
        self.json
    }

    /// A step started: `{"event":"step","component":"pixi","action":"install"}`.
    pub fn step(&self, component: &str, action: &str, detail: &str) {
        if self.json {
            emit(serde_json::json!({
                "event": "step", "component": component,
                "action": action, "detail": detail,
            }));
        } else {
            eprintln!("{component}: {action} — {detail}");
        }
    }

    /// A component finished successfully.
    pub fn done(&self, component: &str, version: &str) {
        if self.json {
            emit(serde_json::json!({
                "event": "done", "component": component, "version": version,
            }));
        } else {
            eprintln!("{component}: ok ({version})");
        }
    }

    /// An informational line (prerequisite notes, summaries).
    pub fn note(&self, text: &str) {
        if self.json {
            emit(serde_json::json!({ "event": "note", "text": text }));
        } else {
            eprintln!("{text}");
        }
    }

    /// A failure for one component; setup continues with the others.
    pub fn failure(&self, component: &str, error: &anyhow::Error) {
        if self.json {
            emit(serde_json::json!({
                "event": "error", "component": component,
                "message": format!("{error:#}"),
            }));
        } else {
            eprintln!("{component}: FAILED — {error:#}");
        }
    }
}

fn emit(value: serde_json::Value) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{value}");
    let _ = stdout.flush();
}