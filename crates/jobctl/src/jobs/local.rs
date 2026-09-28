//! Temporary local agent mode: the agentctl server runs *in-process*
//! instead of inside the VM (engine disk persistence is currently
//! unreliable). `NOEMA_VM=1` switches [`crate::jobs::intent::Intent`] back
//! to provisioning.

use tokio::sync::OnceCell;

use agentctl::handlers::GoalRun;
use agentctl::server::AgentServer;

use crate::jobs::ensure_vm::{EnsureVmEvent, VmInfo};
use crate::jobs::intent::IntentEvent;
use crate::{EngineError, JobCtx};

/// Dedicated local address; 3333 stays free for the VM port-forward.
const LOCAL_ADDR: &str = "127.0.0.1:3334";
const LOCAL_BASE: &str = "http://127.0.0.1:3334";

/// Bound once per process; errors are remembered so callers see the real
/// bind failure instead of retrying forever.
static SERVER: OnceCell<Result<(), String>> = OnceCell::const_new();

/// Ensure the in-process agent server is listening, then return its info.
pub async fn ensure_local_agent(ctx: &JobCtx) -> Result<VmInfo, EngineError> {
    emit(ctx, "in-process agentctl server".into()).await;

    let started = SERVER
        .get_or_try_init(|| async {
            let result =
                match AgentServer::builder().register::<GoalRun>().spawn_serving(LOCAL_ADDR).await
                {
                    Ok(_task) => Ok(()),
                    Err(e) => Err(format!("bind {LOCAL_ADDR}: {e}")),
                };
            Ok::<_, std::convert::Infallible>(result)
        })
        .await
        .expect("infallible")
        .clone();
    if let Err(e) = started {
        // Port already taken: adopt a healthy server (e.g. leftover from a
        // previous run) instead of failing.
        if !health_ok().await {
            return Err(EngineError::JobFailed(e));
        }
        emit(ctx, format!("adopting existing server on {LOCAL_ADDR}")).await;
    }

    // The listener is bound before `spawn_serving` returns; this just
    // absorbs the task-startup race.
    for _ in 0..20 {
        if health_ok().await {
            return Ok(VmInfo {
                name: "local".into(),
                http_base: LOCAL_BASE.into(),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err(EngineError::JobFailed(format!(
        "in-process agent server not answering on {LOCAL_ADDR}"
    )))
}

async fn health_ok() -> bool {
    reqwest::Client::new()
        .get(format!("{LOCAL_BASE}/healthz"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn emit(ctx: &JobCtx, line: String) {
    let _ = ctx
        .emit(&IntentEvent::Provision(EnsureVmEvent {
            step: "local".into(),
            line,
        }))
        .await;
}