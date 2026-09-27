use gpui_kit::base::{h_flex, v_flex, StyledExt};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::gpui::prelude::FluentBuilder;
use gpui_kit::*;

use crate::element::button::*;
use crate::{FONT_FAMILY, TEXT_SM};

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
    IntentAnalysis,
}

pub enum RoleState {
    Draft(&'static str),
    Defined(&'static str),
    Errored(&'static str),
    Final(&'static str),
}

impl RoleState {
    fn label(&self) -> &'static str {
        match self {
            RoleState::Draft(name)
            | RoleState::Defined(name)
            | RoleState::Errored(name)
            | RoleState::Final(name) => name,
        }
    }

    fn tracked(&self) -> bool {
        !matches!(self, RoleState::Draft(_))
    }
}

pub struct CreateSkillView {
    active_tab: usize,
    phase: WizardPhase,
    nodes: Vec<RoleState>,
    name_input: Entity<InputState>,
    goal_input: Entity<InputState>,
    acceptance: Vec<String>,
}

impl CreateSkillView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            active_tab: 0,
            phase: WizardPhase::Initial,
            nodes: Self::nodes_for_phase(WizardPhase::Initial),
            name_input: cx.new(|cx| InputState::new(window, cx).placeholder("unnamed")),
            goal_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder("what should this skill do, in plain words?")
            }),
            acceptance: Vec::new(),
        }
    }

    fn nodes_for_phase(phase: WizardPhase) -> Vec<RoleState> {
        match phase {
            WizardPhase::Initial => vec![
                RoleState::Draft("intake"),
                RoleState::Draft("\u{2026}"),
                RoleState::Draft("\u{2026}"),
                RoleState::Draft("validate"),
                RoleState::Draft("deliver"),
            ],
            WizardPhase::DefineSkill | WizardPhase::IntentAnalysis => vec![
                RoleState::Defined("QC"),
                RoleState::Defined("FIT"),
                RoleState::Defined("KIN"),
                RoleState::Defined("VAL"),
                RoleState::Defined("RPT"),
            ],
        }
    }

    /// Click handler that transitions the wizard to `phase`.
    fn goto(
        &self,
        phase: WizardPhase,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let entity = cx.entity().clone();
        move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.phase = phase;
                this.nodes = Self::nodes_for_phase(phase);
                cx.notify();
            })
        }
    }

    fn render_tab_content(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        match self.active_tab {
            0 => self.render_my_skills(cx).into_any_element(),
            1 => div().child("SKILL TEMPLATES").into_any_element(),
            _ => div().child("Unknown content").into_any_element(),
        }
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
        let title = if self.phase == WizardPhase::IntentAnalysis {
            let name = self.name_input.read(cx).value().to_string();
            if name.is_empty() {
                "\u{25c2} New skill \u{2014} unnamed".to_string()
            } else {
                format!("\u{25c2} New skill \u{2014} \u{201c}{name}\u{201d}")
            }
        } else {
            "\u{25c2} New skill \u{2014} unnamed".to_string()
        };
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
                    .when(self.phase == WizardPhase::Initial, |row| {
                        row.child(
                            div()
                                .px_2()
                                .py_0p5()
                                .border_1()
                                .border_color(theme.border)
                                .rounded(px(3.))
                                .text_size(px(10.))
                                .text_color(theme.muted_foreground)
                                .child("EMPTY"),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_x_4()
                    .items_stretch()
                    .child(Self::render_role_circle(&theme, &self.nodes))
                    .child(Self::render_skill_state(&theme, self.phase))
                    .child(Self::render_training_path(&theme, self.active_step())),
            )
            .when(self.phase != WizardPhase::Initial, |panel| {
                panel.child(self.render_define_form(&theme, cx))
            })
            .when(self.phase == WizardPhase::Initial, |panel| {
                panel.child(
                    div()
                        .max_w(px(520.))
                        .mt_4()
                        .text_size(TEXT_SM)
                        .text_color(theme.muted_foreground)
                        .child(
                            "A skill is a capability the agent builds and validates against your data \u{2014} not a prompt, a certified competence. The path on the right shows how one gets built.",
                        ),
                )
            })
            .child(self.render_action_row(cx))
            .child(
                div()
                    .text_size(TEXT_SM)
                    .text_color(theme.muted_foreground)
                    .child("Certified skills: (none)"),
            )
    }

    /// Index of the highlighted TRAINING PATH step for the current phase.
    fn active_step(&self) -> Option<usize> {
        match self.phase {
            WizardPhase::Initial => None,
            WizardPhase::DefineSkill | WizardPhase::IntentAnalysis => Some(0),
        }
    }

    fn render_define_form(&self, theme: &Theme, _cx: &mut Context<Self>) -> impl IntoElement {
        let analysed = self.phase == WizardPhase::IntentAnalysis;
        let suggested: [&'static str; 3] = [
            "fit converges",
            "validated on held-out runs",
            "report cites CIs",
        ];
        v_flex()
            .gap_y_3()
            .child(
                v_flex()
                    .gap_y_1()
                    .child(
                        h_flex()
                            .gap_x_2()
                            .items_center()
                            .child(Self::form_label("SKILL NAME", theme))
                            .when(analysed, |row| {
                                row.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(theme.primary_foreground)
                                        .child("\u{25c6} refined by agent"),
                                )
                            }),
                    )
                    .child(Input::new(&self.name_input).w_full().h(px(36.))),
            )
            .child(
                v_flex()
                    .gap_y_1()
                    .child(Self::form_label("GOAL (PLAIN LANGUAGE)", theme))
                    .child(Input::new(&self.goal_input).w_full().h(px(36.))),
            )
            .child(
                v_flex()
                    .gap_y_1()
                    .child(
                        h_flex()
                            .gap_x_2()
                            .items_center()
                            .child(Self::form_label(
                                if analysed {
                                    "ACCEPTANCE CRITERIA"
                                } else {
                                    "WHAT COUNTS AS DONE (ACCEPTANCE CRITERIA)"
                                },
                                theme,
                            ))
                            .when(analysed, |row| {
                                row.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(theme.primary_foreground)
                                        .child(
                                            "\u{25c6} proposed by agent \u{2014} accept or edit",
                                        ),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_x_2()
                            .items_center()
                            .when(!analysed && self.acceptance.is_empty(), |row| {
                                row.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .border_1()
                                        .border_color(theme.border)
                                        .rounded_full()
                                        .text_size(px(10.))
                                        .text_color(theme.muted_foreground)
                                        .child("agent will suggest\u{2026}"),
                                )
                            })
                            .when(analysed, |row| {
                                row.children(suggested.iter().map(|criterion| {
                                    div()
                                        .cursor_pointer()
                                        .px_2()
                                        .py_0p5()
                                        .border_1()
                                        .border_color(theme.ring)
                                        .rounded_full()
                                        .text_size(px(10.))
                                        .text_color(theme.primary_foreground)
                                        .child(format!("{criterion} \u{2713} accept"))
                                }))
                            })
                            .when(!analysed, |row| {
                                row.children(self.acceptance.iter().map(|criterion| {
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .border_1()
                                        .border_color(theme.border)
                                        .rounded_full()
                                        .text_size(px(10.))
                                        .text_color(theme.foreground)
                                        .child(criterion.clone())
                                }))
                            })
                            .child(
                                div()
                                    .cursor_pointer()
                                    .px_2()
                                    .py_0p5()
                                    .border_1()
                                    .border_color(theme.ring)
                                    .rounded_full()
                                    .text_size(px(10.))
                                    .text_color(theme.primary_foreground)
                                    .child(if analysed {
                                        "+ add"
                                    } else {
                                        "+ add yourself"
                                    }),
                            ),
                    ),
            )
            .when(!analysed, |form| {
                form.child(
                    div()
                        .max_w(px(720.))
                        .text_size(px(10.))
                        .text_color(theme.muted_foreground)
                        .child(
                            "The agent will read name & goal, match a template, propose acceptance criteria and resolve the method band \u{2014} you review and correct everything in the next step.",
                        ),
                )
            })
            .when(analysed, |form| {
                form.child(Self::render_analysis_box(theme))
            })
    }

    fn render_analysis_box(theme: &Theme) -> impl IntoElement {
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

    fn form_label(text: &'static str, theme: &Theme) -> Div {
        div()
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .child(text)
    }

    fn render_action_row(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        match self.phase {
            WizardPhase::Initial => h_flex()
                .gap_x_2()
                .child(pbutton_auto(
                    "new-skill",
                    "+ New Skill",
                    self.goto(WizardPhase::DefineSkill, cx),
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
                .into_any_element(),
            WizardPhase::DefineSkill => h_flex()
                .gap_x_2()
                .child(sbutton_auto(
                    "cancel",
                    "Cancel",
                    self.goto(WizardPhase::Initial, cx),
                    cx,
                ))
                .child(sbutton_auto("save-draft", "Save draft", |_, _, _| {}, cx))
                .child(pbutton_auto(
                    "analyze-intent",
                    "Analyze intent",
                    self.goto(WizardPhase::IntentAnalysis, cx),
                    cx,
                ))
                .into_any_element(),
            WizardPhase::IntentAnalysis => h_flex()
                .gap_x_2()
                .child(sbutton_auto(
                    "back-raw",
                    "\u{25c2} back to raw input",
                    self.goto(WizardPhase::DefineSkill, cx),
                    cx,
                ))
                .child(sbutton_auto("save-draft", "Save draft", |_, _, _| {}, cx))
                .child(pbutton_auto(
                    "confirm-attach",
                    "Confirm & attach data",
                    |_, _, _| {},
                    cx,
                ))
                .into_any_element(),
        }
    }

    fn render_role_circle(theme: &Theme, nodes: &[RoleState]) -> impl IntoElement {
        // Nodes of 56px on a ring of radius 76 around the center (120,120) of
        // the 240px circle, evenly spaced starting at the top.
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
                Self::role_node(state, left, top, theme)
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

    fn render_skill_state(theme: &Theme, phase: WizardPhase) -> impl IntoElement {
        let box_ = v_flex()
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
            );
        match phase {
            WizardPhase::Initial => box_
                .child("nothing yet \u{2014} a skill starts with a goal.")
                .child("three roles are fixed: intake \u{2794} validate \u{2794} deliver.")
                .child("the method band between them is the template's choice \u{2014}")
                .child("its node count and names appear once a template is picked.")
                .into_any_element(),
            WizardPhase::DefineSkill => box_
                .child(
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
                        ),
                )
                .into_any_element(),
            WizardPhase::IntentAnalysis => box_
                .child(
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
                                .child("template matched \u{b7} 3 criteria proposed"),
                        ),
                )
                .into_any_element(),
        }
    }

    fn render_training_path(theme: &Theme, active: Option<usize>) -> impl IntoElement {
        let steps: [(&str, &str); 6] = [
            ("1 \u{b7} Define skill", "state goal & acceptance criteria"),
            ("2 \u{b7} Attach data", "upload sensorgrams and run metadata"),
            ("3 \u{b7} Inspect & clarify", "agent profiles the data, you answer its questions"),
            ("4 \u{b7} Review plan", "review the proposed steps, estimates, prerequisites"),
            ("5 \u{b7} Train", "agent builds, fits and validates each step against the data"),
            ("6 \u{b7} Certify", "capability certified \u{2014} artifacts kept & callable"),
        ];
        v_flex()
            .w(px(320.))
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
                    .child(Tab::new().label("SKILL TEMPLATES")),
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
