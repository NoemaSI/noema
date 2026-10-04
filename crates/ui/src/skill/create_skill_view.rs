use gpui_kit::base::{h_flex, v_flex, StyledExt};
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::*;
use jobctl::jobs::create_skillpack::{CreateSkillpack,CreateSkillpackOutput, SkillPack};
use jobctl::protocol::AgentEvent;
use jobctl::{EngineHandle, EventStatus, JobDefinition, JobId, engine};

use super::wizard::{self, step_for, WizardPhase, WizardStep};
use crate::element::button::*;
use crate::element::file_drop::FileDropView;
use crate::{FONT_FAMILY, TEXT_SM};

pub struct SkillDraft {
    pub name: String,
    pub created: String,
    pub status: &'static str,
}

pub struct CreateSkillView {
    active_tab: usize,
    //  pub(crate)  means public within the current crate, but not from other crate
    pub(crate) phase: WizardPhase,
    pub(crate) name_input: Entity<InputState>,
    pub(crate) goal_input: Entity<TextareaState>,
    pub(crate) acceptance: Vec<String>,
    pub(crate) skill_pack: Option<SkillPack>,
    pub(crate) error: Vec<String>,
    pub(crate) agent_log: Vec<String>,
    pub(crate) drafts: Vec<SkillDraft>,
    pub(crate) jobctl: EngineHandle,
    pub(crate) filedrop: Entity<FileDropView>,
    /// Root job id of the in-flight intent analysis, if any.
    pub(crate) pending_analysis_root: Option<JobId>,
}

impl CreateSkillView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (jobctl, mut results) = engine()
            .register::<CreateSkillpack>()
            .queue_capacity(32)
            .worker_threads(1)
            .spawn();

        cx.spawn(async move |this: WeakEntity<CreateSkillView>, cx| {
            while let Some(ev) = results.recv().await {
                let Ok(()) = this.update(cx, |view, cx| {

                    println!("ev: {:#?}", &ev);
                    match (ev.status, ev.kind.as_str()) {
                        (EventStatus::Progress, CreateSkillpack::KIND) => {
                            // NOTE: we removed the agent in  jobctl.create_skillpack
                            // these events will never fire
                            
                            // Agent events only; provisioning progress
                            // (VM plumbing) is intentionally not surfaced.
                            if let Ok(agent_event) =
                                serde_json::from_value::<AgentEvent>(ev.payload)
                            {
                                match agent_event {
                                    AgentEvent::Criteria { items } => {
                                        view.acceptance = items.clone();
                                        view.push_log(format!("\u{2691} {items:?}"));
                                    }
                                    AgentEvent::Done { summary } => {
                                        view.push_log(format!("\u{2713} {summary}"));
                                    }
                                    AgentEvent::Failed { error } => {
                                        view.push_log(format!("\u{2715} {error}"));
                                    }
                                }
                            }
                        }
                        (EventStatus::Done, CreateSkillpack::KIND) => {
                            match serde_json::from_value::<CreateSkillpackOutput>(ev.payload) {
                                Ok(out) => {
                                    view.acceptance = out.criteria;
                                    if let Some(summary) = out.summary {
                                        view.push_log(format!("agent: {summary}"));
                                    }
                                    view.push_log(format!(
                                        "\u{2713} analysis complete \u{2014} skillpack \u{201c}{}\u{201d} received. Review & confirm.",
                                        out.safe_name
                                    ));
                                    view.skill_pack = Some(out.skill_pack);
                                }
                                Err(e) => {
                                    view.push_log(format!("\u{2715} could not parse analysis output: {e}"));
                                }
                            }
                            if view.pending_analysis_root == Some(ev.root) {
                                view.pending_analysis_root = None;
                                view.phase = WizardPhase::IntentAnalysisDone;
                            }
                        }
                        (EventStatus::Failed, kind) => {
                            let msg = ev
                                .payload
                                .get("error")
                                .and_then(|e| e.as_str())
                                .unwrap_or("unknown error")
                                .to_string();
                            view.error.push(format!("{kind}: {msg}"));
                            view.push_log(format!("\u{2715} {kind}: {msg}"));
                            if view.pending_analysis_root == Some(ev.root) {
                                view.pending_analysis_root = None;
                            }
                        }
                        _ => {}
                    }
                    cx.notify();
                }) else { break;};
            }
        })
        .detach();
        Self {
            active_tab: 0,
            phase: WizardPhase::Initial,
            name_input: cx.new(|cx| InputState::new(window, cx).placeholder("unnamed")),
            goal_input: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("what should this skill do, in plain words?")
                    .rows(3)
            }),
            filedrop: cx.new(|cx| FileDropView::new()),
            acceptance: Vec::new(),
            error: Vec::new(),
            agent_log: Vec::new(),
            drafts: Vec::new(),
            jobctl,
            pending_analysis_root: None,
            skill_pack: None,
        }
    }

    /// Append a line to the agent log, keeping the most recent 300.
    pub(crate) fn push_log(&mut self, line: String) {
        self.agent_log.push(line);
        if self.agent_log.len() > 300 {
            self.agent_log.remove(0);
        }
    }

    pub(crate) fn step(&self) -> &'static dyn WizardStep {
        step_for(self.phase)
    }

    /// Click handler that transitions the wizard to `phase`.
    pub(crate) fn goto(
        &self,
        phase: WizardPhase,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let entity = cx.entity().clone();
        move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.phase = phase;
                cx.notify();
            })
        }
    }

    fn render_tab_content(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        match self.active_tab {
            0 => self.render_my_skills(cx).into_any_element(),
            1 => self.render_skill_drafts(cx).into_any_element(),
            _ => div().child("Unknown content").into_any_element(),
        }
    }

    fn render_skill_drafts(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let empty = self.drafts.is_empty();
        div()
            .size_full()
            .p_4()
            .bg(theme.background)
            .child(
                v_flex()
                    .w_full()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(4.))
                    .bg(theme.secondary)
                    .font_family(FONT_FAMILY)
                    .text_size(TEXT_SM)
                    .child(
                        h_flex()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .bg(theme.overlay)
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(div().flex_1().child("DRAFT NAME"))
                            .child(div().w(px(140.)).child("CREATED"))
                            .child(div().w(px(120.)).child("STATUS")),
                    )
                    .when(empty, |table| {
                        table.child(
                            v_flex()
                                .items_center()
                                .justify_center()
                                .gap_y_1()
                                .py_12()
                                .text_color(theme.muted_foreground)
                                .child("No drafts yet")
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .child("Save a draft from the wizard and it will appear here."),
                                ),
                        )
                    })
                    .children(self.drafts.iter().map(|draft| {
                        h_flex()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(div().flex_1().child(draft.name.clone()))
                            .child(
                                div()
                                    .w(px(140.))
                                    .text_color(theme.muted_foreground)
                                    .child(draft.created.clone()),
                            )
                            .child(div().w(px(120.)).child(draft.status))
                    })),
            )
    }

    fn render_my_skills(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        h_flex()
            .size_full()
            .items_stretch()
            .gap_4()
            .p_4()
            .bg(theme.background)
            .child(self.render_skill_panel(cx))
            .child(self.render_queue_panel(cx))
    }

    fn render_skill_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let step = self.step();
        let title = step.title(self, cx);
        let nodes = step.nodes();
        v_flex()
            .flex_1()
            .gap_y_4()
            .p_4()
            .bg(theme.secondary)
            .border_1()
            .border_color(theme.border)
            .rounded(px(4.))
            .font_family(FONT_FAMILY)
            .child(
                h_flex()
                    .gap_x_2()
                    .items_center()
                    .mb_2()
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title)
                    .when_some(step.badge(), |row, badge| {
                        row.child(wizard::badge_pill(badge, &theme))
                    }),
            )
            .child(
                h_flex()
                    .gap_x_4()
                    .items_stretch()
                    .child(wizard::role_circle(&theme, &nodes))
                    .child(wizard::state_box(step, self, &theme))
                    .child(wizard::training_path(&theme, step.active_path_step())),
            )
            .child(step.main_area(self, &theme))
            .child(step.actions(self, cx))
            .child(
                div()
                    .text_size(TEXT_SM)
                    .text_color(theme.muted_foreground)
                    .child("Certified skills: (none)"),
            )
    }


    fn render_queue_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .w(px(300.))
            .flex_none()
            .bg(theme.secondary)
            .border_1()
            .border_color(theme.border)
            .rounded(px(4.))
            .font_family(FONT_FAMILY)
            .child(
                h_flex()
                    .justify_between()
                    .p_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(TEXT_SM)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("TRAINING QUEUE"),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child("0/150"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_size(TEXT_SM)
                    .text_color(theme.muted_foreground)
                    .child("Steps appear here")
                    .child("once you track a plan"),
            )
            .child(
                v_flex()
                    .gap_y_2()
                    .p_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .justify_between()
                            .text_size(TEXT_SM)
                            .child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .child("TOTAL"),
                            )
                            .child("\u{2014}"),
                    )
                    .child(
                        h_flex()
                            .gap_x_1()
                            .child(sbutton_auto("queue-pause", "\u{2016}", |_, _, _| {}, cx))
                            .child(sbutton_auto("queue-step", "\u{226b}", |_, _, _| {}, cx)),
                    )
                    .child(
                        div()
                            .w_full()
                            .h_1()
                            .bg(theme.muted)
                            .rounded_full(),
                    ),
            )
    }
}

impl Render for CreateSkillView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .font_family(FONT_FAMILY)
            .bg(theme.background)
            .child(
                TabBar::new("content-tabs")
                    .selected_index(self.active_tab)
                    .on_click(cx.listener(|view, index, _, cx| {
                        view.active_tab = *index;
                        cx.notify();
                    }))
                    .child(Tab::new().label("MY SKILLS"))
                    .child(Tab::new().label("SKILL DRAFTS")),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .v_flex()
                    .child(self.render_tab_content(cx)),
            )
    }
}
