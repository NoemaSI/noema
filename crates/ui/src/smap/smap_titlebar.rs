use gpui_kit::{AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window, base::IndexPath, component::{ActiveTheme, Icon, IconName, Sizable, TitleBar, button::{Button, ButtonVariants}, menu::AppMenuBar, select::{SearchableVec, Select, SelectState}}, div, hsla, prelude::FluentBuilder, px, transparent_black};
use smap::SystemMap;

    // let selectable_states = SearchableVec::new(vec![
    //     SharedString::new("Apple"),
    //     SharedString::new("Orange"),
    //     SharedString::new("Banana"),
    //     SharedString::new("Grape"),
    //     SharedString::new("Pineapple"),
    // ]);
    //
    // let select_state = cx.new(|cx| {
    //     SelectState::new(
    //         selectable_states,
    //         Some(IndexPath::default()), // Select first item
    //         window,
    //         cx,
    //     ).searchable(true)
    // });
    //
        // let dropd = Select::new(&self.selected_map)
        //     .large()
        //     .w(px(320.));

pub struct SmapTitleBar {
    app_menu_bar: Entity<AppMenuBar>,
    current_open: Option<SystemMap>,
    // selected_map: Entity<SelectState<SearchableVec<SharedString>>>,
}

impl SmapTitleBar {
    pub fn new(current_open: Option<SystemMap>, window: &mut Window, cx: &mut Context<Self>) -> Self {


        Self {
            app_menu_bar: AppMenuBar::new(cx),
            current_open,
            // selected_map: select_state ,
        }
    }

    fn render_left(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let maybe_open = self.current_open.is_some();

        let smap_name = match &self.current_open {
            Some(smap) => &smap.name,
            None => &"n/a".to_string()
        };

        div()
            .flex()
            .items_center()
            .gap_2()
            .child(self.app_menu_bar.clone())
            .when(maybe_open, |div| {
                div
                  .child(self.render_pill(smap_name.clone(), cx))
            })
    }

    fn render_center(&self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
        // .child(Select::new(&self.selected_map).small())
    }

    fn render_pill(&self, name: String, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .h(px(24.))
            .px_2()
            .rounded(px(8.))
            //.bg(cx.theme().background)
            .bg(hsla(0.0, 0.0, 0.0, 0.1))
            .border_1()
            .border_color(cx.theme().border)
            .text_color(cx.theme().foreground)
            .child(Icon::new(IconName::SquareTerminal).small())
            .child(name)
            .child(
                Button::new("close-pill")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close),
            )
    }
}

impl Render for SmapTitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new()
            .bg(transparent_black())
            .child(self.render_left(window, cx))
            .child(self.render_center(window, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("settings")
                            .ghost()
                            .icon(IconName::Settings)
                    )
                    .child(
                        Button::new("help")
                            .ghost()
                            .icon(IconName::SquareTerminal)
                    )
            )
    }
}
