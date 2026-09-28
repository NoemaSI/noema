use std::future::Future;

use agentctl::{AgentClient, AgentEvent, AgentRequest};
use serde::{Deserialize, Serialize};

use crate::jobs::ensure_vm::{provision, EnsureVmEvent};
use crate::{EngineError, EngineHandle, JobCtx, JobDefinition, JobId, Services};

/// The one catalogue job the UI knows about: validate the goal, provision the
/// VM, run the goal on the in-guest agent, and stream everything back as a
/// single event stream.
pub struct Intent;

#[derive(Debug, Deserialize)]
pub struct IntentPayload {
    pub name: String,
    pub goal: String,
}

/// Everything the UI sees while the job runs: provisioning progress and the
/// agent's own events, discriminated by shape (untagged).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IntentEvent {
    Provision(EnsureVmEvent),
    Agent(AgentEvent),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IntentOutput {
    pub criteria: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

impl JobDefinition for Intent {
    const KIND: &'static str = "intent";
    const DESCRIPTION: &'static str = "Analyse skill name & goal, propose acceptance criteria";

    type Payload = IntentPayload;
    type Event = IntentEvent;
    type Output = IntentOutput;

    fn run(
        ctx: JobCtx,
        payload: Self::Payload,
        services: Services,
    ) -> impl Future<Output = Result<Self::Output, EngineError>> + Send {
        async move {
            if payload.goal.trim().is_empty() {
                return Err(EngineError::JobFailed(
                    "goal is empty; nothing to analyse".into(),
                ));
            }

            // Bring up the VM + agent server, streaming provisioning events.
            let vm = provision(ctx.clone(), services.clone()).await?;

            // Run the goal on the guest agent, streaming its events.
            let client = AgentClient::new(vm.http_base);
            let mut run = client
                .run_goal(AgentRequest {
                    goal: format!("{}: {}", payload.name, payload.goal),
                    criteria: vec![],
                })
                .await
                .map_err(|e| EngineError::JobFailed(format!("agent request failed: {e}")))?;

            while let Some(event) = run
                .recv()
                .await
                .map_err(|e| EngineError::JobFailed(format!("agent stream error: {e}")))?
            {
                ctx.emit(&IntentEvent::Agent(event)).await?;
            }

            if let Some(error) = run.error() {
                return Err(EngineError::JobFailed(error.to_string()));
            }

            Ok(IntentOutput {
                criteria: run.criteria().to_vec(),
                summary: Some(
                    run.summary()
                        .unwrap_or("agent stream ended without Done")
                        .to_string(),
                ),
            })
        }
    }
}

impl EngineHandle {
    /// Typed façade for the UI: start an intent analysis for `name` & `goal`.
    pub fn analyze_intent(&self, name: String, goal: String) -> Result<JobId, EngineError> {
        self.try_submit(
            Intent::KIND,
            serde_json::json!({ "name": name, "goal": goal }),
        )
    }
}
