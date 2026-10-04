use gpui_kit::base::{Disableable, h_flex, v_flex};
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
        WizardPhase::IntentAnalysisWaiting
    }

    fn active_path_step(&self) -> Option<usize> {
        Some(0)
    }

    fn nodes(&self) -> Vec<RoleState> {
        vec![
            RoleState::Draft("intake"),
            RoleState::Draft("…"),
            RoleState::Draft("…"),
            RoleState::Draft("validate"),
            RoleState::Draft("deliver"),
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
            .when(view.skill_pack.is_none(), |div| {
                div
                .text_color(theme.muted_foreground)
                .child(format!(
                    "nothing yet - complete the analysis",
                ))

            })
            .when(view.skill_pack.is_some(), |div| {
                let maybe_skillpack = view.skill_pack.clone().unwrap_or_default();
                let multi_drive = maybe_skillpack.mapping.drives.len() >= 2;
                let multi_out   = maybe_skillpack.mapping.outputs.len() >= 2;
                let drive_label = if multi_drive {"drives" }   else  {"drive"};
                let out_label   = if multi_out   {"outputs" }  else  {"output"};

                div
                .text_color(theme.muted_foreground)
                .child(
                        format!(
                            "template matched \u{b7} {} {}  -> {} {} proposed",
                            maybe_skillpack.mapping.drives.len(),
                            drive_label,
                            maybe_skillpack.mapping.outputs.len(),
                            out_label,
                        )
                    )

            })
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
            .child(
                v_flex()
                    .id("agent-log")
                    .gap_y_1()
                    .p_3()
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(4.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("AGENT LOG"),
                    )
                    .when(view.agent_log.is_empty(), |log| {
                        log.child(
                            div()
                                .text_size(px(10.))
                                .text_color(theme.muted_foreground)
                                .child("waiting for agent events\u{2026}"),
                        )
                    })
                    .children(view.agent_log.iter().map(|line| {
                        div()
                            .text_size(px(10.))
                            .text_color(theme.foreground)
                            .child(line.clone())
                    })),
            )
            .child(Self::analysis_box(view, theme))
            .into_any_element()
    }

    fn actions(&self, view: &CreateSkillView, cx: &mut Context<CreateSkillView>) -> AnyElement {
        let has_skill = view.skill_pack.is_some();
        h_flex()
            .gap_x_2()
            .child(sbutton_auto(
                "back-raw",
                "\u{25c2} back to raw input",
                view.goto(WizardPhase::DefineSkill, cx),
                cx,
            ))
            .child(sbutton_auto("save-draft", "Save draft", |_, _, _| {}, cx))
            .child(
                pbutton_auto(
                    "confirm-attach",
                    "Confirm & attach data",
                    |_, _, _| {},
                    cx,
                )
                    .disabled(!has_skill)
            )
            .into_any_element()
    }
}

impl IntentAnalysisStep {
    fn analysis_box(view: &CreateSkillView, theme: &Theme) -> Div {
        fn row(prefix: &'static str, content: String, theme: &Theme) -> Div {
            h_flex()
                .gap_x_1()
                .text_size(px(10.))
                .child(div().text_color(theme.muted_foreground).child(prefix))
                .child(div().text_color(theme.foreground).child(content))
        }
        fn join(items: &[String]) -> String {
            items.join(" \u{b7} ")
        }
        let shell = v_flex()
            .gap_y_1()
            .p_3()
            .border_1()
            .border_color(theme.border)
            .rounded(px(4.));
        let Some(pack) = &view.skill_pack else {
            return shell.child(
                div()
                    .text_size(px(10.))
                    .text_color(theme.muted_foreground)
                    .child("Agent analysis will appear here\u{2026}".to_uppercase()),
            );
        };
        let drives = pack
            .mapping
            .drives
            .iter()
            .map(|d| format!("{}={} [{}]", d.name, d.column, d.units))
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        let outputs = pack
            .mapping
            .outputs
            .iter()
            .map(|o| format!("{}={} [{}]", o.name, o.column, o.units))
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        shell
            .child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "AGENT ANALYSED THIS AS \u{2014} REVIEW & CONFIRM \u{b7} {}",
                        pack.name
                    )),
            )
            .child(row(
                "data:",
                format!("{} (delimiter \u{201c}{}\u{201d})", pack.data_path.display(), pack.data_delimiter),
                theme,
            ))
            .child(row(
                "columns:",
                format!(
                    "subject: {} \u{b7} curve: {} \u{b7} time: {}",
                    pack.mapping.subject,
                    join(&pack.mapping.curve),
                    pack.mapping.time,
                ),
                theme,
            ))
            .child(row("drives:", drives, theme))
            .child(row("outputs:", outputs, theme))
            .child(row("condition column:", pack.condition_column.clone(), theme))
            .child(row(
                "split:",
                format!(
                    "train: {} \u{b7} validation: {} \u{b7} hidden: {}",
                    join(&pack.split.train),
                    join(&pack.split.validation),
                    join(&pack.split.hidden),
                ),
                theme,
            ))
    }
}
