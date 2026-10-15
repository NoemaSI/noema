use gpui_kit::*;

use crate::smap::flatgraph::model;
use crate::smap::flatgraph::scheme::Scheme;

pub(crate) fn chip(label: &str, color: Hsla) -> Div {
    div()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(3.))
        .border_1()
        .border_color(color)
        .text_size(px(10.5))
        .text_color(color)
        .whitespace_nowrap()
        .child(label.to_string())
}

pub(crate) fn kind_chip(label: &str, s: &Scheme) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(s.border)
        .text_size(px(10.))
        .text_color(s.dim)
        .whitespace_nowrap()
        .child(label.to_uppercase())
}

pub(crate) fn status_chip(st: &str, s: &Scheme) -> Div {
    let (label, el): (&str, Div) = match st {
        "given" => ("GIVEN", div().border_1().border_color(s.dim).text_color(s.dim)),
        "observed" => ("OBSERVED", div().bg(s.dim).text_color(s.bg)),
        _ => ("⌀ UNKNOWN", div().border_1().border_color(s.dim).text_color(s.dim)),
    };
    el.px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .text_size(px(10.))
        .whitespace_nowrap()
        .child(label)
}

pub(crate) fn role_chip(st: &str, s: &Scheme) -> Div {
    let el = if st == "SP" {
        div().bg(s.dim).text_color(s.bg)
    } else {
        div().border_1().border_color(s.faint).text_color(s.dim)
    };
    el.px(px(5.))
        .rounded(px(3.))
        .text_size(px(9.5))
        .mr(px(6.))
        .whitespace_nowrap()
        .child(st.to_string())
}

pub(crate) fn ebar(fns: &[&model::Function], s: &Scheme) -> Div {
    let mut d = div().flex().gap(px(2.)).w(px(64.)).flex_shrink_0();
    for f in fns {
        let seg = match model::status_str(f.status) {
            "given" => div().bg(s.dim),
            "observed" => div().bg(s.dim.opacity(0.45)).border_1().border_color(s.dim),
            _ => div().border_1().border_color(s.dim),
        };
        d = d.child(seg.w(px(9.)).h(px(10.)).rounded(px(2.)));
    }
    d
}

pub(crate) fn chan_span(chan: &str, s: &Scheme) -> Div {
    let parts: Vec<&str> = chan.split('.').collect();
    div().flex().items_center().font_family("Menlo").text_size(px(12.)).children(vec![
        div().text_color(s.faint).child(parts[0].to_string()).into_any_element(),
        div()
            .text_color(s.mono)
            .child(if parts.len() > 1 { parts[1].to_string() } else { String::new() })
            .into_any_element(),
        div()
            .text_color(s.faint)
            .child(if parts.len() > 2 { format!(".{}", parts[2..].join(".")) } else { String::new() })
            .into_any_element(),
    ])
}

pub(crate) fn hex_lit(s: &str) -> Hsla {
    crate::smap::flatgraph::scheme::hex(s)
}