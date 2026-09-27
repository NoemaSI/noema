use std::sync::Arc;
use gpui_kit::base::v_flex;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::{ImageFormat, px};
use gpui_kit::{Context, Image, ImageSource, IntoElement, ParentElement, Render, Styled, Window, base::StyledExt, component::ActiveTheme, div, img};

use crate::not_logged_in::NOEMA_LOGO;
use crate::FONT_FAMILY;

pub struct CreateSkillView {
    active_tab: usize,
    tab_contents: Vec<String>,
}

impl CreateSkillView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            active_tab: 0,
            tab_contents: vec![
                "MY SKILLS".to_string(),
                "SKILL TEMPLATES".to_string(),
            ],
        }
    }

    fn render_tab_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        match self.active_tab {
            0 => div().child("MY SKILLS"),
            1 => div().child("SKILL TEMPLATES"),
            _ => div().child("Unknown content"),
        }
    }
}

impl Render for CreateSkillView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(
                TabBar::new("content-tabs")
                    .selected_index(self.active_tab)
                    .on_click(cx.listener(|view, index, _, cx| {
                        view.active_tab = *index;
                        cx.notify();
                    }))
                    .child(Tab::new().label("MY SKILLS"))
                    .child(Tab::new().label("SKILL TEMPLATES"))
            )
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .child(self.render_tab_content(cx))
            )
    }
}
