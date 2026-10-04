use gpui_kit::base::Disableable;
use gpui_kit::component::button::*;
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use crate::TEXT_SM;

/// Primary button: translucent cyan fill, dim-cyan outline, cyan text, ▸ suffix.
pub fn pbutton(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Button {
    pbutton_auto(id, label, on_click, cx).w_full().h_10()
}

/// Primary button sized to its content (for toolbars / button rows).
pub fn pbutton_auto(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Button {
    let theme = cx.theme();
    Button::new(id)
        .label(format!("{label} \u{25b8}"))
        .px_4()
        .py_2()
        .rounded(px(4.))
        .text_size(TEXT_SM)
        .bg(theme.primary)
        .text_color(theme.primary_foreground)
        .border_1()
        .border_color(theme.ring)
        .on_click(on_click)
}


/// Secondary button: panel2 fill, grey outline, dark text.
pub fn sbutton(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Button {
    sbutton_auto(id, label, on_click, cx).w_full().h_10()
}

/// Secondary button sized to its content (for toolbars / button rows).
pub fn sbutton_auto(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Button {
    let theme = cx.theme();
    Button::new(id)
        .label(label)
        .px_4()
        .py_2()
        .rounded(px(4.))
        .text_size(TEXT_SM)
        .bg(theme.overlay)
        .text_color(theme.foreground)
        .border_1()
        .border_color(theme.border)
        .on_click(on_click)
}
