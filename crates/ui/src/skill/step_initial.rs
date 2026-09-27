use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Theme;
use gpui_kit::*;

use super::create_skill_view::CreateSkillView;
use super::wizard::{RoleState, WizardPhase, WizardStep};
use crate::element::button::*;
use crate::TEXT_SM;
use crate::skill::wizard::INITIAL_ROLE_DRAFT;

/// Landing state: nothing interacted with yet.
pub struct InitialStep;

impl WizardStep for InitialStep {
    fn phase(&self) -> WizardPhase {
        WizardPhase::Initial
    }

    fn active_path_step(&self) -> Option<usize> {
        None
    }

    fn nodes(&self) -> Vec<RoleState> {
        INITIAL_ROLE_DRAFT.to_vec()
    }

    fn title(&self, _view: &CreateSkillView, _cx: &App) -> String {
        "\u{25c2} New skill \u{2014} unnamed".to_string()
    }

    fn badge(&self) -> Option<&'static str> {
        Some("EMPTY")
    }

    fn state_content(&self, _theme: &Theme) -> AnyElement {
        v_flex()
            .gap_y_1()
            .child("nothing yet \u{2014} a skill starts with a goal.")
            .child("three roles are fixed: intake \u{2794} validate \u{2794} deliver.")
            .child("the method band between them is the template's choice \u{2014}")
            .child("its node count and names appear once a template is picked.")
            .into_any_element()
    }

    fn main_area(&self, _view: &CreateSkillView, theme: &Theme) -> AnyElement {
        div()
            .max_w(px(520.))
            .mt_4()
            .text_size(TEXT_SM)
            .text_color(theme.muted_foreground)
            .child(
                "A skill is a capability the agent builds and validates against your data \u{2014} not a prompt, a certified competence. The path on the right shows how one gets built.",
            )
            .into_any_element()
    }

    fn actions(&self, view: &CreateSkillView, cx: &mut Context<CreateSkillView>) -> AnyElement {
        h_flex()
            .gap_x_2()
            .child(pbutton_auto(
                "new-skill",
                "+ New Skill",
                view.goto(WizardPhase::DefineSkill, cx),
                cx,
            ))
            .child(sbutton_auto(
                "browse-templates",
                "Browse templates",
                |_, _, _| {},
                cx,
            ))
            .child(sbutton_auto(
                "import-plan",
                "Import plan",
                |_, _, _| {},
                cx,
            ))
            .into_any_element()
    }
}
