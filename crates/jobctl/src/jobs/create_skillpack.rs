
use std::future::Future;
use std::path::PathBuf;

use agentctl::{AgentClient, AgentEvent, AgentRequest};
use noema_config::NoemaConfig;
use pack::anyhow::{self, bail};
use pack::author::{AuthorSession, load_subjects, preview_panels};
use pack::plot::DataCurve;
use serde::{Deserialize, Serialize};

use crate::jobs::ensure_vm::{provision, EnsureVmEvent};
use crate::{EngineError, EngineHandle, JobCtx, JobDefinition, JobId, Services};

use pack;

/// The one catalogue job the UI knows about: validate the goal, provision the
/// VM, run the goal on the in-guest agent, and stream everything back as a
/// single event stream.
pub struct CreateSkillpack;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPackDrive {
    pub column: String,
    pub name: String,
    pub units: String,
    pub description: String,
    pub recorded: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPackOutput {
    pub column: String,
    pub name: String,
    pub units: String,
    pub description: String,
    pub recorded: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPackMapping {
    pub subject: String,
    pub curve: Vec<String>,
    pub time: String,
    pub drives: Vec<SkillPackDrive>,
    pub outputs: Vec<SkillPackOutput>
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPackSplit {
    pub train: Vec<String>,
    pub validation: Vec<String>,
    pub hidden: Vec<String>
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPack {
    pub data_path: PathBuf,
    pub data_delimiter: String,
    pub name: String,
    pub mapping: SkillPackMapping,
    pub condition_column: String,
    pub split: SkillPackSplit,
}

fn channel_to_drive(channel: pack::author::ChannelColumn) -> Option<SkillPackDrive> {
    Some(SkillPackDrive {
        column: channel.column?,
        name: channel.name,
        units: channel.units,
        description: channel.description,
        recorded: channel.recorded,
    })
}

fn channel_to_output(channel: pack::author::ChannelColumn) -> Option<SkillPackOutput> {
    Some(SkillPackOutput {
        column: channel.column?,
        name: channel.name,
        units: channel.units,
        description: channel.description,
        recorded: channel.recorded,
    })
}

/// converts a AuthorSession (owned by JobCtl) to a SkillPack (owned by UI)
impl TryFrom<AuthorSession> for SkillPack {
    type Error = String;

    fn try_from(session: AuthorSession) -> Result<Self, Self::Error> {
        let gaps = pack::author::mapping_gaps(&session);
        if !gaps.is_empty() {
            return Err(format!(
                "session mapping is incomplete, missing: {}",
                gaps.join(", ")
            ));
        }

        let subject = session.mapping.subject.ok_or("missing subject column")?;
        let time = session.mapping.time.ok_or("missing time column")?;
        let condition_column = session
            .mapping
            .condition_column
            .or_else(|| session.mapping.drives.first().and_then(|d| d.column.clone()))
            .ok_or("missing condition column and no drive channel to derive it from")?;

        let drives = session
            .mapping
            .drives
            .into_iter()
            .map(channel_to_drive)
            .collect::<Option<Vec<_>>>()
            .ok_or("missing column for a drive channel")?;

        let outputs = session
            .mapping
            .outputs
            .into_iter()
            .map(channel_to_output)
            .collect::<Option<Vec<_>>>()
            .ok_or("missing column for an output channel")?;

        Ok(SkillPack {
            data_path: PathBuf::from(session.data.path),
            data_delimiter: session.data.delimiter.to_string(),
            name: session.name,
            mapping: SkillPackMapping {
                subject,
                curve: session.mapping.curve,
                time,
                drives,
                outputs,
            },
            condition_column,
            split: SkillPackSplit {
                train: session.split.train,
                validation: session.split.validation,
                hidden: session.split.hidden,
            },
        })
    }
}

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

#[derive(Clone, Serialize, Deserialize)]
pub struct SkillPanel {
    pub name: String,
    pub curves: Vec<DataCurve>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateSkillpackOutput {
    pub criteria: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    pub safe_name: String,
    pub skill_pack: SkillPack,
    pub skill_panels: Vec<SkillPanel>
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
                    // Blocking call (creates its own tokio runtime internally);
                    // must not run on a jobctl worker thread.
                    let session = tokio::task::spawn_blocking(move || {
                        pack::new_skill_pack_llm_guided(
                            skill_identifier, 
                            payload.problem_description,
                            payload.baseline_hint,
                            &payload.files_dropped,
                            payload.config
                        )
                    })
                    .await
                    .map_err(|e| EngineError::JobFailed(format!("skillpack task panicked: {e}")))?
                    .map_err(|e| EngineError::JobFailed(format!("{e:#}")))?;


                    let subjects = load_subjects(&session)
                                    .map_err(|err| EngineError::JobFailed(err.to_string()))?;


                    // <Vec<(String, Vec<DataCurve>)>
                    let mut skill_panels = vec![];
                    let panels = preview_panels(&session, &subjects, 2)
                        .map_err(|err| EngineError::JobFailed(err.to_string()))?;

                    for panel in panels {
                        let name = panel.0;
                        let curves = panel.1;
                        skill_panels.push(SkillPanel{name, curves});
                    }
                    let skill_pack = SkillPack::try_from(session)
                        .map_err(|e| EngineError::JobFailed(format!("{e}")))?;

                    return Ok(
                        CreateSkillpackOutput{
                            criteria: vec![],
                            summary: None,
                            safe_name: safe_name.clone(),
                            skill_pack,
                            skill_panels,
                        }
                    )

                }
                Err(e) => {
                    return Err(EngineError::JobFailed(format!("unable to construct skillpack: {e}")))
                }
            }
            unreachable!("both match arms return")
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
