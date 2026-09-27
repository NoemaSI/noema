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
    let theme = cx.theme();
    Button::new(id)
        .label(format!("{label} \u{25b8}"))
        .w_full()
        .h_10()
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

/// Secondary button: white fill, grey outline, dark text.
pub fn sbutton(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Button {
    let theme = cx.theme();
    Button::new(id)
        .label(label)
        .w_full()
        .h_10()
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
