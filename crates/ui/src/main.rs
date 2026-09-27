mod not_logged_in;
mod route;
mod workspace;
mod world;
mod market;

use std::borrow::Cow;

use gpui_kit::component::*;
use gpui_kit::*;

use not_logged_in::{LoginClicked, NotLoggedIn};
use route::Route;
use workspace::Workspace;

static IBM_PLEX_SANS_SEMIBOLD: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf");
static IBM_PLEX_SANS_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
static IBM_PLEX_SANS_MEDIUM: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Medium.ttf");

static IOSKELEY_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/ioskeley/IoskeleyMono-Regular.ttf");
static IOSKELEY_MEDIUM: &[u8] =
    include_bytes!("../../../assets/fonts/ioskeley/IoskeleyMono-Medium.ttf");
static IOSKELEY_SEMIBOLD: &[u8] =
    include_bytes!("../../../assets/fonts/ioskeley/IoskeleyMono-SemiBold.ttf");

pub const FONT_FAMILY: &str = "Ioskeley Mono";

pub const TEXT_SM: Pixels = px(13.0);
pub const TEXT_BASE: Pixels = px(14.0);
pub const TEXT_LG: Pixels = px(16.0);

struct AppRoot {
    route: Route,
    not_logged_in: Entity<NotLoggedIn>,
    workspace: Entity<Workspace>,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let not_logged_in = cx.new(|cx| NotLoggedIn::new(window, cx));
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        cx.subscribe(&not_logged_in, |this, _, _: &LoginClicked, cx| {
            this.route = Route::Workspace;
            cx.notify();
        })
        .detach();
        Self {
            route: Route::default(),
            not_logged_in,
            workspace,
        }
    }
}

impl Render for AppRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match self.route {
            Route::NotLoggedIn => div().size_full().child(self.not_logged_in.clone()).into_any_element(),
            Route::Workspace => div().size_full().child(self.workspace.clone()).into_any_element(),
            Route::Settings => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .font_family(FONT_FAMILY)
                .child("Settings")
                .into_any_element(),
        }
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        // Load custom fonts
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX_SANS_SEMIBOLD)])
            .expect("failed to load IBM Plex Sans SemiBold font");

        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX_SANS_MEDIUM)])
            .expect("failed to load IBM Plex Sans Medium font");

        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX_SANS_REGULAR)])
            .expect("failed to load IBM Plex Sans Regular font");

        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(IOSKELEY_REGULAR)])
            .expect("failed to load Ioskeley Mono Regular font");

        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(IOSKELEY_MEDIUM)])
            .expect("failed to load Ioskeley Mono Medium font");

        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(IOSKELEY_SEMIBOLD)])
            .expect("failed to load Ioskeley Mono SemiBold font");

        // This must be called before using any GPUI Component features.
        gpui_kit::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| AppRoot::new(window, cx));
                // This first level on the window, should be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
