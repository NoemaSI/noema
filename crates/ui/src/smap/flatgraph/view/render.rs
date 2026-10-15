use gpui_kit::component::input::Input;
use gpui_kit::*;

use smap::FunctionStatus;

use crate::smap::flatgraph::model::{self, Dir};
use crate::smap::flatgraph::scheme::Scheme;

use super::chips::{chan_span, chip, ebar, hex_lit, kind_chip, role_chip, status_chip};
use super::geometry::paint_prim;
use super::rows::{Row, RowKind};
use super::{FlatGraph, RIB_W};

impl FlatGraph {
    pub(crate) fn render_row(&self, r: &Row, s: &Scheme, cx: &mut Context<Self>) -> AnyElement {
        let wrap = |inner: Div, bg: Option<Hsla>| {
            let mut d = inner
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(RIB_W))
                .pr(px(10.))
                .h(px(r.h))
                .w_full()
                .flex_shrink_0()
                .overflow_hidden();
            if let Some(b) = bg {
                d = d.bg(b);
            }
            if r.dim {
                d = d.opacity(0.2);
            }
            d
        };
        match r.kind {
            RowKind::Band => wrap(
                div().child(
                    div()
                        .text_size(px(11.))
                        .text_color(s.faint)
                        .child(r.label.to_string()),
                ),
                None,
            )
            .into_any_element(),
            RowKind::District => self.render_district(r, s, &wrap, cx),
            RowKind::Station => self.render_station(r, s, &wrap, cx),
            RowKind::Platform => self.render_platform(r, s, &wrap, cx),
            RowKind::Train => {
                let mut line = div().flex().items_center().gap(px(6.)).pl(px(40.)).child(
                    div().text_color(s.text).child(r.label.to_string()),
                );
                for (f, _) in &r.chips {
                    line = line.child(chip(f, s.flow(f)));
                }
                wrap(div().w_full().child(div().flex_1().min_w_0().child(line)), None)
                    .text_color(s.dim)
                    .into_any_element()
            }
            RowKind::Exit => {
                let line = div().flex().items_center().gap(px(6.)).pl(px(40.))
                    .child(role_chip(&r.status, s))
                    .child(div().child(r.label.to_string()))
                    .child(div().ml(px(6.)).child(chan_span(&r.sub, s)));
                wrap(div().w_full().child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(line)), None)
                    .text_color(s.dim)
                    .into_any_element()
            }
            RowKind::StateGroup => {
                let id = r.id.clone();
                let disc = self.disc(&id, r.open, s, cx);
                let line = div().flex().items_center().gap(px(6.)).pl(px(18.))
                    .child(div().child(r.label.to_string()))
                    .child(div().text_size(px(12.)).text_color(s.faint).child(r.sub.to_string()));
                wrap(div().w_full().child(disc).child(div().flex_1().min_w_0().child(line)), None)
                    .text_color(s.dim)
                    .into_any_element()
            }
            RowKind::Fn => self.render_fn(r, s, &wrap, cx),
            RowKind::FnDetail => {
                let mut d = div().w_full().pl(px(40.)).pr(px(16.));
                let inner = div()
                    .p(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(s.faint)
                    .text_size(px(12.5))
                    .text_color(s.dim);
                let mut inner = if !r.sub.is_empty() {
                    inner.child(div().child(r.sub.to_string()))
                } else {
                    inner
                };
                if !r.sub2.is_empty() {
                    inner = inner.child(
                        div()
                            .mt(px(6.))
                            .p(px(6.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(s.faint)
                            .text_color(s.faint)
                            .child(r.sub2.to_string()),
                    );
                }
                d = d.child(inner);
                wrap(d, None).into_any_element()
            }
        }
    }

    /// shared disclosure +/- disc
    fn disc(&self, id: &SharedString, open: bool, s: &Scheme, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(ElementId::Name(format!("open|{}", id).into()))
            .w(px(18.))
            .flex_shrink_0()
            .text_size(px(14.))
            .text_color(s.faint)
            .cursor_pointer()
            .child(if open { "−" } else { "+" })
            .on_click(cx.listener({
                let id = id.clone();
                move |me, _, _, _| {
                    me.toggle(&id);
                }
            }))
    }

    fn render_district(
        &self,
        r: &Row,
        s: &Scheme,
        wrap: &dyn Fn(Div, Option<Hsla>) -> Div,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let fns = self.model.subtree(&r.label).into_iter().flat_map(|n| self.model.fn_alloc(&n)).collect::<Vec<_>>();
        let ticks = self.ticks(&r.label, s);
        wrap(
            div()
                .w_full()
                .child(div().w(px(18.)).flex_shrink_0())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(
                            div().flex().items_center().gap(px(8.)).child(
                                div().font_weight(FontWeight(600.0)).child(r.label.to_string()),
                            ).child(kind_chip("subsystem", s)).child(
                                div().text_size(px(12.)).text_color(s.faint).child(r.sub.to_string()),
                            ),
                        ),
                )
                .child(ebar(&fns, s))
                .child(ticks),
            None,
        )
        .into_any_element()
    }

    fn render_station(
        &self,
        r: &Row,
        s: &Scheme,
        wrap: &dyn Fn(Div, Option<Hsla>) -> Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = r.id.clone();
        let fns = self.model.fn_alloc(&r.label);
        let disc = self.disc(&id, r.open, s, cx);
        let mut name = div().flex().items_center().gap(px(8.)).child(
            div().text_size(px(15.)).font_weight(FontWeight(600.0)).child(r.label.to_string()),
        );
        for (c, role) in &r.chips {
            if *role == 1 {
                name = name.child(
                    div()
                        .px(px(5.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(s.faint)
                        .text_size(px(11.))
                        .text_color(s.amb)
                        .child(c.to_string()),
                );
            }
        }
        name = name.child(kind_chip(&r.sub, s));
        if !r.sub2.is_empty() {
            name = name.child(
                div().text_size(px(12.)).text_color(s.faint).truncate().child(r.sub2.to_string()),
            );
        }
        let ticks = self.ticks(&r.label, s);
        let meta = format!(
            "p{} · s{} · ƒ{}",
            self.model.el(&r.label).map(|e| e.ports.len()).unwrap_or(0),
            self.model.el(&r.label).map(|e| self.model.count_signals(e)).unwrap_or(0),
            fns.len()
        );
        wrap(
            div()
                .w_full()
                .child(disc)
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                .child(ebar(&fns, s))
                .child(ticks)
                .child(
                    div()
                        .w(px(110.))
                        .flex_shrink_0()
                        .text_right()
                        .font_family("Menlo")
                        .text_size(px(11.5))
                        .text_color(s.faint)
                        .child(meta),
                ),
            None,
        )
        .into_any_element()
    }

    fn render_platform(
        &self,
        r: &Row,
        s: &Scheme,
        wrap: &dyn Fn(Div, Option<Hsla>) -> Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = r.id.clone();
        let mut line = div().flex().items_center().gap(px(6.)).pl(px(18.)).child(
            div().child(r.label.to_string()),
        );
        line = line.child(div().text_color(s.faint).child(format!("· {}", r.sub)));
        for (f, _) in &r.chips {
            line = line.child(chip(f, s.flow(f)));
        }
        line = line.child(div().text_size(px(12.)).text_color(s.faint).child(r.sub2.to_string()));
        let disc = self.disc(&id, r.open, s, cx);
        wrap(div().w_full().child(disc).child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(line)), None)
            .text_color(s.dim)
            .into_any_element()
    }

    fn render_fn(
        &self,
        r: &Row,
        s: &Scheme,
        wrap: &dyn Fn(Div, Option<Hsla>) -> Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = r.id.clone();
        let disc = self.disc(&id, r.open, s, cx);
        let mut line = div().flex().items_center().gap(px(6.))
            .child(
                div().flex().items_center().child(
                    div().text_color(s.faint).child("ƒ ").into_any_element(),
                ).child(div().font_weight(FontWeight(600.0)).text_color(s.text).child(r.label.to_string())),
            )
            .child(status_chip(&r.status, s));
        if !r.sub.is_empty() {
            line = line.child(div().text_color(s.faint).child(r.sub.to_string()));
        }
        for (c, _) in &r.chips {
            let cs = c.to_string();
            if cs == "⇄" || cs == "∅" {
                line = line.child(div().text_color(s.faint).child(cs));
            } else if cs.starts_with("⇢") || cs.starts_with("⟲") {
                line = line.child(div().text_color(s.faint).child(cs));
            } else if self.model.map.flows.iter().any(|f| f.name == cs) {
                line = line.child(chip(&cs, s.flow(&cs)));
            } else {
                line = line.child(div().text_color(s.dim).child(cs));
            }
        }
        wrap(div().w_full().child(disc).child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(line)), None)
            .into_any_element()
    }

    fn ticks(&self, el: &str, s: &Scheme) -> Div {
        let flows = self.model.flows_at(el);
        let mut d = div().flex().gap(px(5.)).w(px(90.)).flex_shrink_0().items_center();
        for f in &self.model.map.flows {
            let dir = flows.iter().find(|(n, _)| n == &f.name).map(|(_, d)| *d);
            let col = s.flow(&f.name);
            let dot = match dir {
                Some(Dir::In) => div().border_2().border_color(col).bg(s.bg),
                Some(Dir::Out) => div().bg(col),
                Some(Dir::Both) => div().bg(col).border_2().border_color(s.bg),
                None => div().border_1().border_color(if s.light { hex_lit("#cbd5e1") } else { hex_lit("#20262e") }),
            };
            d = d.child(dot.w(px(10.)).h(px(10.)).rounded_full().flex_shrink_0());
        }
        d
    }
}

impl Render for FlatGraph {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Scheme::new(self.light);
        let rows = self.build_rows();
        let (prims, total) = self.build_geometry(&rows, &s);
        let body_rows: Vec<AnyElement> = rows.iter().map(|r| self.render_row(r, &s, cx)).collect();

        div()
            .size_full()
            .bg(s.bg)
            .text_color(s.text)
            .flex()
            .flex_col()
            .child(self.render_header(&s, cx))
            .children(self.req_strip(&s))
            .child(
                div()
                    .id("scroller")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .child(
                                canvas(
                                    move |_, _, _| {},
                                    move |bounds, _, window, _| {
                                        let ox: f32 = bounds.origin.x.into();
                                        let oy: f32 = bounds.origin.y.into();
                                        for p in &prims {
                                            paint_prim(p, ox, oy, window);
                                        }
                                    },
                                )
                                .absolute()
                                .left_0()
                                .top_0()
                                .w(px(RIB_W))
                                .h(px(total)),
                            )
                            .child(div().flex().flex_col().w_full().children(body_rows)),
                    ),
            )
            .into_any_element()
    }
}

impl FlatGraph {
    fn render_header(&self, s: &Scheme, cx: &mut Context<Self>) -> Div {
        let legend: Vec<AnyElement> = self
            .model
            .map
            .flows
            .iter()
            .map(|f| {
                let col = s.flow(&f.name);
                let hot = self
                    .model
                    .map
                    .functions
                    .iter()
                    .filter(|fn_| fn_.status == FunctionStatus::Unknown && (fn_.reads.iter().any(|r| r == &f.name) || fn_.writes.iter().any(|w| w == &f.name)))
                    .count();
                let on = self.focus.as_ref().map_or(false, |x| x == &f.name);
                let name = f.name.clone();
                div()
                    .id(ElementId::Name(format!("leg|{}", name).into()))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(10.))
                    .py(px(3.))
                    .rounded_sm()
                    .border_1()
                    .border_color(if on { col } else { s.border })
                    .bg(s.surface)
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(if on { s.text } else { s.dim })
                    .child(div().w(px(22.)).h(px(3.)).rounded_full().bg(col))
                    .child(name.clone())
                    // .children(if hot > 0 {
                    //     Some(div().text_size(px(11.)).text_color(s.s1).child(format!("{}", hot)).into_any_element())
                    // } else {
                    //     None
                    // })
                    .on_click(cx.listener(move |me, _, _, _| {
                        me.focus = match &me.focus {
                            Some(f) if f == &name => None,
                            _ => Some(name.clone().into()),
                        };
                    }))
                    .into_any_element()
            })
            .collect();

        let lens_label = if self.behavior { "behavior" } else { "topology" };
        let theme_label = if self.light { "light" } else { "dark" };

        div()
            .px_0()
            .pt_0()
            .pb_2()
            .border_b_1()
            .border_color(s.border)
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div().flex().items_center().gap(px(10.)).child(
                            div().text_size(px(18.)).font_weight(FontWeight(700.0)).child(self.model.map.name.clone()),
                        ).child(div().text_size(px(13.)).text_color(s.dim).child("system map")),
                    )
                    .child(div().flex_1())
                    .child(
                        div().w(px(240.)).child(Input::new(&self.input)),
                    )
                    .child(
                        div()
                            .id(ElementId::Name("lens".into()))
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(s.border)
                            .bg(s.surface)
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(s.dim)
                            .child(format!("lens: {}", lens_label))
                            .on_click(cx.listener(|me, _, _, _| {
                                me.behavior = !me.behavior;
                            })),
                    )
                    // theme switch
                    // .child(
                    //     div()
                    //         .id(ElementId::Name("theme".into()))
                    //         .px(px(10.))
                    //         .py(px(4.))
                    //         .rounded(px(6.))
                    //         .border_1()
                    //         .border_color(s.border)
                    //         .bg(s.surface)
                    //         .cursor_pointer()
                    //         .text_size(px(12.))
                    //         .text_color(s.dim)
                    //         .child(format!("theme: {}", theme_label))
                    //         .on_click(cx.listener(|me, _, _, _| {
                    //             me.light = !me.light;
                    //         })),
                    // ),
            )
            .child(div().flex().items_center().gap(px(8.)).flex_wrap().children(legend))
            .child(
                div().text_size(px(11.)).text_color(s.faint).child(
                    "hollow node = unknown mechanism · ring dot = stream in · filled dot = stream out · ⧉ = identical design · ∴ = derived, not in file · click a line to trace it",
                ),
            )
    }

    fn req_strip(&self, s: &Scheme) -> Option<Div> {
        let r = self.model.map.requirements.first().cloned();
        let r = r?;
        let mut d = div()
            .mx_0()
            .my(px(8.))
            .p(px(8.))
            .rounded(px(6.))
            .border_1()
            .border_color(s.border)
            .bg(s.surface)
            .text_size(px(12.5))
            .text_color(s.dim)
            .flex()
            .items_center()
            .gap(px(8.))
            .overflow_hidden()
            .child(div().font_weight(FontWeight(700.0)).text_color(s.text).child(r.id));
        // d = d.child(format!("\"{}\"", r.text));
        for n in &r.satisfies {
            let st = self.model.fn_by_name(n).map(|f| model::status_str(f.status).to_string()).unwrap_or_default();
            d = d.child(chip(n, s.dim)).child(status_chip(&st, s));
        }
        let unk = r.satisfies.iter().filter(|n| self.model.fn_by_name(n).map(|f| f.status == FunctionStatus::Unknown).unwrap_or(false)).count();
        Some(d.child(div().text_color(s.faint).child(format!("currently {} unknown mechanisms", unk))))
    }
}
