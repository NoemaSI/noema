use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::Theme;
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::*;

use super::create_skill_view::CreateSkillView;
use super::step_define_skill::DefineSkillStep;
use super::step_initial::InitialStep;
use super::step_intent_analysis::IntentAnalysisStep;
use crate::TEXT_SM;

pub const INITIAL_ROLE_DRAFT: &[RoleState] = &[
    RoleState::Draft("intake"),
    RoleState::Draft("…"),
    RoleState::Draft("…"),
    RoleState::Draft("validate"),
    RoleState::Draft("deliver"),
];


/// The create-skill wizard walks the user through up to ten phases.
/// Implemented so far: Initial, DefineSkill, IntentAnalysis.
/// Planned: AttachData, Inspect, ReviewPlan, Train, Validate, Certify, Done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardPhase {
    /// Nothing interacted with yet; landing state.
    Initial,
    /// "New Skill" clicked: goal + acceptance criteria form is open.
    DefineSkill,
    /// "Analyze intent" clicked: agent reads name & goal.
    IntentAnalysisWaiting,
    IntentAnalysisDone,
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum RoleState {
    Draft(&'static str),
    Defined(&'static str),
    Errored(&'static str),
    Final(&'static str),
}

/// One step of the wizard. Each phase implements this trait and lives in its
/// own file; the view renders the shell and delegates everything step-specific.
pub trait WizardStep {
    /// The phase this step represents.
    #[allow(dead_code)]
    fn phase(&self) -> WizardPhase;

    /// Index of the highlighted TRAINING PATH step (0-based), if any.
    fn active_path_step(&self) -> Option<usize>;

    /// Nodes drawn on the role-circle spoke diagram.
    fn nodes(&self) -> Vec<RoleState>;

    /// Header title, e.g. "◂ New skill — unnamed".
    fn title(&self, view: &CreateSkillView, cx: &App) -> String;

    /// Optional badge rendered next to the title.
    fn badge(&self) -> Option<&'static str> {
        None
    }

    /// Content of the SKILL STATE box, below its heading.
    fn state_content(&self, view: &CreateSkillView, theme: &Theme) -> AnyElement;

    /// Main area between the circle row and the action row.
    fn main_area(&self, view: &CreateSkillView, theme: &Theme) -> AnyElement;

    /// Bottom button row.
    fn actions(&self, view: &CreateSkillView, cx: &mut Context<CreateSkillView>) -> AnyElement;
}

pub fn step_for(phase: WizardPhase) -> &'static dyn WizardStep {
    match phase {
        WizardPhase::Initial => &InitialStep,
        WizardPhase::DefineSkill => &DefineSkillStep,
        WizardPhase::IntentAnalysisWaiting => &IntentAnalysisStep,
        WizardPhase::IntentAnalysisDone => &IntentAnalysisStep,
    }
}

/// Small bordered badge (e.g. "EMPTY").
pub fn badge_pill(label: &'static str, theme: &Theme) -> Div {
    div()
        .px_2()
        .py_0p5()
        .border_1()
        .border_color(theme.border)
        .rounded(px(3.))
        .text_size(px(10.))
        .text_color(theme.muted_foreground)
        .child(label)
}

/// The SKILL STATE box: heading plus the step's state content.
pub fn state_box(step: &dyn WizardStep, view: &CreateSkillView, theme: &Theme) -> Div {
    v_flex()
        .flex_1()
        .gap_y_1()
        .p_3()
        .border_1()
        .border_color(theme.border)
        .rounded(px(4.))
        .text_size(TEXT_SM)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child("SKILL STATE"),
        )
        .child(step.state_content(view, theme))
}

/// Nodes of 56px on a ring of radius 76 around the center (120,120) of the
/// 240px circle, evenly spaced starting at the top, with spokes to the center.
pub fn role_circle(theme: &Theme, nodes: &[RoleState]) -> impl IntoElement {
    let rgb = theme.border.to_rgb();
    let hex = format!(
        "#{:02x}{:02x}{:02x}",
        (rgb.r * 255.) as u8,
        (rgb.g * 255.) as u8,
        (rgb.b * 255.) as u8
    );
    let n = nodes.len().max(1) as f32;
    let mut spokes = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240"><g fill="{}">"#,
        hex
    );
    for i in 0..nodes.len() {
        let angle = -90. + i as f32 * 360. / n;
        spokes.push_str(&format!(
            r#"<rect x="120" y="119.5" width="76" height="1" transform="rotate({angle} 120 120)"/>"#
        ));
    }
    spokes.push_str("</g></svg>");
    div()
        .relative()
        .size(px(240.))
        .flex_none()
        .border_1()
        .border_color(theme.border)
        .rounded_full()
        .child(
            svg()
                .absolute()
                .left_0()
                .top_0()
                .size(px(240.))
                .text_color(theme.border)
                .data(spokes.as_bytes()),
        )
        .children(nodes.iter().enumerate().map(|(i, state)| {
            let a = (i as f32 * 360. / n - 90.) * std::f32::consts::PI / 180.;
            let left = px(120. + 76. * a.cos() - 28.);
            let top = px(120. + 76. * a.sin() - 28.);
            role_node(state, left, top, theme)
        }))
}

fn role_node(state: &RoleState, left: Pixels, top: Pixels, theme: &Theme) -> Div {
    let base = div()
        .absolute()
        .left(left)
        .top(top)
        .size(px(56.))
        .rounded_full()
        .bg(theme.overlay)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.));
    match state {
        RoleState::Draft(name) => base
            .border_2()
            .border_dashed()
            .border_color(theme.border)
            .text_color(theme.muted_foreground)
            .child(*name),
        RoleState::Defined(name) => base
            .border_1()
            .border_color(theme.border)
            .text_color(theme.muted_foreground)
            .child(*name),
        RoleState::Final(_name) => base
            .border_2()
            .border_color(theme.foreground)
            .text_color(theme.foreground)
            .child("\u{2713}"),
        RoleState::Errored(_name) => base
            .border_2()
            .border_color(theme.danger)
            .text_color(theme.danger)
            .child("\u{2715}"),
    }
}

/// The TRAINING PATH panel with the given step highlighted (0-based).
pub fn training_path(theme: &Theme, active: Option<usize>) -> impl IntoElement {
    let steps: [(&str, &str); 6] = [
        ("1 \u{b7} Define skill", "state goal & acceptance criteria"),
        ("2 \u{b7} Attach data", "upload sensorgrams and run metadata"),
        ("3 \u{b7} Inspect & clarify", "agent profiles the data, you answer its questions"),
        ("4 \u{b7} Review plan", "review the proposed steps, estimates, prerequisites"),
        ("5 \u{b7} Train", "agent builds, fits and validates each step against the data"),
        ("6 \u{b7} Certify", "capability certified \u{2014} artifacts kept & callable"),
    ];
    v_flex()
        .w(px(340.))
        .flex_none()
        .gap_y_3()
        .p_3()
        .bg(theme.overlay)
        .border_1()
        .border_color(theme.border)
        .rounded(px(4.))
        .text_size(TEXT_SM)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child("TRAINING PATH"),
        )
        .children(steps.iter().enumerate().map(|(i, (title, desc))| {
            let is_active = active == Some(i);
            h_flex()
                .gap_x_2()
                .child(
                    div()
                        .mt(px(4.))
                        .size(px(6.))
                        .flex_none()
                        .rounded_full()
                        .map(|dot| {
                            if is_active {
                                dot.bg(theme.primary_foreground)
                            } else {
                                dot.border_1().border_color(theme.border)
                            }
                        }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_color(if is_active {
                                    theme.primary_foreground
                                } else {
                                    theme.foreground
                                })
                                .font_weight(if is_active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .child(*title),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(theme.muted_foreground)
                                .child(*desc),
                        ),
                )
        }))
}

/// Small uppercase label above a form field.
pub fn form_label(text: &'static str, theme: &Theme) -> Div {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(text)
}

/// Cyan "◆ \u{2026}" annotation next to a form label.
pub fn annotation(text: &'static str, theme: &Theme) -> Div {
    div()
        .text_size(px(10.))
        .text_color(theme.primary_foreground)
        .child(text)
}

/// Rounded-full pill.
pub fn pill(label: impl Into<SharedString>, border: Hsla, fg: Hsla) -> Div {
    div()
        .cursor_pointer()
        .px_2()
        .py_0p5()
        .border_1()
        .border_color(border)
        .rounded_full()
        .text_size(px(10.))
        .text_color(fg)
        .child(label.into())
}

pub fn render_filedrop(view: &CreateSkillView, theme: &Theme) -> Div {
    v_flex()
        .child(view.filedrop.clone())
}

/// SKILL NAME / GOAL / ACCEPTANCE CRITERIA fields shared by the DefineSkill
/// and IntentAnalysis steps. `analysed` switches to the agent-proposed view.
pub fn define_form_fields(view: &CreateSkillView, theme: &Theme, analysed: bool) -> Div {
    v_flex()
        .gap_y_3()
        .child(
            v_flex()
                .gap_y_1()
                .child(
                    h_flex()
                        .gap_x_2()
                        .items_center()
                        .child(form_label("SKILL NAME", theme))
                        .when(analysed, |row| {
                            row.child(annotation("\u{25c6} refined by agent", theme))
                        }),
                )
                .child(Input::new(&view.name_input).w_full().h(px(36.))),
        )
        .child(
            v_flex()
                .gap_y_1()
                .child(form_label("GOAL (PLAIN LANGUAGE)", theme))
                .child(Textarea::new(&view.goal_input).w_full().h(px(76.))),
        )
        .child(
            v_flex()
                .gap_y_1()
                .child(
                    h_flex()
                        .gap_x_2()
                        .items_center()
                        .child(form_label(
                            if analysed {
                                "ACCEPTANCE CRITERIA"
                            } else {
                                "WHAT COUNTS AS DONE (ACCEPTANCE CRITERIA)"
                            },
                            theme,
                        ))
                        .when(analysed, |row| {
                            row.child(annotation(
                                "\u{25c6} proposed by agent \u{2014} accept or edit",
                                theme,
                            ))
                        }),
                )
                .child(render_filedrop(&view, &theme))
                .child(
                    h_flex()
                        .gap_x_2()
                        .items_center()
                        .when(!analysed && view.acceptance.is_empty(), |row| {
                            row.child(pill(
                                "agent will suggest\u{2026}",
                                theme.border,
                                theme.muted_foreground,
                            ))
                        })
                        .when(analysed, |row| {
                            row.children(view.acceptance.iter().map(|criterion| {
                                pill(
                                    format!("{criterion} \u{2713} accept"),
                                    theme.ring,
                                    theme.primary_foreground,
                                )
                            }))
                        })
                        .when(!analysed, |row| {
                            row.children(view.acceptance.iter().map(|criterion| {
                                pill(criterion.clone(), theme.border, theme.foreground)
                            }))
                        })
                        .child(pill(
                            if analysed {
                                "+ add"
                            } else {
                                "+ add yourself"
                            },
                            theme.ring,
                            theme.primary_foreground,
                        )),
                ),
        )
}
