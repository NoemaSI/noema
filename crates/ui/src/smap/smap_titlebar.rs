use gpui_kit::{AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window, base::IndexPath, component::{IconName, Sizable, TitleBar, button::{Button, ButtonVariants}, menu::AppMenuBar, select::{SearchableVec, Select, SelectState}}, div, px};

pub struct SmapTitleBar {
    app_menu_bar: Entity<AppMenuBar>,
    selected_map: Entity<SelectState<SearchableVec<SharedString>>>,
}

impl SmapTitleBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {

        let selectable_states = SearchableVec::new(vec![
            SharedString::new("Apple"),
            SharedString::new("Orange"),
            SharedString::new("Banana"),
            SharedString::new("Grape"),
            SharedString::new("Pineapple"),
        ]);

        let select_state = cx.new(|cx| {
            SelectState::new(
                selectable_states,
                Some(IndexPath::default()), // Select first item
                window,
                cx,
            ).searchable(true)
        });

        Self {
            app_menu_bar: AppMenuBar::new(cx),
            selected_map: select_state ,
        }
    }

    fn render_left(&self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let dropd = Select::new(&self.selected_map)
            .large()
            .w(px(320.));
        div()
            .flex()
            .items_start()
            .gap_2()
            .child(self.app_menu_bar.clone())
            .child(dropd)
    }

    fn render_center(&self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            // .child(Select::new(&self.selected_map).small())
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
