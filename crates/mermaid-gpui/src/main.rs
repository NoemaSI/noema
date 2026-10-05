//! Demo: renders a couple of Mermaid diagrams natively via the Scene API.
//!
//! `cargo run -p mermaid-gpui --profile release-fast`

use gpui_kit::component::Root;
use gpui_kit::gpui::px;
use gpui_kit::{
    App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window, div,
    point, rgb, size,
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

/// Paints a raw red triangle straight through PathBuilder + paint_path to
/// isolate the GPUI path pipeline from the scene replay.
struct TriangleProbe;

impl gpui_kit::gpui::Element for TriangleProbe {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui_kit::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::LayoutId, ()) {
        (
            window.request_layout(
                gpui_kit::Style {
                    size: gpui_kit::Size {
                        width: gpui_kit::Length::Definite(gpui_kit::DefiniteLength::Fraction(1.0)),
                        height: gpui_kit::Length::Definite(gpui_kit::DefiniteLength::Fraction(
                            1.0,
                        )),
                    },
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        _bounds: Bounds<gpui_kit::Pixels>,
        _request_layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<gpui_kit::Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let mut builder = gpui_kit::gpui::PathBuilder::fill();
        builder.move_to(point(bounds.origin.x + px(50.), bounds.origin.y + px(50.)));
        builder.line_to(point(bounds.origin.x + px(250.), bounds.origin.y + px(50.)));
        builder.line_to(point(bounds.origin.x + px(150.), bounds.origin.y + px(250.)));
        builder.close();
        if let Ok(path) = builder.build() {
            window.paint_path(path, rgb(0xff0000));
        }
    }
}

impl IntoElement for TriangleProbe {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

/// Paints a hand-built scene (solid rect, gradient rect, multi-contour path)
/// through MermaidScene::paint to bisect replay vs scene content.
struct ProbeScene;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Vec<mermaid_gpui::PathCommand> {
    use mermaid_gpui::PathCommand::*;
    vec![
        MoveTo { x, y },
        LineTo { x: x + w, y },
        LineTo { x: x + w, y: y + h },
        LineTo { x, y: y + h },
        Close,
    ]
}

impl gpui_kit::gpui::Element for ProbeScene {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui_kit::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::LayoutId, ()) {
        (
            window.request_layout(
                gpui_kit::Style {
                    size: gpui_kit::Size {
                        width: gpui_kit::Length::Definite(gpui_kit::DefiniteLength::Fraction(1.0)),
                        height: gpui_kit::Length::Definite(gpui_kit::DefiniteLength::Fraction(
                            1.0,
                        )),
                    },
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        _bounds: Bounds<gpui_kit::Pixels>,
        _request_layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<gpui_kit::Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        use mermaid_gpui::{Color, FillRule, GradientStop, Paint, Scene, SceneCommand};
        let black = Color { r: 0, g: 0, b: 0, a: 1.0 };
        let red = Color { r: 255, g: 0, b: 0, a: 1.0 };
        let blue = Color { r: 0, g: 0, b: 255, a: 1.0 };
        let scene = Scene {
            width: 100.0,
            height: 100.0,
            commands: vec![
                SceneCommand::FillPath {
                    path: rect(0., 0., 100., 100.),
                    paint: Paint::Solid(Color { r: 255, g: 255, b: 0, a: 1.0 }),
                    fill_rule: FillRule::NonZero,
                },
                SceneCommand::FillPath {
                    path: rect(5., 5., 25., 25.),
                    paint: Paint::Solid(black),
                    fill_rule: FillRule::NonZero,
                },
                SceneCommand::FillPath {
                    path: rect(35., 5., 25., 25.),
                    paint: Paint::LinearGradient {
                        start: (35., 5.),
                        end: (60., 30.),
                        stops: vec![
                            GradientStop { offset: 0.0, color: red },
                            GradientStop { offset: 1.0, color: blue },
                        ],
                    },
                    fill_rule: FillRule::NonZero,
                },
                SceneCommand::FillPath {
                    path: [rect(65., 5., 25., 25.), rect(70., 10., 15., 15.)].concat(),
                    paint: Paint::Solid(black),
                    fill_rule: FillRule::EvenOdd,
                },
            ],
        };
        let probe = mermaid_gpui::MermaidScene::from_scene(scene);
        probe.paint(bounds, window);
    }
}

impl IntoElement for ProbeScene {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Demo {
    fn new() -> Self {
        let options = mermaid_gpui::mermaid_rs_renderer::RenderOptions::default();
        let primary = MermaidScene::build(DEMO_SOURCE, options.clone())
            .expect("demo flowchart must render");
        let secondary = MermaidScene::build(SECOND_SOURCE, options)
            .expect("demo sequence diagram must render");
        Self { primary, secondary }
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
