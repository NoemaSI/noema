use gpui_kit::base::{h_flex, v_flex, StyledExt};
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::*;
use jobctl::{Engine, EngineError, Submitter};

use jobctl::intent::IntentHandler;
use super::wizard::{self, step_for, WizardPhase, WizardStep};
use crate::element::button::*;
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
    pub(crate) error: Vec<EngineError>,
    pub(crate) drafts: Vec<SkillDraft>,
    pub(crate) jobctl: Submitter<IntentHandler>,
}

impl CreateSkillView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (submitter, mut results) = Engine::spawn(IntentHandler, 32, 1);
        let weak = cx.weak_entity();
        cx.spawn(async move |this: WeakEntity<CreateSkillView>, cx| {
            while let Some((_id, outcome)) = results.recv().await {
                let Ok(()) = this.update(cx, |view, cx| {
                    match outcome {
                        Ok(criteria) => {
                            view.acceptance = criteria;
                        }
                        Err(e) => {
                            view.error.push(e);
                        }
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
            acceptance: Vec::new(),
            error: Vec::new(),
            drafts: Vec::new(),
            jobctl: submitter,
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
