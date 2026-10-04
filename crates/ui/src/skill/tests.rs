//! UI integration tests for the create-skill wizard.
//!
//! The LLM path is stubbed at the jobctl layer: a stub job registered under
//! [`CreateSkillpack::KIND`] answers the real submit, so the production view,
//! engine handle, event loop and render path all run unchanged.

use std::future::Future;
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, EntityId, TestAppContext, WindowBounds, WindowOptions, px,
    size,
};
use jobctl::jobs::create_skillpack::{
    CreateSkillpack, CreateSkillpackEvent, CreateSkillpackOutput, CreateSkillpackPayload,
    SkillPack, SkillPackDrive, SkillPackMapping, SkillPackOutput, SkillPackSplit,
};
use jobctl::{EngineError, EngineHandle, JobCtx, JobDefinition, Results, Services, engine};
use noema_config::{LlmConfig, NoemaConfig};

use super::create_skill_view::{CreateSkillView, QueueStatus};
use super::wizard::WizardPhase;

/// Stub of the LLM-guided skillpack analysis: answers instantly with a fixed
/// [SkillPack]; fails when the requested name is `"boom"`.
struct StubCreateSkillpack;

impl JobDefinition for StubCreateSkillpack {
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

/// The Textarea's inner Input is keyed by its state entity: `("input", id)`.
fn goal_input_id(view: &Entity<CreateSkillView>, cx: &mut TestAppContext) -> EntityId {
    cx.update(|cx| view.read(cx).goal_input.entity_id())
}

/// Point `$HOME` at a temp dir containing a valid `~/.noema/config.json` so
/// the submit handler's `NoemaConfig::from_home()` succeeds hermetically.
fn ensure_test_home() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join("noema-ui-test-home");
        let noema = dir.join(".noema");
        std::fs::create_dir_all(&noema).expect("create test home");
        let config = NoemaConfig {
            llm: LlmConfig {
                model_name: "stub-model".to_string(),
                api_key: None,
                endpoint: "http://127.0.0.1:1".to_string(),
            },
            workdir: dir.clone(),
        };
        std::fs::write(
            noema.join("config.json"),
            serde_json::to_string(&config).unwrap(),
        )
        .expect("write test config");
        // SAFETY: tests using this fixture all agree on the same value and
        // only ever read it back through `NoemaConfig::from_home()`.
        unsafe { std::env::set_var("HOME", &dir) };
    });
}

fn spawn_stub_engine() -> (EngineHandle, Results) {
    engine()
        .register::<StubCreateSkillpack>()
        .queue_capacity(8)
        .worker_threads(1)
        .spawn()
}

fn open_wizard(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<CreateSkillView>) {
    let (jobctl, results) = spawn_stub_engine();
    let slot = std::cell::Cell::new(None);
    let (handle, _root) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1200.), px(900.)), cx)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| {
                    CreateSkillView::with_engine(window, cx, jobctl, results)
                });
                slot.set(Some(view.clone()));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open test window")
    });
    (handle, slot.take().unwrap())
}

#[gpui_kit::test]
async fn analyze_intent_runs_through_the_engine_and_completes_the_step(
    cx: &mut TestAppContext,
) {
    ensure_test_home();
    cx.update(gpui_kit::init);
    let (window, view) = open_wizard(cx);

    // Initial phase: the analysis form is not on screen yet.
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("new-skill").is_some());
        assert!(window.try_find("analyze-intent").is_none());
    })
    .unwrap();

    // Enter the wizard and fill the form.
    let goal = goal_input_id(&view, cx);
    cx.update_window(window, |_, window, cx| {
        window.click("new-skill", cx);
        assert!(window.try_find("analyze-intent").is_some());
        window.click("skill-name", cx);
        window.input("foo barr", cx);
        window.click(("input", goal), cx);
        window.input("tumour volume response", cx);
        assert_eq!(window.find("skill-name").value(), Some("foo barr"));
    })
    .unwrap();

    // Submit: the job is queued, the queue panel tracks it as running.
    cx.update_window(window, |_, window, cx| {
        window.click("analyze-intent", cx);
    })
    .unwrap();
    cx.update(|cx| {
        view.update(cx, |view, _| {
            assert_eq!(view.phase, WizardPhase::IntentAnalysisWaiting);
            assert_eq!(view.queue.len(), 1);
            assert_eq!(view.queue[0].status, QueueStatus::Running);
            assert!(view.skill_pack.is_none());
        })
    });

    // The stub engine's Done event drives the view to the finished state.
    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        view.read(cx).skill_pack.is_some()
    })
    .await;

    cx.update(|cx| {
        view.update(cx, |view, _| {
            assert_eq!(view.phase, WizardPhase::IntentAnalysisDone);
            assert_eq!(view.queue[0].status, QueueStatus::Done);
            assert_eq!(view.acceptance, vec!["criterion a", "criterion b"]);
            let pack = view.skill_pack.as_ref().expect("skill pack was set");
            assert_eq!(pack.name, "foo_barr");
            assert_eq!(pack.condition_column, "dose");
            assert_eq!(pack.split.hidden, vec!["16".to_string()]);
        })
    });

    // The finished step renders (analysis box + confirm action present).
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("confirm-attach").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn failed_analysis_marks_the_queue_entry_and_keeps_the_step(
    cx: &mut TestAppContext,
) {
    ensure_test_home();
    cx.update(gpui_kit::init);
    let (window, view) = open_wizard(cx);

    let goal = goal_input_id(&view, cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("new-skill", cx);
        window.click("skill-name", cx);
        window.input("boom", cx);
        window.click(("input", goal), cx);
        window.input("anything", cx);
        window.click("analyze-intent", cx);
    })
    .unwrap();

    cx.wait_for(window, Duration::from_secs(5), |_, cx| {
        view.read(cx).queue.first().is_some_and(|e| e.status == QueueStatus::Failed)
    })
    .await;

    cx.update(|cx| {
        view.update(cx, |view, _| {
            assert_eq!(view.phase, WizardPhase::IntentAnalysisWaiting);
            assert_eq!(view.queue[0].status, QueueStatus::Failed);
            assert!(view.skill_pack.is_none());
            assert!(view.pending_analysis_root.is_none());
            assert!(
                view.error.iter().any(|e| e.contains("stub failure")),
                "expected stub failure in {:#?}",
                view.error
            );
        })
    });
}

#[gpui_kit::test]
fn fresh_wizard_has_empty_queue_and_no_analysis(cx: &mut TestAppContext) {
    ensure_test_home();
    cx.update(gpui_kit::init);
    let (window, view) = open_wizard(cx);

    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();

    cx.update(|cx| {
        view.update(cx, |view, _| {
            assert_eq!(view.phase, WizardPhase::Initial);
            assert!(view.queue.is_empty());
            assert!(view.skill_pack.is_none());
        })
    });
}

