use std::sync::Arc;
use gpui_kit::base::{TreeItem, TreeState, h_flex};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tree::tree;
use gpui_kit::{AppContext, Entity, ImageFormat, px};
use gpui_kit::{Context, Image, ImageSource, IntoElement, ParentElement, Render, Styled, Window, base::StyledExt, component::ActiveTheme, div, img};

use crate::market::market_factory::{MarketSection, build_market};
use crate::not_logged_in::NOEMA_LOGO;
use crate::FONT_FAMILY;


pub struct MarketView {
    logo: Image,
    sections: Vec<MarketSection>,
    tree_state: Entity<TreeState>,
}


impl MarketView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        // TODO: add icons
        // https://gpui-kit.com/component/tree/#file-tree-with-icons
        let tree_state = cx.new(|cx| {
            TreeState::new(cx).items(vec![
                TreeItem::new("src", "src")
                    .expanded(true)
                    .child(TreeItem::new("src/lib.rs", "lib.rs"))
                    .child(TreeItem::new("src/main.rs", "main.rs")),
                TreeItem::new("Cargo.toml", "Cargo.toml"),
                TreeItem::new("README.md", "README.md"),
            ])
        });

        Self {
            logo: Image::from_bytes(ImageFormat::Png, NOEMA_LOGO.to_vec()),
            sections: build_market(),
            tree_state,
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
impl Render for MarketView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let treeview = tree(&self.tree_state, |ix, entry, selected, window, cx| {
            ListItem::new(ix)
                .child(
                    h_flex()
                        .gap_2()
                        .child(entry.item().label.clone())
                )
        });

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
                    .child(treeview)
            )
    }
}


