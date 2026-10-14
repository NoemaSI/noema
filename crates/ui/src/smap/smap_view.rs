use std::sync::Arc;
use gpui_kit::component::button::Button;
use gpui_kit::component::{IconName, TitleBar};
use gpui_kit::{AppContext, ImageFormat, px};
use gpui_kit::{Context, Image, ImageSource, IntoElement, ParentElement, Render, Styled, Window, base::StyledExt, component::ActiveTheme, div, img};

use crate::not_logged_in::NOEMA_LOGO;
use crate::FONT_FAMILY;
use crate::smap::smap_titlebar::SmapTitleBar;

pub struct SmapView {
    logo: Image,
    title_bar: gpui_kit::Entity<SmapTitleBar>,
}


impl SmapView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            logo: Image::from_bytes(ImageFormat::Png, NOEMA_LOGO.to_vec()),
            title_bar: cx.new(|cx| SmapTitleBar::new(window, cx)),
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
}
impl Render for SmapView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .child(self.title_bar.clone())
            .child(self.render_header(cx))
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .items_start()
                    .px_4()
                    .pt_8()
                    .gap_y_3()
            )
    }
}

