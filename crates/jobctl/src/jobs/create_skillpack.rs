
use std::future::Future;
use std::path::PathBuf;

use agentctl::{AgentClient, AgentEvent, AgentRequest};
use noema_config::NoemaConfig;
use serde::{Deserialize, Serialize};

use crate::jobs::ensure_vm::{provision, EnsureVmEvent};
use crate::{EngineError, EngineHandle, JobCtx, JobDefinition, JobId, Services};

use pack;

/// The one catalogue job the UI knows about: validate the goal, provision the
/// VM, run the goal on the in-guest agent, and stream everything back as a
/// single event stream.
pub struct CreateSkillpack;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSkillpackPayload {
    pub name: String,
    pub problem_description: String,
    pub baseline_hint: Option<String>,
    pub files_dropped: Vec<String>,
    pub config: NoemaConfig,
}

/// Everything the UI sees while the job runs: provisioning progress and the
/// agent's own events, discriminated by shape (untagged).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CreateSkillpackEvent {
    Provision(EnsureVmEvent),
    Agent(AgentEvent),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSkillpackOutput {
    pub criteria: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    pub safe_name: String,
}

impl JobDefinition for CreateSkillpack {
    const KIND: &'static str = "create_skillpack";
    const DESCRIPTION: &'static str = "Create a new skillpack with the help of the LLM";

    type Payload = CreateSkillpackPayload;
    type Event = CreateSkillpackEvent;
    type Output = CreateSkillpackOutput;

    fn run(
        ctx: JobCtx,
        payload: Self::Payload,
        services: Services,
    ) -> impl Future<Output = Result<Self::Output, EngineError>> + Send {


        async move {
            let safe_name = &payload.name.trim().to_ascii_lowercase().replace(' ', "_");
            if payload.files_dropped.is_empty() {
                return Err(EngineError::JobFailed(format!("please provide task specific data by dropping one one or multiple files into the file upload field.")));
            }

            match pack::SkillpackIdentifier::new(&*safe_name) {
                Ok(skill_identifier) => {
                    let _ = pack::copy_uploaded_files_to_pack_input_dir(&payload.files_dropped, &skill_identifier, &payload.config);
                    // Blocking call (creates its own tokio runtime internally);
                    // must not run on a jobctl worker thread.
                    let skill_pack = tokio::task::spawn_blocking(move || {
                        pack::new_skill_pack_llm_guided(skill_identifier, payload.problem_description, payload.baseline_hint, payload.config)
                    })
                    .await
                    .map_err(|e| EngineError::JobFailed(format!("skillpack task panicked: {e}")))?
                    .map_err(|e| EngineError::JobFailed(format!("{e:#}")))?;
                }
                Err(e) => {
                    return Err(EngineError::JobFailed(format!("unable to construct skillpack: {e}")))
                }
            }
            Ok(CreateSkillpackOutput {
                criteria: vec![],
                summary: None,
                safe_name: safe_name.clone(),
            })
        }
    }
}

impl EngineHandle {
    /// Typed façade for the UI: start a skillpack creation.
    /// The payload is serialized here so the wire format is always in sync
    /// with `CreateSkillpackPayload` (no hand-built JSON).
    pub fn create_skillpack(&self, payload: CreateSkillpackPayload) -> Result<JobId, EngineError> {
        self.try_submit(
            CreateSkillpack::KIND,
            serde_json::to_value(&payload)?,
        )
    }
}
