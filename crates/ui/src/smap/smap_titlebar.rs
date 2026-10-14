use gpui_kit::{Context, Entity, IntoElement, ParentElement, Render, Styled, Window, component::{IconName, TitleBar, button::{Button, ButtonVariants}, menu::AppMenuBar}, div};

pub struct SmapTitleBar {
    app_menu_bar: Entity<AppMenuBar>,
}

impl SmapTitleBar {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            app_menu_bar: AppMenuBar::new(cx),
        }
    }

    fn render_left(&self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child("System Map")
            .child(self.app_menu_bar.clone())
    }

    fn render_center(&self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child("center")
    }
}

impl Render for SmapTitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new()
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
