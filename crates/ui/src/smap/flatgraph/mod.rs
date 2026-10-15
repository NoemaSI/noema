pub mod model;
pub mod scheme;
pub mod view;

use std::path::PathBuf;

use gpui_kit::*;
use view::FlatGraph;

fn _main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tablet_line.json"));
    application()
        .with_assets(assets::Assets)
        .run(|cx| {
            init(cx);
            open_window(WindowOptions::default(), cx, move |window, cx| {
                let map = smap::read_system_map(path.clone()).ok();
                cx.new(|cx| FlatGraph::new(map, window, cx))
            })
            .expect("Failed to open window");
        });
}
