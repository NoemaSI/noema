use std::collections::HashMap;

use gpui_kit::{point, px, Hsla, PathBuilder, Pixels, Point};

use smap::Direction;

use crate::smap::flatgraph::model::el_port;
use crate::smap::flatgraph::scheme::{mix, Scheme};

use super::rows::{Row, RowKind};
use super::{FlatGraph, INTERIOR};

pub(crate) fn lane_x(i: usize) -> f32 {
    14.0 + i as f32 * 24.0
}

pub(crate) fn ngon(cx: f32, cy: f32, r: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|i| {
            let a = std::f32::consts::PI * 2.0 * i as f32 / n as f32 - std::f32::consts::FRAC_PI_2;
            [cx + a.cos() * r, cy + a.sin() * r]
        })
        .collect()
}

fn cubic(p0: [f32; 2], c1: [f32; 2], c2: [f32; 2], p3: [f32; 2], n: usize) -> Vec<[f32; 2]> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let b0 = u * u * u;
            let b1 = 3.0 * u * u * t;
            let b2 = 3.0 * u * t * t;
            let b3 = t * t * t;
            [
                b0 * p0[0] + b1 * c1[0] + b2 * c2[0] + b3 * p3[0],
                b0 * p0[1] + b1 * c1[1] + b2 * c2[1] + b3 * p3[1],
            ]
        })
        .collect()
}

#[derive(Clone)]
pub(crate) enum Prim {
    Line {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        w: f32,
        color: Hsla,
        dash: Option<[f32; 2]>,
    },
    Curve {
        pts: Vec<[f32; 2]>,
        w: f32,
        color: Hsla,
    },
    Disc {
        cx: f32,
        cy: f32,
        r: f32,
        color: Hsla,
    },
    Ring {
        cx: f32,
        cy: f32,
        r: f32,
        w: f32,
        color: Hsla,
        dash: bool,
    },
    Tri {
        pts: Vec<[f32; 2]>,
        color: Hsla,
    },
}

/// paints one prim into the ribbon canvas
pub(crate) fn paint_prim(p: &Prim, ox: f32, oy: f32, window: &mut gpui_kit::Window) {
    match p {
        Prim::Line { x0, y0, x1, y1, w, color, dash } => {
            let mut b = PathBuilder::stroke(px(*w));
            if let Some(d) = dash {
                b = b.dash_array(&[px(d[0]), px(d[1])]);
            }
            b.move_to(point(px(x0 + ox), px(y0 + oy)));
            b.line_to(point(px(x1 + ox), px(y1 + oy)));
            if let Ok(path) = b.build() {
                window.paint_path(path, *color);
            }
        }
        Prim::Curve { pts, w, color } => {
            let mut b = PathBuilder::stroke(px(*w));
            if let Some(first) = pts.first() {
                b.move_to(point(px(first[0] + ox), px(first[1] + oy)));
                for pt in &pts[1..] {
                    b.line_to(point(px(pt[0] + ox), px(pt[1] + oy)));
                }
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, *color);
            }
        }
        Prim::Disc { cx, cy, r, color } => {
            let mut b = PathBuilder::fill();
            let pts: Vec<Point<Pixels>> = ngon(*cx, *cy, *r, 20)
                .into_iter()
                .map(|p| point(px(p[0] + ox), px(p[1] + oy)))
                .collect();
            b.add_polygon(&pts, true);
            if let Ok(path) = b.build() {
                window.paint_path(path, *color);
            }
        }
        Prim::Ring { cx, cy, r, w, color, dash } => {
            let mut b = PathBuilder::stroke(px(*w));
            if *dash {
                b = b.dash_array(&[px(2.5), px(2.5)]);
            }
            let pts: Vec<Point<Pixels>> = ngon(*cx, *cy, *r, 24)
                .into_iter()
                .map(|p| point(px(p[0] + ox), px(p[1] + oy)))
                .collect();
            b.add_polygon(&pts, true);
            if let Ok(path) = b.build() {
                window.paint_path(path, *color);
            }
        }
        Prim::Tri { pts, color } => {
            let mut b = PathBuilder::fill();
            let pp: Vec<Point<Pixels>> = pts
                .iter()
                .map(|p| point(px(p[0] + ox), px(p[1] + oy)))
                .collect();
            b.add_polygon(&pp, true);
            if let Ok(path) = b.build() {
                window.paint_path(path, *color);
            }
        }
    }
}

impl FlatGraph {
    pub(crate) fn build_geometry(&self, rows: &[Row], s: &Scheme) -> (Vec<Prim>, f32) {
        let mut prims: Vec<Prim> = Vec::new();
        let mut ys: HashMap<String, f32> = HashMap::new();
        let mut port_ys: HashMap<String, f32> = HashMap::new();
        let mut y = 0.0f32;
        for r in rows {
            if let Some(el) = &r.el {
                ys.entry(el.to_string()).or_insert(y + r.h / 2.0);
            }
            if let Some(p) = &r.port {
                port_ys.insert(p.to_string(), y + r.h / 2.0);
            }
            y += r.h;
        }
        let total = y;
        let ink = |a: f32| {
            let mut c = s.ink;
            c.a = a;
            c
        };

        let resolve = |ref_: &str| -> Option<f32> {
            let (el, _) = el_port(ref_);
            if let Some(py) = port_ys.get(ref_) {
                return Some(*py);
            }
            ys.get(&el).copied()
        };

        self.lane_prims(&mut prims, resolve, &ink, s);
        self.train_prims(&mut prims, rows, resolve, s);
        self.interior_prims(&mut prims, rows, &ys, &port_ys, &ink, s);
        (prims, total)
    }

    /// flow lanes (vertical lines) + station nodes on each lane
    fn lane_prims(
        &self,
        prims: &mut Vec<Prim>,
        resolve: impl Fn(&str) -> Option<f32>,
        ink: &dyn Fn(f32) -> Hsla,
        s: &Scheme,
    ) {
        let m = &self.model;
        let _ = ink;
        for f in &m.map.flows {
            let lx = lane_x(m.lane(&f.name));
            let col = s.flow(&f.name);
            let op = self.flow_op(&f.name);
            let mut ns: Vec<f32> = Vec::new();
            for ed in &f.edges {
                for ref_ in [&ed.from, &ed.to] {
                    if let Some(yy) = resolve(ref_) {
                        if !ns.iter().any(|v| (v - yy).abs() < 0.5) {
                            ns.push(yy);
                        }
                    }
                }
            }
            ns.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if ns.len() > 1 {
                prims.push(Prim::Line {
                    x0: lx,
                    y0: ns[0],
                    x1: lx,
                    y1: *ns.last().unwrap(),
                    w: m.flow_width(&f.name),
                    color: col.opacity(op),
                    dash: m.flow_dash(&f.name),
                });
            }
            for ny in ns {
                let mut owner = String::new();
                for ed in &f.edges {
                    for ref_ in [&ed.from, &ed.to] {
                        if resolve(ref_) == Some(ny) {
                            owner = el_port(ref_).0;
                        }
                    }
                }
                let unk = m.unknown_fn_touching(&owner, &f.name);
                if unk {
                    prims.push(Prim::Disc { cx: lx, cy: ny, r: 4.5, color: s.bg.opacity(op) });
                    prims.push(Prim::Ring { cx: lx, cy: ny, r: 4.5, w: 1.5, color: col.opacity(op), dash: true });
                } else {
                    prims.push(Prim::Disc { cx: lx, cy: ny, r: 4.5, color: col.opacity(op) });
                }
            }
        }
    }

    /// train threads: curved connectors from a train row to its far port
    fn train_prims(
        &self,
        prims: &mut Vec<Prim>,
        rows: &[Row],
        resolve: impl Fn(&str) -> Option<f32>,
        s: &Scheme,
    ) {
        let m = &self.model;
        let total: f32 = rows.iter().map(|r| r.h).sum();
        let mut seen: HashMap<(String, String), i32> = HashMap::new();
        for r in rows {
            let Some(t) = &r.train else { continue };
            let (Some(yf), Some(yl)) = (resolve(&t.far_port), resolve(&t.local_port)) else { continue };
            let key = (t.far_port.to_string(), t.flow.to_string());
            let idx = seen.get(&key).copied().unwrap_or(0);
            seen.insert(key, idx + 1);
            let lx = lane_x(m.lane(&t.flow));
            let col = s.flow(&t.flow);
            let op = self.flow_op(&t.flow);
            let tx = lx + 11.0 + idx as f32 * 6.0;
            let up = yf < yl;
            let bend = if up { -14.0 } else { 14.0 };
            let mut pts = cubic([lx, yl], [tx, yl], [tx, yl + bend], [tx, yl + bend], 12);
            pts.extend(cubic([tx, yl + bend], [tx, yl + bend], [tx, yf - bend], [tx, yf - bend], 4));
            pts.extend(cubic([tx, yf - bend], [tx, yf], [lx, yf], [lx, yf], 12));
            prims.push(Prim::Curve { pts, w: 2.0, color: col.opacity(0.95 * op) });
            // dot at the train row itself
            let train_y = {
                let mut acc = 0.0;
                let mut found = total;
                for x in rows {
                    if x.id == r.id {
                        found = acc + x.h / 2.0;
                        break;
                    }
                    acc += x.h;
                }
                found
            };
            prims.push(Prim::Disc { cx: tx, cy: train_y, r: 3.0, color: col.opacity(op) });
            let ay = if t.dir_to { yf } else { yl };
            prims.push(Prim::Tri { pts: vec![[lx + 4.0, ay], [lx + 11.0, ay - 4.0], [lx + 11.0, ay + 4.0]], color: col.opacity(op) });
            prims.push(Prim::Ring { cx: lx, cy: ay, r: 7.0, w: 1.0, color: col.opacity(0.45 * op), dash: false });
        }
    }

    /// expanded machine interiors: bracket spine, port stubs, function-routed curves
    fn interior_prims(
        &self,
        prims: &mut Vec<Prim>,
        rows: &[Row],
        ys: &HashMap<String, f32>,
        port_ys: &HashMap<String, f32>,
        ink: &dyn Fn(f32) -> Hsla,
        s: &Scheme,
    ) {
        let m = &self.model;
        for r in rows {
            let Some(el) = &r.el else { continue };
            if r.kind != RowKind::Station {
                continue;
            }
            let Some(e) = m.el(el) else { continue };
            let ey = ys.get(el.as_str()).copied().unwrap_or(0.0);
            if !r.open {
                let lanes: Vec<usize> = m.flows_at(el).into_iter().map(|(f, _)| m.lane(&f)).collect();
                if lanes.len() > 1 {
                    prims.push(Prim::Line {
                        x0: lane_x(*lanes.iter().min().unwrap()),
                        y0: ey,
                        x1: lane_x(*lanes.iter().max().unwrap()),
                        y1: ey,
                        w: 2.0,
                        color: ink(0.13),
                        dash: None,
                    });
                }
                continue;
            }
            let pys: Vec<f32> = e.ports.iter().filter_map(|p| port_ys.get(&format!("{}.{}", el, p.name))).copied().collect();
            if pys.is_empty() {
                continue;
            }
            prims.push(Prim::Line {
                x0: INTERIOR,
                y0: pys.iter().cloned().fold(f32::MAX, f32::min),
                x1: INTERIOR,
                y1: pys.iter().cloned().fold(f32::MIN, f32::max),
                w: 2.0,
                color: ink(0.22),
                dash: None,
            });
            for p in &e.ports {
                let ref_ = format!("{}.{}", el, p.name);
                let Some(py) = port_ys.get(&ref_).copied() else { continue };
                for (flow, _, _) in m.edges_at(&ref_) {
                    let op = self.flow_op(&flow);
                    prims.push(Prim::Line {
                        x0: lane_x(m.lane(&flow)),
                        y0: py,
                        x1: INTERIOR,
                        y1: py,
                        w: 1.5,
                        color: s.flow(&flow).opacity(0.8 * op),
                        dash: None,
                    });
                }
            }
            let ins: Vec<(&str, f32, String)> = e
                .ports
                .iter()
                .filter(|p| p.direction == Direction::In)
                .filter_map(|p| {
                    let ref_ = format!("{}.{}", el, p.name);
                    port_ys.get(&ref_).map(|y| (p.name.as_str(), *y, ref_))
                })
                .collect();
            let outs: Vec<(&str, f32, String)> = e
                .ports
                .iter()
                .filter(|p| p.direction == Direction::Out)
                .filter_map(|p| {
                    let ref_ = format!("{}.{}", el, p.name);
                    port_ys.get(&ref_).map(|y| (p.name.as_str(), *y, ref_))
                })
                .collect();
            for (_, yi, iref) in &ins {
                let fin = m.edges_at(iref).into_iter().find(|(_, _far, dir)| !*dir).map(|(f, _, _)| f);
                let routed = outs.iter().any(|(_, _, oref)| {
                    let fout = m.edges_at(oref).into_iter().find(|(_, _, dir)| *dir).map(|(f, _, _)| f);
                    match (&fin, &fout) {
                        (Some(a), Some(b)) => m.routes(a, b),
                        _ => false,
                    }
                });
                if !routed {
                    let c = fin.as_ref().map(|f| s.flow(f)).unwrap_or(s.dim);
                    prims.push(Prim::Ring { cx: INTERIOR, cy: *yi, r: 3.0, w: 1.5, color: c.opacity(0.8), dash: false });
                    continue;
                }
                for (_, yo, oref) in &outs {
                    let fout = m.edges_at(oref).into_iter().find(|(_, _, dir)| *dir).map(|(f, _, _)| f);
                    let (Some(a), Some(b)) = (&fin, &fout) else { continue };
                    if !m.routes(a, b) {
                        continue;
                    }
                    let col = mix(s.flow(a), s.flow(b), 0.5).opacity(if self.focus.is_some() { 0.25 } else { 0.85 });
                    let pts = cubic([INTERIOR, *yi], [INTERIOR + 8.0, *yi], [INTERIOR + 8.0, *yo], [INTERIOR, *yo], 16);
                    prims.push(Prim::Curve { pts, w: 2.0, color: col });
                }
            }
        }
    }
}