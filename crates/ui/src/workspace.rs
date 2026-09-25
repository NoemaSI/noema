use std::sync::Arc;

use gpui_kit::component::*;
use gpui_kit::gpui::{InteractiveElement, StatefulInteractiveElement};
use gpui_kit::*;

use crate::not_logged_in::NOEMA_LOGO;
use crate::FONT_FAMILY;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceItem {
    EvolveResearch,
    ResumeSession,
    NewDiscovery,
}

pub struct Workspace {
    logo: Image,
    selected: WorkspaceItem,
}

impl Workspace {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            logo: Image::from_bytes(ImageFormat::Png, NOEMA_LOGO.to_vec()),
            selected: WorkspaceItem::EvolveResearch,
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h_flex()
            .items_center()
            .p_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                img(ImageSource::Image(Arc::new(self.logo.clone())))
                    .h(px(60.))
                    .border_0(),
            )
    }

    fn render_item(
        &self,
        item: WorkspaceItem,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.selected == item;
        div()
            .id(id)
            .w(px(360.))
            .p_3()
            .cursor_pointer()
            .rounded(px(4.))
            .border_1()
            .border_color(if selected {
                cx.theme().accent
            } else {
                cx.theme().border
            })
            .bg(if selected {
                cx.theme().overlay
            } else {
                cx.theme().background
            })
            .font_family(FONT_FAMILY)
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = item;
                cx.notify();
            }))
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .child(self.render_header(cx))
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .items_start()
                    .px_4()
                    .pt_8()
                    .gap_y_3()
                    .child(self.render_item(
                        WorkspaceItem::EvolveResearch,
                        "evolve-research",
                        "Evolve research",
                        cx,
                    ))
                    .child(self.render_item(
                        WorkspaceItem::ResumeSession,
                        "resume-session",
                        "Resume session",
                        cx,
                    ))
                    .child(self.render_item(
                        WorkspaceItem::NewDiscovery,
                        "new-discovery",
                        "New discovery",
                        cx,
                    )),
            )
    }
}
