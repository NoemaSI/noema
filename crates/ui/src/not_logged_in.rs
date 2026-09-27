
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use client::StoredIdentity;
use gpui_kit::component::{button::*, *};
use gpui_kit::*;

use crate::element::button::*;
use crate::FONT_FAMILY;

/// Emitted when the user presses Login (until real auth exists).
#[derive(Clone)]
pub struct LoginClicked;

pub(crate) static NOEMA_LOGO: &[u8] = include_bytes!("../../../assets/noema_logo.png");
//const FONT_FAMILY: &str= "IBM Plex Sans";

pub struct NotLoggedIn {
    logo: Image,
    identity_path: PathBuf,
    identity: Option<StoredIdentity>,
}

impl EventEmitter<LoginClicked> for NotLoggedIn {}

impl NotLoggedIn {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let logo = Image::from_bytes(ImageFormat::Png, NOEMA_LOGO.to_vec());
        let identity_path = PathBuf::from(".noema").join("identity.json");
        let identity = StoredIdentity::load(&identity_path).ok();
        Self {
            logo,
            identity_path,
            identity,
        }
    }

    fn create_identity(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        match StoredIdentity::create(&self.identity_path) {
            Ok(stored) => {
                println!(
                    "created identity '{}' at {:?}",
                    stored.name, self.identity_path
                );
                self.identity = Some(stored);
                cx.notify();
            }
            Err(e) => eprintln!("failed to create identity: {e}"),
        }
    }

    fn destroy_identity(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        match fs::remove_file(&self.identity_path) {
            Ok(()) => {
                println!("destroyed identity at {:?}", self.identity_path);
                self.identity = None;
                cx.notify();
            }
            Err(e) => eprintln!("failed to destroy identity: {e}"),
        }
    }

    fn render_logo(&self) -> impl IntoElement {
        img(ImageSource::Image(Arc::new(self.logo.clone())))
            .size(px(158.))
            .border_0()
    }

    fn render_brand(&self) -> impl IntoElement {
        div()
            .v_flex()
            .items_center()
            .gap_y_4()
            .font_family(FONT_FAMILY)
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(16.0))
            .child(self.render_logo())
            .child(
                div()
                    .v_flex()
                    .items_center()
                    .child("Connecting minds and machines")
                    .child("for scientific discovery"),
            )
    }

    fn render_sign_up(&self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let heading = match &self.identity {
            Some(stored) => format!("Welcome back {}", stored.name),
            None => "Welcome".to_string(),
        };
        let card = div()
            .w(px(360.))
            .p_6()
            .v_flex()
            .gap_y_3()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().overlay)
            .rounded(px(4.))
            .child(
                div()
                    .mb_2()
                    .font_family(FONT_FAMILY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(20.0))
                    .child(heading),
            );
        let card = match &self.identity {
            Some(_) => card
                .child(pbutton(
                    "login",
                    "Login",
                    cx.listener(|_, _, _, cx| cx.emit(LoginClicked)),
                    cx,
                ))
                .child(sbutton(
                    "destroy-identity",
                    "Destroy identity",
                    cx.listener(Self::destroy_identity),
                    cx,
                )),
            None => card.child(pbutton(
                "create-identity",
                "Create identity",
                cx.listener(Self::create_identity),
                cx,
            )),
        };
        div()
            .v_flex()
            .items_center()
            .justify_center()
            .flex_1()
            .child(card)
    }
}

impl Render for NotLoggedIn {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .h_flex()
            .border_0()
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .child(self.render_brand()),
            )
            .child(self.render_sign_up(window, cx))
    }
}
