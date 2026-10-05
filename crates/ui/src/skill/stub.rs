//! Stub of the LLM-guided skillpack analysis, shared by the UI integration
//! tests and the `stub-jobs` preview mode.
//!
//! It registers under [`CreateSkillpack::KIND`], so the production view,
//! engine handle, event loop and render path all run unchanged without
//! touching the LLM.

use std::future::Future;

use jobctl::jobs::create_skillpack::{
    CreateSkillpack, CreateSkillpackEvent, CreateSkillpackOutput, CreateSkillpackPayload, SkillPack,
    SkillPackDrive, SkillPackMapping, SkillPackOutput, SkillPackSplit,
};
use jobctl::{EngineError, EngineHandle, JobCtx, JobDefinition, Results, Services, engine};

/// Answers instantly with a fixed [SkillPack]; fails when the requested name
/// is `"boom"` (used by the failure-path test).
pub struct StubCreateSkillpack;

impl JobDefinition for StubCreateSkillpack {
    // make this a stub for the CreateSkillpack job by assigning its KIND
    const KIND: &'static str = CreateSkillpack::KIND;
    const DESCRIPTION: &'static str = "test stub";

    type Payload = CreateSkillpackPayload;
    type Event = CreateSkillpackEvent;
    type Output = CreateSkillpackOutput;

    fn run(
        _ctx: JobCtx,
        payload: Self::Payload,
        _services: Services,
    ) -> impl Future<Output = Result<Self::Output, EngineError>> + Send {
        async move {
            if payload.name == "boom" {
                return Err(EngineError::JobFailed("stub failure".to_string()));
            }
            let safe_name = payload.name.trim().to_ascii_lowercase().replace(' ', "_");
            Ok(CreateSkillpackOutput {
                criteria: vec!["criterion a".to_string(), "criterion b".to_string()],
                summary: Some("stub summary".to_string()),
                skill_pack: SkillPack {
                    data_path: "/data/tumor_t0.csv".into(),
                    data_delimiter: ",".to_string(),
                    name: safe_name.clone(),
                    mapping: SkillPackMapping {
                        subject: "line".to_string(),
                        curve: vec!["dose".to_string()],
                        time: "t".to_string(),
                        drives: vec![SkillPackDrive {
                            column: "dose".to_string(),
                            name: "dose".to_string(),
                            units: "mg/kg".to_string(),
                            description: "single administered dose".to_string(),
                            recorded: false,
                        }],
                        outputs: vec![SkillPackOutput {
                            column: "tumor".to_string(),
                            name: "tumor_volume".to_string(),
                            units: "mm^3".to_string(),
                            description: "measured tumour volume".to_string(),
                            recorded: true,
                        }],
                    },
                    condition_column: "dose".to_string(),
                    split: SkillPackSplit {
                        train: vec!["0".to_string(), "2".to_string()],
                        validation: vec!["4".to_string()],
                        hidden: vec!["16".to_string()],
                    },
                },
                safe_name,
            })
        }
    }
}

/// Spawn an engine whose only job is the stub, registered under the real
/// `create_skillpack` kind.
pub fn spawn_stub_engine() -> (EngineHandle, Results) {
    engine()
        .register::<StubCreateSkillpack>()
        .queue_capacity(8)
        .worker_threads(1)
        .spawn()
}
