use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Theme;
use gpui_kit::*;

use super::create_skill_view::CreateSkillView;
use super::wizard::{define_form_fields, RoleState, WizardPhase, WizardStep};
use crate::element::button::*;
use crate::skill::wizard::INITIAL_ROLE_DRAFT;

/// "New Skill" clicked: the user states goal & acceptance criteria.
pub struct DefineSkillStep;

impl WizardStep for DefineSkillStep {
    fn phase(&self) -> WizardPhase {
        WizardPhase::DefineSkill
    }

    fn active_path_step(&self) -> Option<usize> {
        Some(0)
    }

    fn nodes(&self) -> Vec<RoleState> {
        INITIAL_ROLE_DRAFT.to_vec()
    }

    fn title(&self, _view: &CreateSkillView, _cx: &App) -> String {
        "\u{25c2} New skill \u{2014} unnamed".to_string()
    }

    fn state_content(&self, _view: &CreateSkillView, theme: &Theme) -> AnyElement {
        h_flex()
            .gap_x_6()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("DEFINE SKILL"),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("goal stated \u{b7} nothing analysed yet"),
            )
            .into_any_element()
    }

    fn main_area(&self, view: &CreateSkillView, theme: &Theme) -> AnyElement {
        v_flex()
            .gap_y_3()
            .child(define_form_fields(view, theme, false))
            .child(
                div()
                    .max_w(px(720.))
                    .text_size(px(10.))
                    .text_color(theme.muted_foreground)
                    .child(
                        "The agent will read name & goal, match a template, propose acceptance criteria and resolve the method band \u{2014} you review and correct everything in the next step.",
                    ),
            )
            .into_any_element()
    }

    fn actions(&self, view: &CreateSkillView, cx: &mut Context<CreateSkillView>) -> AnyElement {
        let entity = cx.entity().clone();
        h_flex()
            .gap_x_2()
            .child(sbutton_auto(
                "cancel",
                "Cancel",
                view.goto(WizardPhase::Initial, cx),
                cx,
            ))
            .child(sbutton_auto("save-draft", "Save draft", |_, _, _| {}, cx))
            .child(pbutton_auto(
                "analyze-intent",
                "Analyze intent",
                move |_, _, app| {
                    entity.update(app, |this, cx| {
                        this.error.clear();
                        this.agent_log.clear();
                        this.acceptance.clear();
                        let name = this.name_input.read(cx).value().to_string();
                        let goal = this.goal_input.read(cx).value().to_string();
                        match this.jobctl.analyze_intent(name.clone(), goal.clone()) {
                            Ok(id) => this.push_log(format!("→ analyze submitted (root {id})")),
                            Err(err) => this.push_log(format!("\u{2715} submit failed: {err}")),
                        }
                        this.phase = WizardPhase::IntentAnalysis;
                        cx.notify();
                    });
                },
                cx,
            ))
            .into_any_element()
    }
}
