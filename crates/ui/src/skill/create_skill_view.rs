use gpui_kit::base::{h_flex, v_flex, StyledExt};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::*;

use crate::element::button::*;
use crate::{FONT_FAMILY, TEXT_SM};

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
    nodes: Vec<RoleState>,
}

impl CreateSkillView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            active_tab: 0,
            nodes: vec![
                RoleState::Draft("intake"),
                RoleState::Draft("\u{2026}"),
                RoleState::Draft("\u{2026}"),
                RoleState::Draft("validate"),
                RoleState::Draft("deliver"),
            ],
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
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("\u{25c2} New skill \u{2014} unnamed")
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(3.))
                            .text_size(px(10.))
                            .text_color(theme.muted_foreground)
                            .child("EMPTY"),
                    ),
            )
            .child(
                h_flex()
                    .gap_x_4()
                    .items_start()
                    .child(Self::render_role_circle(&theme, &self.nodes))
                    .child(Self::render_skill_state(&theme))
                    .child(Self::render_training_path(&theme)),
            )
            .child(
                div()
                    .max_w(px(520.))
                    .text_size(TEXT_SM)
                    .text_color(theme.muted_foreground)
                    .child(
                        "A skill is a capability the agent builds and validates against your data \u{2014} not a prompt, a certified competence. The path on the right shows how one gets built.",
                    ),
            )
            .child(
                h_flex()
                    .gap_x_2()
                    .child(pbutton_auto(
                        "new-skill",
                        "+ New Skill",
                        |_, _, _| {},
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
                    )),
            )
            .child(
                div()
                    .text_size(TEXT_SM)
                    .text_color(theme.muted_foreground)
                    .child("Certified skills: (none)"),
            )
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

    fn render_skill_state(theme: &Theme) -> impl IntoElement {
        v_flex()
            .flex_1()
            .gap_y_1()
            .p_3()
            .border_1()
            .border_color(theme.border)
            .border_dashed()
            .rounded(px(4.))
            .text_size(TEXT_SM)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child("SKILL STATE"),
            )
            .child("nothing yet \u{2014} a skill starts with a goal.")
            .child("three roles are fixed: intake \u{2794} validate \u{2794} deliver.")
            .child("the method band between them is the template's choice \u{2014}")
            .child("its node count and names appear once a template is picked.")
    }

    fn render_training_path(theme: &Theme) -> impl IntoElement {
        let steps: [(&str, &str); 6] = [
            ("1 \u{b7} Define skill", "state goal & acceptance criteria"),
            ("2 \u{b7} Attach data", "upload sensorgrams and run metadata"),
            ("3 \u{b7} Inspect & clarify", "agent profiles the data, you answer its questions"),
            ("4 \u{b7} Review plan", "review the proposed steps, estimates, prerequisites"),
            ("5 \u{b7} Train", "agent builds, fits and validates each step against the data"),
            ("6 \u{b7} Certify", "capability certified \u{2014} artifacts kept & callable"),
        ];
        v_flex()
            .w(px(260.))
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
            .children(steps.iter().map(|(title, desc)| {
                v_flex()
                    .gap_0p5()
                    .child(div().text_color(theme.foreground).child(*title))
                    .child(div().text_size(px(10.)).text_color(theme.muted_foreground).child(*desc))
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
