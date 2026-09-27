use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Theme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::create_skill_view::CreateSkillView;
use super::wizard::{define_form_fields, RoleState, WizardPhase, WizardStep};
use crate::element::button::*;

/// "Analyze intent" clicked: the agent's interpretation is reviewed.
pub struct IntentAnalysisStep;

impl WizardStep for IntentAnalysisStep {
    fn phase(&self) -> WizardPhase {
        WizardPhase::IntentAnalysis
    }

    fn active_path_step(&self) -> Option<usize> {
        Some(0)
    }

    fn nodes(&self) -> Vec<RoleState> {
        vec![
            RoleState::Defined("QC"),
            RoleState::Defined("FIT"),
            RoleState::Defined("KIN"),
            RoleState::Defined("VAL"),
            RoleState::Defined("RPT"),
        ]
    }

    fn title(&self, view: &CreateSkillView, cx: &App) -> String {
        let name = view.name_input.read(cx).value().to_string();
        if name.is_empty() {
            "\u{25c2} New skill \u{2014} unnamed".to_string()
        } else {
            format!("\u{25c2} New skill \u{2014} \u{201c}{name}\u{201d}")
        }
    }

    fn state_content(&self, view: &CreateSkillView, theme: &Theme) -> AnyElement {
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
                    .child(format!(
                        "template matched \u{b7} {} criteria proposed",
                        view.acceptance.len()
                    )),
            )
            .child(
                div()
                    .text_color(theme.red)
                    .when(!view.error.is_empty(), |div| {
                        div.child(format!(
                            "{}",
                            view.error.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(",")
                        ))
                    } )
            )
            .into_any_element()
    }

    fn main_area(&self, view: &CreateSkillView, theme: &Theme) -> AnyElement {
        v_flex()
            .gap_y_3()
            .child(define_form_fields(view, theme, true))
            .child(Self::analysis_box(theme))
            .into_any_element()
    }

    fn actions(&self, view: &CreateSkillView, cx: &mut Context<CreateSkillView>) -> AnyElement {
        h_flex()
            .gap_x_2()
            .child(sbutton_auto(
                "back-raw",
                "\u{25c2} back to raw input",
                view.goto(WizardPhase::DefineSkill, cx),
                cx,
            ))
            .child(sbutton_auto("save-draft", "Save draft", |_, _, _| {}, cx))
            .child(pbutton_auto(
                "confirm-attach",
                "Confirm & attach data",
                |_, _, _| {},
                cx,
            ))
            .into_any_element()
    }
}

impl IntentAnalysisStep {
    fn analysis_box(theme: &Theme) -> Div {
        fn row(prefix: &'static str, content: &'static str, link: &'static str, theme: &Theme) -> Div {
            h_flex()
                .gap_x_1()
                .text_size(px(10.))
                .child(div().text_color(theme.muted_foreground).child(prefix))
                .child(div().text_color(theme.foreground).child(content))
                .child(
                    div()
                        .cursor_pointer()
                        .text_color(theme.primary_foreground)
                        .child(format!("[{link}]")),
                )
        }
        v_flex()
            .gap_y_1()
            .p_3()
            .border_1()
            .border_color(theme.border)
            .rounded(px(4.))
            .child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child("AGENT ANALYSED THIS AS \u{2014} REVIEW & CONFIRM"),
            )
            .child(row(
                "task type:",
                "protein\u{2013}ligand binding kinetics (SPR) \u{b7} template: Binding kinetics \u{25b8}",
                "swap template",
                theme,
            ))
            .child(row(
                "expected outputs:",
                "Kd/IC50 curves \u{b7} kon/koff \u{b7} report + model card",
                "edit",
                theme,
            ))
            .child(row(
                "required data:",
                "sensorgram CSV \u{b7} run metadata",
                "adjust",
                theme,
            ))
            .child(row(
                "method band resolved:",
                "2 methods \u{2794} FIT \u{b7} KIN \u{2192} wheel becomes QC \u{b7} FIT \u{b7} KIN \u{b7} VAL \u{b7} RPT",
                "swap template to rename",
                theme,
            ))
    }
}
