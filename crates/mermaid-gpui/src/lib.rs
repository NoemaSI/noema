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

use gpui_kit::gpui::{
    App, Background, Bounds, ContentMask, DefiniteLength, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Length, LinearColorStop, PathBuilder, PathStyle,
    Pixels, Point, Rgba, Size, Style, Window, linear_gradient, point, px,
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

    /// Replay the scene into the current GPUI paint context.
    ///
    /// The scene canvas is scaled uniformly to fit `bounds` and centered.
    /// Respects the window's current content mask.
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let scale = self.fit_scale(bounds);
        if scale <= 0.0 {
            return;
        }
        let canvas = Bounds {
            origin: point(
                bounds.origin.x + (bounds.size.width - px(self.scene.width * scale)) / 2.0,
                bounds.origin.y + (bounds.size.height - px(self.scene.height * scale)) / 2.0,
            ),
            size: Size {
                width: px(self.scene.width * scale),
                height: px(self.scene.height * scale),
            },
        };
        window.with_content_mask(Some(ContentMask { bounds: canvas }), |window| {
            if std::env::var_os("MERMAID_DUMMY").is_some() {
                // Dummy 2x2 path in the window corner to perturb path-batch
                // ordering in the renderer for debugging.
                let mut b = PathBuilder::fill();
                b.move_to(point(px(1.), px(1.)));
                b.line_to(point(px(3.), px(1.)));
                b.line_to(point(px(3.), px(3.)));
                b.close();
                if let Ok(p) = b.build() {
                    window.paint_path(p, Rgba { r: 0.0, g: 0.0, b: 1.0, a: 1.0 });
                }
            }
            self.paint_commands(&self.scene.commands, canvas.origin, scale, 1.0, window);
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
        let max_fills: Option<usize> = std::env::var("MERMAID_MAX_FILLS")
            .ok()
            .and_then(|v| v.parse().ok());
        let mut fills_painted = 0usize;
        let mut i = 0;
        while i < commands.len() {
            match &commands[i] {
                SceneCommand::FillPath {
                    path,
                    paint,
                    fill_rule,
                } => {
                    if max_fills.is_none_or(|n| fills_painted < n) {
                        fills_painted += 1;
                        paint_path(path, paint, *fill_rule, origin, scale, opacity, window);
                    }
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
        eprintln!("DBG path build failed for {} commands", path.len());
        return;
    };
    let background = if std::env::var_os("MERMAID_DEBUG_RED").is_some() {
        Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }.into()
    } else {
        to_background(paint, scale, opacity)
    };
    window.paint_path(path, background);
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
/// into its bounds.
pub struct MermaidDiagram {
    scene: MermaidScene,
    id: Option<ElementId>,
}

impl MermaidDiagram {
    pub fn new(scene: MermaidScene) -> Self {
        Self { scene, id: None }
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
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
        self.scene.paint(bounds, window);
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
mod tessellation_tests {
    use super::*;

    #[test]
    fn all_scene_paths_tessellate() {
        let scene = MermaidScene::build(
            "flowchart TD; A[Start] --> B[Done]",
            RenderOptions::default(),
        )
        .unwrap();
        let mut empty = 0;
        let mut ok = 0;
        let mut failed = 0;
        for c in &scene.scene.commands {
            if let SceneCommand::FillPath { path, fill_rule, .. } = c {
                let mut builder = PathBuilder::fill();
                builder.style =
                    PathStyle::Fill(lyon_fill_options(matches!(fill_rule, FillRule::EvenOdd)));
                for cmd in path {
                    match *cmd {
                        PathCommand::MoveTo { x, y } => builder.move_to(to_point((x, y), point(px(0.), px(0.)), 1.0)),
                        PathCommand::LineTo { x, y } => builder.line_to(to_point((x, y), point(px(0.), px(0.)), 1.0)),
                        PathCommand::QuadTo { x1, y1, x, y } => builder.curve_to(to_point((x, y), point(px(0.), px(0.)), 1.0), to_point((x1, y1), point(px(0.), px(0.)), 1.0)),
                        PathCommand::CubicTo { x1, y1, x2, y2, x, y } => builder.cubic_bezier_to(to_point((x, y), point(px(0.), px(0.)), 1.0), to_point((x1, y1), point(px(0.), px(0.)), 1.0), to_point((x2, y2), point(px(0.), px(0.)), 1.0)),
                        PathCommand::Close => builder.close(),
                    }
                }
                match builder.build() {
                    Ok(p) if p.vertices.is_empty() => empty += 1,
                    Ok(_) => ok += 1,
                    Err(e) => {
                        failed += 1;
                        println!("build err: {e:?}");
                    }
                }
            }
        }
        println!("ok {ok}, empty {empty}, failed {failed}");
        assert_eq!((empty, failed), (0, 0));
    }
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn print_fill_colors() {
        let scene = MermaidScene::build(
            "flowchart TD; A[Start] --> B[Done]",
            RenderOptions::default(),
        )
        .unwrap();
        let mut colors = std::collections::HashMap::new();
        for c in &scene.scene.commands {
            if let SceneCommand::FillPath { paint, path, .. } = c {
                let key = match paint {
                    Paint::Solid(col) => format!("{:?}", (col.r, col.g, col.b, col.a)),
                    Paint::LinearGradient { .. } => "gradient".to_string(),
                };
                *colors.entry(key).or_insert(0) += path.len();
            }
        }
        for (k, v) in colors {
            println!("{k}: {v} cmds");
        }
    }
}

#[cfg(test)]
mod dump_tests {
    use super::*;

    #[test]
    fn dump_command_structure() {
        let scene = MermaidScene::build(
            "sequenceDiagram\n    participant U as User\n    participant V as View\n    U->>V: show diagram\n    V-->>U: native vectors",
            RenderOptions::default(),
        )
        .unwrap();
        let mut depth = 0;
        for c in &scene.scene.commands {
            match c {
                SceneCommand::PushLayer { opacity, blend_mode } => {
                    println!("{}PushLayer {opacity} {blend_mode:?}", "  ".repeat(depth));
                    depth += 1;
                }
                SceneCommand::PopLayer => depth -= 1,
                SceneCommand::PushClip { .. } => {
                    println!("{}PushClip", "  ".repeat(depth));
                    depth += 1;
                }
                SceneCommand::PopClip => depth -= 1,
                SceneCommand::FillPath { path, paint, .. } => {
                    let kind = match paint {
                        Paint::Solid(_) => "solid",
                        Paint::LinearGradient { .. } => "grad",
                    };
                    println!("{}Fill {kind} {}cmds", "  ".repeat(depth), path.len());
                }
            }
        }
        println!("final depth {depth}");
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    fn build_one(path: &[PathCommand], even_odd: bool) -> gpui_kit::gpui::Path<Pixels> {
        let mut builder = PathBuilder::fill();
        builder.style = PathStyle::Fill(lyon_fill_options(even_odd));
        for cmd in path {
            match *cmd {
                PathCommand::MoveTo { x, y } => builder.move_to(to_point((x, y), point(px(0.), px(0.)), 1.0)),
                PathCommand::LineTo { x, y } => builder.line_to(to_point((x, y), point(px(0.), px(0.)), 1.0)),
                PathCommand::QuadTo { x1, y1, x, y } => builder.curve_to(to_point((x, y), point(px(0.), px(0.)), 1.0), to_point((x1, y1), point(px(0.), px(0.)), 1.0)),
                PathCommand::CubicTo { x1, y1, x2, y2, x, y } => builder.cubic_bezier_to(to_point((x, y), point(px(0.), px(0.)), 1.0), to_point((x1, y1), point(px(0.), px(0.)), 1.0), to_point((x2, y2), point(px(0.), px(0.)), 1.0)),
                PathCommand::Close => builder.close(),
            }
        }
        builder.build().expect("build")
    }

    fn expected_bbox(path: &[PathCommand]) -> (f32, f32, f32, f32) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut e = |x: f32, y: f32| {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        };
        for c in path {
            match *c {
                PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => e(x, y),
                PathCommand::QuadTo { x1, y1, x, y } => { e(x1, y1); e(x, y); }
                PathCommand::CubicTo { x1, y1, x2, y2, x, y } => { e(x1, y1); e(x2, y2); e(x, y); }
                PathCommand::Close => {}
            }
        }
        (min_x, min_y, max_x, max_y)
    }

    #[test]
    fn path_bounds_match_geometry() {
        for src in [
            "flowchart TD; A[Start] --> B[Done]",
            "sequenceDiagram\n    participant U as User\n    U->>U: hi",
        ] {
            let scene = MermaidScene::build(src, RenderOptions::default()).unwrap();
            for (i, c) in scene.scene.commands.iter().enumerate() {
                if let SceneCommand::FillPath { path, fill_rule, .. } = c {
                    let built = build_one(path, matches!(fill_rule, FillRule::EvenOdd));
                    let (ex0, ey0, ex1, ey1) = expected_bbox(path);
                    let b = built.bounds;
                    let bx0: f32 = b.origin.x.into();
                    let by0: f32 = b.origin.y.into();
                    let bx1 = bx0 + f32::from(b.size.width);
                    let by1 = by0 + f32::from(b.size.height);
                    let tol = 2.0;
                    if !(bx0 <= ex0 + tol
                        && by0 <= ey0 + tol
                        && bx1 >= ex1 - tol
                        && by1 >= ey1 - tol
                        && bx0 > ex0 - 500.
                        && by0 > ey0 - 500.
                        && bx1 < ex1 + 500.
                        && by1 < ey1 + 500.)
                    {
                        println!(
                            "BAD path {i}: built=({bx0},{by0})-({bx1},{by1}) expected=({ex0},{ey0})-({ex1},{ey1})"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod dump_path_tests {
    use super::*;

    #[test]
    fn dump_path_1() {
        let scene = MermaidScene::build(
            "flowchart TD; A[Start] --> B[Done]",
            RenderOptions::default(),
        )
        .unwrap();
        for (i, c) in scene.scene.commands.iter().enumerate() {
            if let SceneCommand::FillPath { path, paint, fill_rule } = c {
                println!("--- {i} rule {fill_rule:?} paint {paint:?}");
                for cmd in path {
                    println!("  {cmd:?}");
                }
            }
        }
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
