use std::sync::Arc;
use gpui_kit::component::button::Button;
use gpui_kit::component::{IconName, TitleBar};
use gpui_kit::{AppContext, ImageFormat, hsla, px};
use gpui_kit::{Context, Image, ImageSource, IntoElement, ParentElement, Render, Styled, Window, base::StyledExt, component::ActiveTheme, div, img};
use noema_config::NoemaConfig;
use smap::SystemMap;

use crate::not_logged_in::NOEMA_LOGO;
use crate::FONT_FAMILY;
use crate::smap::smap_titlebar::SmapTitleBar;
use crate::smap::smap_utils::read_system_maps;

pub struct SmapView {
    logo: Image,
    title_bar: gpui_kit::Entity<SmapTitleBar>,
    system_maps: Vec<SystemMap>,
}


impl SmapView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cfg = NoemaConfig::from_home().expect("unable to read config");
        let system_maps = read_system_maps(&cfg);

        let smap_name = system_maps.first().cloned();

        Self {
            logo: Image::from_bytes(ImageFormat::Png, NOEMA_LOGO.to_vec()),
            title_bar: cx.new(|cx| SmapTitleBar::new( smap_name, window, cx)),
            system_maps,
        }
    }
}
impl Render for SmapView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .bg(hsla(0.0, 0.0, 0.0, 0.1))
            .child(self.title_bar.clone())
            // .child(self.render_header(cx))
            .child(
                div()
                    .flex_1()
                    .m_3()
                    .mt_0()
                    .rounded(px(10.))
                    .overflow_hidden()
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .v_flex()
                    .items_start()
                    .px_4()
                    .pt_8()
                    .gap_y_3()
            )
    }
}

