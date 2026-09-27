use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::*;
use gpui_kit::gpui::{InteractiveElement, StatefulInteractiveElement};
use gpui_kit::*;

use crate::market::market_view::MarketView;
use crate::not_logged_in::NOEMA_LOGO;
use crate::FONT_FAMILY;
use crate::world::world_view::WorldView;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceItem {
    EvolveResearch,
    ViewMarket,
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
        open: Rc<dyn Fn(&mut Window, &mut App)>,
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
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = item;

                // open a new window with the view
                open(window, cx);

                cx.notify();
            }))
    }
}

fn open_world_view(_window: &mut Window, cx: &mut App) {
    let _ = cx.open_window(WindowOptions::default(), |window, cx| {
        let view = cx.new(|cx| WorldView::new(window, cx));
        // This first level on the window, should be a Root.
        cx.new(|cx| Root::new(view, window, cx))
    });
}

fn open_market_view(_window: &mut Window, cx: &mut App) {
    let _ = cx.open_window(WindowOptions::default(), |window, cx| {
        let view = cx.new(|cx| MarketView::new(window, cx));
        // This first level on the window, should be a Root.
        cx.new(|cx| Root::new(view, window, cx))
    });
}

fn open_noop(_window: &mut Window, _cx: &mut App) {}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let world_view: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(open_world_view);
        let market_view: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(open_market_view);
        let noop: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(open_noop);

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
                        world_view,
                        cx,
                    ))
                    .child(self.render_item(
                        WorkspaceItem::ViewMarket,
                        "view-market",
                        "Regional market",
                        market_view,
                        cx,
                    ))
                    .child(self.render_item(
                        WorkspaceItem::ResumeSession,
                        "resume-session",
                        "Resume session",
                        noop.clone(),
                        cx,
                    ))
                    .child(self.render_item(
                        WorkspaceItem::NewDiscovery,
                        "new-discovery",
                        "New discovery",
                        noop,
                        cx,
                    )),
            )
    }
}
