use std::future::Future;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vm::{ExecEvent, ExecOptions, Port, Vm};

use crate::{EngineError, JobCtx, Services};

/// Guest port the agentctl HTTP server listens on.
pub const AGENT_GUEST_PORT: u16 = 3333;
/// Host port forwarded to [`AGENT_GUEST_PORT`].
pub const AGENT_HOST_PORT: u16 = 3333;

const AGENT_DIR: &str = "/opt/noema/agentctl";
const SHA_MARKER: &str = "/opt/noema/agentctl.sha";
/// Portable "kill all agentctl processes" one-liner (no procps in slim).
const KILL_AGENTCTL: &str = "for d in /proc/[0-9]*; do [ -r \"$d/comm\" ] && \
                             [ \"$(cat \"$d/comm\")\" = agentctl ] && kill \"${d#/proc/}\" \
                             2>/dev/null; done; true";
const AGENT_BIN: &str = "/opt/noema/agentctl/target/release/agentctl";

/// Result of [`EnsureVm`]: where to reach the agent HTTP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmInfo {
    pub name: String,
    pub http_base: String,
}

fn default_name() -> String {
    "noema".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsureVmEvent {
    pub step: String,
    pub line: String,
}

/// Attach-or-create the persistent Debian machine, make sure it has a Rust
/// toolchain, build `agentctl` in-guest when its source changed, and leave
/// the HTTP server healthy on port 3333.
///
/// Not a catalogue job: called by [`crate::jobs::intent::Intent`], which
/// re-emits its progress on the single UI-facing event stream.
pub async fn provision(ctx: JobCtx, services: Services) -> Result<VmInfo, EngineError> {
    async move {
            let name = default_name();
            emit(&ctx, "vm", format!("attaching or creating machine '{name}'")).await?;
            let vm = spawn_vm(name.clone()).await?;

            let reported = {
                let vm = vm.clone();
                tokio::task::spawn_blocking(move || {
                    vm.host_port(AGENT_GUEST_PORT)
                        .map_err(|e| EngineError::JobFailed(e.to_string()))
                })
                .await
                .map_err(|_| EngineError::ShuttingDown)??
            };
            // `Machine::connect` handles do not report published ports (see vmctl), so
// an attached VM reports None even though the forward is live. Fall back to
// the fixed host-port convention and let the health check verify it.
let host_port = reported.unwrap_or(AGENT_HOST_PORT);

            // --- ensure_rust -------------------------------------------------
            let have_cargo = exec_streaming(
                &ctx,
                "rust",
                vm.clone(),
                sh("cargo --version"),
            )
            .await?;
            if have_cargo != 0 {
                emit(&ctx, "rust", "installing build tools".into()).await?;
                exec_streaming(
                    &ctx,
                    "rust",
                    vm.clone(),
                    sh("apt-get update -qq && apt-get install -y -qq --no-install-recommends \
                        build-essential curl ca-certificates"),
                )
                .await?;
                emit(&ctx, "rust", "installing rustup (minimal profile)".into()).await?;
                exec_streaming(
                    &ctx,
                    "rust",
                    vm.clone(),
                    sh("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
                        | sh -s -- -y --profile minimal"),
                )
                .await?;
            }

            // --- ensure_agentctl ----------------------------------------------
            let sources = tokio::task::spawn_blocking(agentctl_source)
                .await
                .map_err(|_| EngineError::ShuttingDown)??;
            let digest = digest_sources(&sources);
            let marker = {
                let vm = vm.clone();
                tokio::task::spawn_blocking(move || vm.read_file(SHA_MARKER).ok())
                    .await
                    .map_err(|_| EngineError::ShuttingDown)?
            };
            if marker.as_deref() != Some(digest.as_bytes()) {
                emit(
                    &ctx,
                    "agentctl",
                    format!("source changed; uploading and building ({})", digest),
                )
                .await?;
                let dirs = unique_parents(&sources);
                exec_streaming(&ctx, "agentctl", vm.clone(), sh(&format!("mkdir -p {AGENT_DIR}")))
                    .await?;
                for dir in &dirs {
                    exec_streaming(
                        &ctx,
                        "agentctl",
                        vm.clone(),
                        sh(&format!("mkdir -p {AGENT_DIR}/{dir}")),
                    )
                    .await?;
                }
                for (rel, bytes) in &sources {
                    let vm = vm.clone();
                    let path = format!("{AGENT_DIR}/{rel}");
                    let bytes = bytes.clone();
                    tokio::task::spawn_blocking(move || {
                        vm.write_file(&path, bytes)
                            .map_err(|e| EngineError::JobFailed(format!("{path}: {e}")))
                    })
                    .await
                    .map_err(|_| EngineError::ShuttingDown)??;
                }
                // Marker first: the guest build.rs embeds it into the binary,
                // so /healthz reports exactly the sources this binary was
                // built from. Removed again if the build fails.
                let marker_vm = vm.clone();
                let marker_value = digest.clone();
                tokio::task::spawn_blocking(move || {
                    marker_vm.write_file(SHA_MARKER, marker_value)
                        .map_err(|e| EngineError::JobFailed(e.to_string()))
                })
                .await
                .map_err(|_| EngineError::ShuttingDown)??;
                let code = exec_streaming(
                    &ctx,
                    "agentctl",
                    vm.clone(),
                    sh(&format!("cd {AGENT_DIR} && cargo build --release")),
                )
                .await?;
                if code != 0 {
                    let vm = vm.clone();
                    tokio::task::spawn_blocking(move || {
                        vm.exec(sh(&format!("rm -f {SHA_MARKER}")))
                    })
                    .await
                    .ok();
                    return Err(EngineError::JobFailed(format!(
                        "agentctl build failed in guest (exit {code})"
                    )));
                }
            } else {
                emit(&ctx, "agentctl", "guest build up to date".into()).await?;
            }

            // --- start + health ------------------------------------------------
            // Health check includes the source digest: a server built from
            // older sources counts as unhealthy and gets replaced.
            let http_base = format!("http://127.0.0.1:{host_port}");
            let client = services
                .get::<reqwest::Client>()
                .map(|c| (*c).clone())
                .unwrap_or_default();
            if !health(&client, &http_base, &digest).await {
                emit(&ctx, "agentctl", "starting agent server".into()).await?;
                exec_streaming(
                    &ctx,
                    "agentctl",
                    vm.clone(),
                    sh(&format!(
                        "{KILL_AGENTCTL}; nohup {AGENT_BIN} serve >/var/log/agentctl.log 2>&1 & \
                         echo launched"
                    )),
                )
                .await?;
                let mut healthy = false;
                for _ in 0..40 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if health(&client, &http_base, &digest).await {
                        healthy = true;
                        break;
                    }
                }
                if !healthy {
                    return Err(EngineError::JobFailed(format!(
                        "agentctl did not become healthy at {http_base}; check \
                         /var/log/agentctl.log, and if the machine predates port \
                         {AGENT_GUEST_PORT} recreate it once with `vmctl delete {name} && \
                         vmctl up {name}`"
                    )));
                }
            }
            emit(&ctx, "agentctl", format!("healthy at {http_base}")).await?;

            let info = VmInfo {
                name: name.clone(),
                http_base,
            };
        Ok(info)
    }
    .await
}

fn emit(ctx: &JobCtx, step: &str, line: String) -> impl Future<Output = Result<(), EngineError>> {
    let ctx = ctx.clone();
    let step = step.to_string();
    async move { ctx.emit(&EnsureVmEvent { step, line }).await }
}

/// Wrap a shell command so `cargo` from rustup is on PATH.
fn sh(command: &str) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("export PATH=\"$HOME/.cargo/bin:$PATH\"; {command}"),
    ]
}

async fn spawn_vm(name: String) -> Result<Vm, EngineError> {
    tokio::task::spawn_blocking(move || -> Result<Vm, EngineError> {
        let failed = |e: vm::Error| EngineError::JobFailed(e.to_string());
        if let Ok(vm) = Vm::attach(&name) {
            if !vm.is_running() {
                vm.start().map_err(failed)?;
            }
            vm.wait_until_ready().map_err(failed)?;
            return Ok(vm);
        }
        let builder = Vm::builder(name.clone())
            .image("debian:stable-slim")
            .cpus(1)
            .memory_mib(2048)
            .network(true)
            .overlay_gib(1)
            .storage_gib(1)
            .persistent(true)
            .port(Port::new(AGENT_HOST_PORT, AGENT_GUEST_PORT));
        Vm::spawn(builder).map_err(|e| {
            EngineError::JobFailed(format!(
                "machine {name} exists but could not be started or recreated: {e}; recreate it \
                 with `vmctl delete {name} && vmctl up {name}`"
            ))
        })
    })
    .await
    .map_err(|_| EngineError::ShuttingDown)?
}

/// Run a guest command on the blocking pool, streaming its output as
/// [`EnsureVmEvent`]s; returns the exit code.
async fn exec_streaming(
    ctx: &JobCtx,
    step: &str,
    vm: Vm,
    command: Vec<String>,
) -> Result<i32, EngineError> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let step = step.to_string();
    let runner = tokio::task::spawn_blocking(move || -> Result<i32, EngineError> {
        let failed = |e: vm::Error| EngineError::JobFailed(e.to_string());
        let mut exit = -1;
        for event in vm.exec_stream(command, ExecOptions::new()).map_err(failed)? {
            match event {
                ExecEvent::Stdout(bytes) => {
                    let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
                }
                ExecEvent::Stderr(bytes) => {
                    let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
                }
                ExecEvent::Exit(code) => exit = code,
                ExecEvent::Error(message) => return Err(EngineError::JobFailed(message)),
            }
        }
        Ok(exit)
    });
    while let Some(line) = rx.recv().await {
        emit(ctx, &step, line).await?;
    }
    runner.await.map_err(|_| EngineError::ShuttingDown)?
}

async fn health(client: &reqwest::Client, base: &str, digest: &str) -> bool {
    match client.get(format!("{base}/healthz")).send().await {
        Ok(r) if r.status().is_success() => r
            .text()
            .await
            .map(|body| body.trim() == format!("ok {digest}"))
            .unwrap_or(false),
        _ => false,
    }
}

/// All files that make up the agentctl package, as (relative path, bytes),
/// sorted; read from the sibling `crates/agentctl` directory.
fn agentctl_source() -> Result<Vec<(String, Vec<u8>)>, EngineError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../agentctl")
        .canonicalize()
        .map_err(|_| {
            EngineError::JobFailed("crates/agentctl not found next to jobctl".into())
        })?;
    let mut files = Vec::new();
    collect_file(&root, "Cargo.toml", &mut files);
    collect_file(&root, "build.rs", &mut files);
    collect_dir(&root.join("src"), "src", &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if files.is_empty() {
        return Err(EngineError::JobFailed(
            "crates/agentctl has no buildable sources".into(),
        ));
    }
    Ok(files)
}

fn collect_file(root: &Path, rel: &str, out: &mut Vec<(String, Vec<u8>)>) {
    if let Ok(bytes) = std::fs::read(root.join(rel)) {
        out.push((rel.to_string(), bytes));
    }
}

fn collect_dir(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let rel = format!("{prefix}/{name}");
        if path.is_dir() {
            collect_dir(&path, &rel, out);
        } else {
            if let Ok(bytes) = std::fs::read(&path) {
                out.push((rel, bytes));
            }
        }
    }
}

fn digest_sources(sources: &[(String, Vec<u8>)]) -> String {
    let mut hasher = Sha256::new();
    for (rel, bytes) in sources {
        hasher.update(rel.as_bytes());
        hasher.update(bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn unique_parents(sources: &[(String, Vec<u8>)]) -> Vec<String> {
    let mut dirs: Vec<String> = sources
        .iter()
        .filter_map(|(rel, _)| Path::new(rel).parent().map(|p| p.display().to_string()))
        .filter(|p| !p.is_empty() && p != ".")
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}