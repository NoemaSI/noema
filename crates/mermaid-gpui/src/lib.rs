//! Native GPUI rendering of Mermaid diagrams.
//!
//! Uses the `mermaid-rs-renderer` native vector **Scene API** (`render_scene`)
//! and replays the resulting display list into GPUI paint primitives
//! (`PathBuilder` → `Path` → `Window::paint_path`). Nothing is rasterized:
//! text arrives as glyph outlines and strokes arrive as filled paths.
//!
//! # Approximations
//!
//! The Scene API is richer than GPUI's quad/path painter. The following
//! scene features are approximated rather than silently dropped:
//!
//! - `PushClip` paths are approximated by their axis-aligned bounding box
//!   (GPUI content masks are rectangular).
//! - `PushLayer` group opacity is applied by multiplying the layer opacity
//!   into each enclosed paint's alpha (GPUI has no offscreen group
//!   compositing). This differs from true group compositing when enclosed
//!   primitives overlap.
//! - `BlendMode::Multiply` is painted as `Normal` (GPUI has no blend modes).
//! - Multi-stop linear gradients are reduced to their first and last stops
//!   and re-anchored to the path bounds with the gradient axis angle
//!   (GPUI gradients are angle-based two-stop).

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::gpui::{
    App, Background, Bounds, ContentMask, Context, CursorStyle, DefiniteLength, Element, ElementId,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, Length,
    LinearColorStop, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder,
    PathStyle, PinchEvent, Pixels, Point, Render, Rgba, ScrollDelta, ScrollWheelEvent, Size, Style,
    Styled, Window, div, linear_gradient, point, px,
};
use mermaid_rs_renderer::{RenderOptions, render_scene};

pub use mermaid_rs_renderer;
pub use mermaid_rs_renderer::scene::{
    BlendMode, Color, FillRule, GradientStop, Paint, PathCommand, Scene, SceneCommand,
};

/// A cached, rendered Mermaid diagram ready for GPUI painting.
#[derive(Clone, Debug)]
pub struct MermaidScene {
    scene: Scene,
}

impl MermaidScene {
    /// Parse, layout and normalize a Mermaid source into a paintable scene.
    ///
    /// This is CPU work (layout + font shaping) and should not run every
    /// frame; render once and clone the result into the view.
    pub fn build(source: &str, options: RenderOptions) -> anyhow::Result<Self> {
        let scene = render_scene(source, options)?;
        anyhow::ensure!(scene.width > 0.0 && scene.height > 0.0, "empty scene");
        Ok(Self { scene })
    }

    /// Wrap an already-built [`Scene`] for painting.
    pub fn from_scene(scene: Scene) -> Self {
        Self { scene }
    }

    /// Intrinsic canvas size in logical pixels.
    pub fn size(&self) -> Size<f32> {
        Size {
            width: self.scene.width,
            height: self.scene.height,
        }
    }

    /// Uniform fit-scale needed to draw this scene into `bounds`.
    pub fn fit_scale(&self, bounds: Bounds<Pixels>) -> f32 {
        let w: f32 = bounds.size.width.into();
        let h: f32 = bounds.size.height.into();
        if self.scene.width <= 0.0 || self.scene.height <= 0.0 || w <= 0.0 || h <= 0.0 {
            return 0.0;
        }
        (w / self.scene.width).min(h / self.scene.height)
    }

    /// Screen origin of the scene's (0,0) and the effective scale for a given
    /// viewport `bounds`, `zoom` (multiplier on the fit scale) and `pan`
    /// (screen-pixel offset). Shared by [`MermaidScene::paint_view`] and the
    /// zoom/pan handlers so their transforms stay in sync.
    pub fn view_origin(&self, bounds: Bounds<Pixels>, zoom: f32, pan: Point<Pixels>) -> (Point<Pixels>, f32) {
        let scale = self.fit_scale(bounds) * zoom;
        let origin = point(
            bounds.origin.x + (bounds.size.width - px(self.scene.width * scale)) / 2.0 + pan.x,
            bounds.origin.y + (bounds.size.height - px(self.scene.height * scale)) / 2.0 + pan.y,
        );
        (origin, scale)
    }

    /// Replay the scene fit-centered into `bounds` (identity view transform).
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        self.paint_view(bounds, window, 1.0, point(px(0.), px(0.)));
    }

    /// Replay the scene with an additional view transform.
    ///
    /// `zoom` multiplies the fit-to-bounds scale (1.0 = fit); `pan` offsets the
    /// result in screen pixels. Content is clipped to `bounds` so panning and
    /// zooming move the diagram within the element rather than outside it.
    pub fn paint_view(
        &self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        zoom: f32,
        pan: Point<Pixels>,
    ) {
        let (origin, scale) = self.view_origin(bounds, zoom, pan);
        if scale <= 0.0 {
            return;
        }
        {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 40 {
                eprintln!(
                    "DBG paint #{n} bounds=({},{},{},{}) zoom={zoom:.3} pan=({},{}) origin=({},{}) scale={scale:.3}",
                    p(bounds.origin.x), p(bounds.origin.y), p(bounds.size.width), p(bounds.size.height),
                    p(pan.x), p(pan.y), p(origin.x), p(origin.y),
                );
            }
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.paint_commands(&self.scene.commands, origin, scale, 1.0, window);
        });
    }

    fn paint_commands(
        &self,
        commands: &[SceneCommand],
        origin: Point<Pixels>,
        scale: f32,
        opacity: f32,
        window: &mut Window,
    ) {
        let mut i = 0;
        while i < commands.len() {
            match &commands[i] {
                SceneCommand::FillPath {
                    path,
                    paint,
                    fill_rule,
                } => {
                    paint_path(path, paint, *fill_rule, origin, scale, opacity, window);
                    i += 1;
                }
                SceneCommand::PushClip { path, fill_rule } => {
                    // matching_end is relative to the sub-slice starting at i.
                    let end = i + matching_end(&commands[i..], |c| matches!(c, SceneCommand::PopClip));
                    if let Some(mask_bounds) = clip_bounds(path, *fill_rule, origin, scale) {
                        window.with_content_mask(
                            Some(ContentMask { bounds: mask_bounds }),
                            |window| {
                                self.paint_commands(
                                    &commands[i + 1..end],
                                    origin,
                                    scale,
                                    opacity,
                                    window,
                                );
                            },
                        );
                    } else {
                        self.paint_commands(
                            &commands[i + 1..end],
                            origin,
                            scale,
                            opacity,
                            window,
                        );
                    }
                    i = end + 1;
                }
                SceneCommand::PushLayer {
                    opacity: layer_opacity,
                    blend_mode,
                } => {
                    let end = i
                        + matching_end(&commands[i..], |c| matches!(c, SceneCommand::PopLayer));
                    if matches!(blend_mode, BlendMode::Multiply) {
                        log_multiply_fallback();
                    }
                    self.paint_commands(
                        &commands[i + 1..end],
                        origin,
                        scale,
                        opacity * layer_opacity,
                        window,
                    );
                    i = end + 1;
                }
                // Stray pop (unbalanced input) - ignore.
                SceneCommand::PopClip | SceneCommand::PopLayer => i += 1,
            }
        }
    }
}

fn log_multiply_fallback() {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    LOGGED.call_once(|| {
        eprintln!(
            "mermaid-gpui: Multiply blend mode painted as Normal (GPUI has no blending support)"
        );
    });
}

/// Index of the closing command matching the opener at offset 0 of `slice`.
/// Returns `slice.len() - 1` when unbalanced.
fn matching_end(slice: &[SceneCommand], is_close: impl Fn(&SceneCommand) -> bool) -> usize {
    let mut depth = 0i32;
    for (idx, command) in slice.iter().enumerate() {
        match command {
            SceneCommand::PushClip { .. } | SceneCommand::PushLayer { .. } => depth += 1,
            SceneCommand::PopClip | SceneCommand::PopLayer => {
                depth -= 1;
                if depth == 0 && is_close(command) {
                    return idx;
                }
            }
            SceneCommand::FillPath { .. } => {}
        }
    }
    slice.len() - 1
}

fn to_point(p: (f32, f32), origin: Point<Pixels>, scale: f32) -> Point<Pixels> {
    point(
        origin.x + px(p.0 * scale),
        origin.y + px(p.1 * scale),
    )
}

fn paint_path(
    path: &[PathCommand],
    paint: &Paint,
    fill_rule: FillRule,
    origin: Point<Pixels>,
    scale: f32,
    opacity: f32,
    window: &mut Window,
) {
    let mut builder = PathBuilder::fill();
    match fill_rule {
        FillRule::NonZero => {
            builder.style = PathStyle::Fill(lyon_fill_options(false));
        }
        FillRule::EvenOdd => {
            builder.style = PathStyle::Fill(lyon_fill_options(true));
        }
    }
    for command in path {
        match *command {
            PathCommand::MoveTo { x, y } => builder.move_to(to_point((x, y), origin, scale)),
            PathCommand::LineTo { x, y } => builder.line_to(to_point((x, y), origin, scale)),
            PathCommand::QuadTo { x1, y1, x, y } => builder.curve_to(
                to_point((x, y), origin, scale),
                to_point((x1, y1), origin, scale),
            ),
            PathCommand::CubicTo { x1, y1, x2, y2, x, y } => builder.cubic_bezier_to(
                to_point((x, y), origin, scale),
                to_point((x1, y1), origin, scale),
                to_point((x2, y2), origin, scale),
            ),
            PathCommand::Close => builder.close(),
        }
    }
    let Ok(path) = builder.build() else {
        // Rare lyon failure (e.g. TooManyVertices on a pathological path).
        // Skip this primitive rather than aborting the whole diagram.
        return;
    };
    window.paint_path(path, to_background(paint, scale, opacity));
}

fn lyon_fill_options(even_odd: bool) -> gpui_kit::gpui::FillOptions {
    let mut options = gpui_kit::gpui::FillOptions::default();
    if even_odd {
        options = options.with_fill_rule(gpui_kit::gpui::FillRule::EvenOdd);
    }
    options
}

fn to_background(paint: &Paint, scale: f32, opacity: f32) -> Background {
    match paint {
        Paint::Solid(color) => to_rgba(*color, opacity).into(),
        Paint::LinearGradient { start, end, stops } => {
            let Some(first) = stops.first() else {
                return Background::default();
            };
            let Some(last) = stops.last() else {
                return Background::default();
            };
            // GPUI gradients are angle-based over the bounds; convert the
            // canvas-local axis into an angle (0 deg = pointing up, CW).
            let dx = (end.0 - start.0) * scale;
            let dy = (end.1 - start.1) * scale;
            let angle = if dx == 0.0 && dy == 0.0 {
                0.0
            } else {
                let deg = dy.atan2(dx).to_degrees(); // 0 = +x axis, CCW
                (90.0 - deg).rem_euclid(360.0)
            };
            let from = LinearColorStop {
                color: to_rgba(first.color, opacity).into(),
                percentage: first.offset.clamp(0.0, 1.0),
            };
            let to = LinearColorStop {
                color: to_rgba(last.color, opacity).into(),
                percentage: last.offset.clamp(0.0, 1.0),
            };
            linear_gradient(angle, from, to)
        }
    }
}

fn to_rgba(color: Color, opacity: f32) -> Rgba {
    Rgba {
        r: color.r as f32 / 255.0,
        g: color.g as f32 / 255.0,
        b: color.b as f32 / 255.0,
        a: color.a.clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0),
    }
}

/// Axis-aligned bounds of a clip path, used as a rectangular content mask.
fn clip_bounds(
    path: &[PathCommand],
    _fill_rule: FillRule,
    origin: Point<Pixels>,
    scale: f32,
) -> Option<Bounds<Pixels>> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut extend = |x: f32, y: f32| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    for command in path {
        match *command {
            PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => extend(x, y),
            PathCommand::QuadTo { x1, y1, x, y } => {
                extend(x1, y1);
                extend(x, y);
            }
            PathCommand::CubicTo { x1, y1, x2, y2, x, y } => {
                extend(x1, y1);
                extend(x2, y2);
                extend(x, y);
            }
            PathCommand::Close => {}
        }
    }
    if min_x > max_x || min_y > max_y {
        return None;
    }
    Some(Bounds {
        origin: to_point((min_x, min_y), origin, scale),
        size: Size {
            width: px((max_x - min_x) * scale),
            height: px((max_y - min_y) * scale),
        },
    })
}

/// A GPUI element that renders a prebuilt [`MermaidScene`], fit-centered
/// into its bounds, with an optional zoom/pan view transform.
pub struct MermaidDiagram {
    scene: MermaidScene,
    id: Option<ElementId>,
    zoom: f32,
    pan: Point<Pixels>,
    bounds_sink: Option<Rc<RefCell<Option<Bounds<Pixels>>>>>,
}

impl MermaidDiagram {
    pub fn new(scene: MermaidScene) -> Self {
        Self {
            scene,
            id: None,
            zoom: 1.0,
            pan: point(px(0.), px(0.)),
            bounds_sink: None,
        }
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Apply a view transform (`zoom` multiplies the fit scale, `pan` is a
    /// screen-pixel offset).
    pub fn transform(mut self, zoom: f32, pan: Point<Pixels>) -> Self {
        self.zoom = zoom;
        self.pan = pan;
        self
    }

    /// Report this element's painted bounds into `sink` each frame, so an
    /// owning view can run zoom/pan math against the live bounds.
    pub fn bounds_sink(mut self, sink: Rc<RefCell<Option<Bounds<Pixels>>>>) -> Self {
        self.bounds_sink = Some(sink);
        self
    }
}

impl IntoElement for MermaidDiagram {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MermaidDiagram {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = window.request_layout(
            Style {
                size: Size {
                    width: Length::Definite(DefiniteLength::Fraction(1.0)),
                    height: Length::Definite(DefiniteLength::Fraction(1.0)),
                },
                ..Default::default()
            },
            [],
            cx,
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if let Some(sink) = &self.bounds_sink {
            *sink.borrow_mut() = Some(bounds);
        }
        self.scene.paint_view(bounds, window, self.zoom, self.pan);
    }
}

/// Convenience: build a [`MermaidScene`], falling back to an empty scene on
/// error so views can stay infallible while showing parse failures elsewhere.
pub fn mermaid_scene_or_empty(source: &str, options: RenderOptions) -> MermaidScene {
    MermaidScene::build(source, options).unwrap_or_else(|err| {
        eprintln!("mermaid-gpui: failed to render diagram: {err:#}");
        MermaidScene {
            scene: Scene {
                width: 1.0,
                height: 1.0,
                commands: Vec::new(),
            },
        }
    })
}

/// Convenience helper mirroring `div()`-style construction:
/// `mermaid(scene)` where `scene` was built once via [`MermaidScene::build`].
pub fn mermaid(scene: MermaidScene) -> MermaidDiagram {
    MermaidDiagram::new(scene)
}

fn p(v: Pixels) -> f32 {
    v.into()
}

/// Zoom range enforced by [`MermaidViewer`].
pub const MIN_ZOOM: f32 = 0.2;
/// See [`MIN_ZOOM`].
pub const MAX_ZOOM: f32 = 8.0;

/// A stateful, interactive Mermaid viewer: fit-centered by default, with
/// trackpad scroll / pinch to zoom (about the cursor) and left-drag to pan.
///
/// Build once with [`MermaidViewer::new`] (the scene is pre-rendered, so this
/// is cheap) and place the returned [`Entity`] in your view tree.
pub struct MermaidViewer {
    scene: MermaidScene,
    zoom: f32,
    pan: Point<Pixels>,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    id: ElementId,
}

impl MermaidViewer {
    pub fn new(scene: MermaidScene) -> Self {
        Self {
            scene,
            zoom: 1.0,
            pan: point(px(0.), px(0.)),
            drag: None,
            bounds: Rc::new(RefCell::new(None)),
            id: ElementId::Name("mermaid-viewer".into()),
        }
    }

    /// Customize the element id (must be unique among siblings).
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    /// Reset to the fit-centered view.
    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan = point(px(0.), px(0.));
    }

    /// Zoom by `factor` keeping the world point under `cursor` fixed.
    pub fn zoom_at(&mut self, cursor: Point<Pixels>, factor: f32) {
        let Some(bounds) = *self.bounds.borrow() else {
            return;
        };
        let fit = self.scene.fit_scale(bounds);
        if fit <= 0.0 {
            return;
        }
        let (origin_old, s_old) = self.scene.view_origin(bounds, self.zoom, self.pan);
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let s_new = fit * new_zoom;
        if s_old <= 0.0 || s_new <= 0.0 {
            return;
        }
        // World point (scene units) currently under the cursor.
        let wx = (p(cursor.x) - p(origin_old.x)) / s_old;
        let wy = (p(cursor.y) - p(origin_old.y)) / s_old;
        // Desired origin so that world point lands back under the cursor:
        // cursor = origin + w * scale  =>  origin = cursor - w * scale.
        let want_x = p(cursor.x) - wx * s_new;
        let want_y = p(cursor.y) - wy * s_new;
        // Base (unpanned) origin at the new zoom.
        let (base_new, _) = self.scene.view_origin(bounds, new_zoom, point(px(0.), px(0.)));
        self.pan = point(px(want_x - p(base_new.x)), px(want_y - p(base_new.y)));
        self.zoom = new_zoom;
    }
}

impl Render for MermaidViewer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let diagram = mermaid(self.scene.clone())
            .transform(self.zoom, self.pan)
            .bounds_sink(self.bounds.clone());
        div()
            .id(self.id.clone())
            .size_full()
            .overflow_hidden()
            .cursor(if self.drag.is_some() {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            })
            .child(diagram)
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _window, cx| {
                let dy = match ev.delta {
                    ScrollDelta::Pixels(d) => p(d.y),
                    ScrollDelta::Lines(d) => d.y,
                };
                if dy != 0.0 {
                    this.zoom_at(ev.position, (-dy * 0.01).exp());
                    cx.notify();
                }
            }))
            .on_pinch(cx.listener(|this, ev: &PinchEvent, _window, cx| {
                if ev.delta != 0.0 {
                    this.zoom_at(ev.position, 1.0 + ev.delta);
                    cx.notify();
                }
            }))
            .on_mouse_down(
                gpui_kit::gpui::MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                    this.drag = Some((ev.position, this.pan));
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                if let Some((start, start_pan)) = this.drag {
                    this.pan = point(
                        px(p(start_pan.x) + p(ev.position.x) - p(start.x)),
                        px(p(start_pan.y) + p(ev.position.y) - p(start.y)),
                    );
                    cx.notify();
                }
            }))
            .on_mouse_up(
                gpui_kit::gpui::MouseButton::Left,
                cx.listener(|this, _ev: &MouseUpEvent, _window, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_contains_geometry() {
        let scene = MermaidScene::build(
            "flowchart TD; A[Start] --> B[Done]",
            RenderOptions::default(),
        )
        .unwrap();
        let fills = scene
            .scene
            .commands
            .iter()
            .filter(|c| matches!(c, SceneCommand::FillPath { .. }))
            .count();
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        for c in &scene.scene.commands {
            if let SceneCommand::FillPath { path, .. } = c {
                for cmd in path {
                    let (x, _) = match *cmd {
                        PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => (x, y),
                        PathCommand::QuadTo { x, .. } | PathCommand::CubicTo { x, .. } => (x, 0.),
                        PathCommand::Close => continue,
                    };
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                }
            }
        }
        println!(
            "canvas {}x{}, fills {fills}, x range {min_x}..{max_x}",
            scene.scene.width, scene.scene.height
        );
        assert!(fills > 0);
    }
}
#[cfg(test)]
mod matching_tests {
    use super::*;

    fn push_clip() -> SceneCommand {
        SceneCommand::PushClip { path: Vec::new(), fill_rule: FillRule::NonZero }
    }
    fn push_layer() -> SceneCommand {
        SceneCommand::PushLayer { opacity: 1.0, blend_mode: BlendMode::Normal }
    }

    #[test]
    fn matching_end_finds_closest_opener() {
        let cmds = vec![
            push_clip(),
            SceneCommand::FillPath {
                path: Vec::new(),
                paint: Paint::Solid(Color { r: 0, g: 0, b: 0, a: 1. }),
                fill_rule: FillRule::NonZero,
            },
            SceneCommand::PopClip,
            SceneCommand::FillPath {
                path: Vec::new(),
                paint: Paint::Solid(Color { r: 0, g: 0, b: 0, a: 1. }),
                fill_rule: FillRule::NonZero,
            },
            SceneCommand::PopClip,
        ];
        // Opener at 0 must match the PopClip at index 2, not 4.
        assert_eq!(
            matching_end(&cmds, |c| matches!(c, SceneCommand::PopClip)),
            2
        );
    }

    #[test]
    fn matching_end_handles_nested_mixed_types() {
        let cmds = vec![
            push_clip(),
            push_layer(),
            SceneCommand::PopLayer,
            SceneCommand::PopClip,
            SceneCommand::PopClip,
        ];
        assert_eq!(matching_end(&cmds, |c| matches!(c, SceneCommand::PopClip)), 3);
    }
}
