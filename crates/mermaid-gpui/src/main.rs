//! Demo: renders a couple of Mermaid diagrams natively via the Scene API.
//!
//! `cargo run -p mermaid-gpui --profile release-fast`

use gpui_kit::component::Root;
use gpui_kit::gpui::px;
use gpui_kit::{
    AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window, div, rgb, size,
};
use mermaid_gpui::{MermaidScene, mermaid};

const DEMO_SOURCE: &str = r#"
flowchart TD
    A[Fetch world] --> B{Cached?}
    B -- yes --> C[Render scene]
    B -- no --> D[Layout graph]
    D --> C
    C --> E[GPUI paint paths]
"#;

const SECOND_SOURCE: &str = r#"
sequenceDiagram
    participant U as User
    participant V as View
    participant M as mmdr
    U->>V: show diagram
    V->>M: render_scene(source)
    M-->>V: Scene { commands }
    V-->>U: native vectors
"#;

struct Demo {
    primary: MermaidScene,
    secondary: MermaidScene,
}

impl Demo {
    fn new() -> Self {
        let options = mermaid_gpui::mermaid_rs_renderer::RenderOptions::default();
        let primary =
            MermaidScene::build(DEMO_SOURCE, options.clone()).expect("demo flowchart must render");
        let secondary = MermaidScene::build(SECOND_SOURCE, options)
            .expect("demo sequence diagram must render");
        Self {
            primary,
            secondary,
        }
    }
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .child(
                div()
                    .w(px(600.))
                    .h(px(400.))
                    .child(mermaid(self.primary.clone())),
            )
            .child(
                div()
                    .w(px(600.))
                    .h(px(400.))
                    .child(mermaid(self.secondary.clone())),
            )
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(|cx| {
        gpui_kit::init(cx);

        let bounds = Bounds::centered(None, size(px(620.), px(820.)), cx);
        cx.spawn(async move |cx| {
            cx.open_window(
                gpui_kit::WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
                    is_resizable: false,
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|_cx| Demo::new());
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("failed to open demo window");
        })
        .detach();
    });
}