use std::path::PathBuf;

use gpui_kit::gpui::{InteractiveElement, StatefulInteractiveElement};
use gpui_kit::{
    App, Context, ExternalPaths, IntoElement, ParentElement, Render, Styled, Window, div, rgb,
};

pub struct FileDropView {
    pub dropped_paths: Vec<PathBuf>,
    pub drag_hovering: bool,
    pub on_drop: Option<Box<dyn Fn(Vec<PathBuf>, &mut Window, &mut App)>>,
}

impl FileDropView {
    pub fn new() -> Self {
        FileDropView {
            dropped_paths: vec![],
            drag_hovering: false,
            on_drop: None,
        }
    }

    pub fn on_drop(
        mut self,
        handler: impl Fn(Vec<PathBuf>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drop = Some(Box::new(handler));
        self
    }
}

impl Render for FileDropView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dropped = self.dropped_paths.clone();
        let content = if dropped.is_empty() {
            div().child("Drop files here")
        } else {
            div().child(format!("{} files dropped", dropped.len())).children(
                dropped
                    .iter()
                    .map(|path| div().child(path.display().to_string())),
            )
        };

        div()
            .id("file-drop")
            .size_full()
            .bg(if self.drag_hovering { rgb(0x4488ff) } else { rgb(0xeeeeee) })
            .child(content)
            .on_hover(cx.listener(|this, hovering, _window, cx| {
                this.drag_hovering = *hovering;
                cx.notify();
            }))
            .on_file_drop_exit(cx.listener(|this, _event, _window, cx| {
                this.drag_hovering = false;
                cx.notify();
            }))
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.drag_hovering = false;
                for path in paths.paths() {
                    if !this.dropped_paths.contains(path) {
                        this.dropped_paths.push(path.clone());
                    }
                }
                let dropped = this.dropped_paths.clone();
                if let Some(on_drop) = this.on_drop.as_ref() {
                    on_drop(dropped, window, cx);
                }
                cx.notify();
            }))
    }
}