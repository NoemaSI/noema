use std::collections::HashSet;

use gpui_kit::*;

use smap::{Direction, ElementKind, FunctionStatus, SignalRole};

use crate::smap::flatgraph::model::{self, Model};

use super::FlatGraph;

#[derive(Clone, Copy, PartialEq)]
pub enum RowKind {
    Band,
    District,
    Station,
    Platform,
    Train,
    Exit,
    StateGroup,
    Fn,
    FnDetail,
}

pub struct Row {
    pub id: SharedString,
    pub kind: RowKind,
    pub h: f32,
    pub flows: Vec<SharedString>,
    pub dim: bool,
    pub el: Option<SharedString>,
    pub port: Option<SharedString>,
    pub train: Option<Train>,
    pub open: bool,
    // payload
    pub label: SharedString,
    pub sub: SharedString,
    pub sub2: SharedString,
    pub chips: Vec<(SharedString, usize)>, // (text, color-role)
    pub status: SharedString,
    pub flows_data: Vec<SharedString>,
}

#[derive(Clone)]
pub struct Train {
    pub flow: SharedString,
    pub local_port: SharedString,
    pub far_port: SharedString,
    pub dir_to: bool,
}

impl FlatGraph {
    pub(crate) fn row(id: &str, kind: RowKind, h: f32) -> Row {
        Row {
            id: id.into(),
            kind,
            h,
            flows: vec![],
            dim: false,
            el: None,
            port: None,
            train: None,
            open: false,
            label: "".into(),
            sub: "".into(),
            sub2: "".into(),
            chips: vec![],
            status: "".into(),
            flows_data: vec![],
        }
    }

    pub(crate) fn build_rows(&mut self) -> Vec<Row> {
        let m = self.model.clone();
        let mut rows: Vec<Row> = Vec::new();
        let open = self.open.clone();
        let q = self.query.clone();

        let push_fn = |rows: &mut Vec<Row>, fname: &str, mount: Option<&str>, alloc_txt: String| {
            let f = m.map.functions.iter().find(|x| x.name == fname).unwrap();
            let id: SharedString = if let Some(mo) = mount {
                format!("fn|{}|{}", fname, mo).into()
            } else {
                format!("fn|{}", fname).into()
            };
            let is_open = open.contains(&id);
            let mut rw: Vec<SharedString> = Vec::new();
            let recirc = f.reads.len() == 1
                && f.writes.len() == 1
                && f.reads[0] == f.writes[0];
            if f.reads.is_empty() && f.writes.is_empty() {
                if f.status == FunctionStatus::Unknown {
                    rw.push("⇢ unmapped interface (no controller in the map)".into());
                } else if !f.writes.is_empty() {
                    rw.extend(f.writes.iter().map(|w| w.clone().into()));
                } else {
                    rw.extend(f.reads.iter().map(|r| r.clone().into()));
                }
            } else if recirc {
                rw.push(f.reads[0].clone().into());
                rw.push("⟲ recirculates".into());
            } else {
                if f.reads.is_empty() {
                    rw.push("∅".into());
                } else {
                    rw.extend(f.reads.iter().map(|r| r.clone().into()));
                }
                rw.push("⇄".into());
                if f.writes.is_empty() {
                    rw.push("∅".into());
                } else {
                    rw.extend(f.writes.iter().map(|w| w.clone().into()));
                }
            }
            let mut r = Self::row(&id, RowKind::Fn, 36.0);
            r.label = fname.into();
            r.status = model::status_str(f.status).into();
            r.sub = alloc_txt.into();
            r.chips = rw.into_iter().map(|s| (s, 0)).collect();
            r.flows = f.reads.iter().chain(f.writes.iter()).cloned().map(SharedString::from).collect();
            r.open = is_open;
            rows.push(r);
            if is_open {
                let mut d = Self::row(&format!("{}|d", id), RowKind::FnDetail, 112.0);
                d.sub = f.description.clone().into();
                if let Some((i, o)) = model::envelope(fname) {
                    d.sub2 = format!("∴ derived variable envelope (from flow topology + signal classes, not in file)\n▸ plausible inputs: {}\n▸ plausible outputs: {}", i, o).into();
                }
                rows.push(d);
            }
        };

        if self.behavior {
            for (st, label) in [
                (FunctionStatus::Unknown, "UNKNOWN — MECHANISMS TO FIND"),
                (FunctionStatus::Observed, "OBSERVED — BEHAVIOR SEEN, MECHANISM UNSEEN"),
                (FunctionStatus::Given, "GIVEN — BOUNDARY ASSUMPTIONS"),
            ] {
                let n = m.map.functions.iter().filter(|f| f.status == st).count();
                let mut b = Self::row(&format!("band|{}", model::status_str(st)), RowKind::Band, 30.0);
                b.label = format!("{} ({})", label, n).into();
                rows.push(b);
                for f in m.map.functions.iter().filter(|f| f.status == st) {
                    push_fn(&mut rows, &f.name, None, format!("@ {}", f.allocated_to.join(", ")));
                }
            }
            return rows;
        }

        let mut band = Self::row("band|boundary", RowKind::Band, 26.0);
        band.label = "BOUNDARY — OUTSIDE THE RECORDED LINE".into();
        rows.push(band);

        for name in m.order.clone() {
            let Some(e) = m.el(&name) else { continue };
            let dim = !q.is_empty() && !self.element_matches(&name);
            if !q.is_empty() && !dim && !name.to_lowercase().contains(q.as_str()) {
                self.open.insert(name.as_str().into());
            }
            if m.is_district(&name) {
                let sub = m.subtree(&name);
                let machines = sub.iter().filter(|n| m.el(n).map(|x| x.kind == ElementKind::Hardware).unwrap_or(false)).count();
                let ports: usize = sub.iter().map(|n| m.el(n).map(|x| x.ports.len()).unwrap_or(0)).sum();
                let sigs: usize = sub.iter().map(|n| m.el(n).map(|x| m.count_signals(x)).unwrap_or(0)).sum();
                let unk: HashSet<String> = sub
                    .iter()
                    .flat_map(|n| m.fn_alloc(n))
                    .filter(|f| f.status == FunctionStatus::Unknown)
                    .map(|f| f.name.clone())
                    .collect();
                let mut r = Self::row(&name, RowKind::District, 36.0);
                r.label = name.clone().into();
                r.sub = format!("∴ {} machines · {} ports · {} signals · {} unknown mechanisms within", machines, ports, sigs, unk.len()).into();
                r.el = Some(name.clone().into());
                r.flows = m.flows_at(&name).into_iter().map(|(f, _)| f.into()).collect();
                r.dim = dim;
                rows.push(r);
            } else {
                let id: SharedString = name.clone().into();
                let is_open = self.open.contains(&id);
                let arch = m
                    .arch_group
                    .values()
                    .find(|g| g.contains(&name))
                    .map(|g| g.clone());
                let mut r = Self::row(&name, RowKind::Station, 44.0);
                r.label = name.clone().into();
                r.sub = model::kind_str(&e.kind).into();
                r.sub2 = e.description.clone().into();
                if let Some(g) = arch {
                    r.chips.push((format!("⧉ ×{}", g.len()).into(), 1));
                }
                r.el = Some(id.clone());
                r.open = is_open;
                r.flows = m.flows_at(&name).into_iter().map(|(f, _)| f.into()).collect();
                r.dim = dim;
                rows.push(r);

                if is_open {
                    self.build_port_rows(&mut rows, &name, e);
                    if !e.signals.is_empty() {
                        let gid = format!("{}|state", name);
                        let g_open = self.open.contains(&SharedString::from(gid.as_str()));
                        let mut r = Self::row(&gid, RowKind::StateGroup, 30.0);
                        r.label = "state exits".into();
                        r.sub = format!("×{} — how the machine feels", e.signals.len()).into();
                        r.el = Some(name.clone().into());
                        r.flows = m.flows_at(&name).into_iter().map(|(f, _)| f.into()).collect();
                        r.open = g_open;
                        rows.push(r);
                        if g_open {
                            for s in &e.signals {
                                let mut r = Self::row(&format!("{}|x|{}", gid, s.name), RowKind::Exit, 30.0);
                                r.label = s.name.clone().into();
                                r.status = if s.role == SignalRole::Setpoint { "SP".into() } else { "ACT".into() };
                                r.sub = s.channel.clone().unwrap_or_default().into();
                                rows.push(r);
                            }
                        }
                    }
                    for f in m.fn_alloc(&name) {
                        push_fn(&mut rows, &f.name, Some(&name), String::new());
                    }
                }
            }
        }
        rows
    }

    /// platform (port) rows + their expanded train/exit rows
    fn build_port_rows(&self, rows: &mut Vec<Row>, name: &str, e: &smap::Element) {
        let m = &self.model;
        for p in &e.ports {
            let ref_ = format!("{}.{}", name, p.name);
            let eds = m.edges_at(&ref_);
            let pid = format!("{}|p|{}", name, p.name);
            let p_open = self.open.contains(&SharedString::from(pid.as_str()));
            let pairs = Model::pair_signals(&p.signals);
            let mut r = Self::row(&pid, RowKind::Platform, 30.0);
            r.label = format!("{} {}", if p.direction == Direction::In { "▸" } else { "◂" }, p.name).into();
            r.sub = model::iface_str(&p.kind).into();
            r.chips = eds.iter().map(|(f, _, _)| (f.clone().into(), 0)).collect();
            r.sub2 = if pairs.is_empty() {
                "no exits — stream properties unmeasured here".into()
            } else {
                format!("exits {}", pairs.len()).into()
            };
            r.port = Some(ref_.clone().into());
            r.el = Some(name.into());
            r.flows = eds.iter().map(|(f, _, _)| f.clone().into()).collect::<HashSet<_>>().into_iter().collect();
            r.flows_data = r.flows.clone();
            r.open = p_open;
            rows.push(r);
            if p_open {
                for (flow, far, dir_to) in &eds {
                    let mut r = Self::row(&format!("{}|t|{}", pid, far), RowKind::Train, 30.0);
                    r.label = format!("{} {}", if *dir_to { "▸ to" } else { "◂ from" }, far).into();
                    r.chips = vec![(flow.clone().into(), 0)];
                    r.flows = vec![flow.clone().into()];
                    r.train = Some(Train {
                        flow: flow.clone().into(),
                        local_port: ref_.clone().into(),
                        far_port: far.clone().into(),
                        dir_to: *dir_to,
                    });
                    rows.push(r);
                }
                for (nm, act, sp, sig) in &pairs {
                    let mut r = Self::row(&format!("{}|x|{}", pid, nm), RowKind::Exit, 30.0);
                    r.label = nm.clone().into();
                    r.status = if *act && *sp {
                        "ACT·SP".into()
                    } else if *act {
                        "ACT".into()
                    } else {
                        "SP".into()
                    };
                    r.sub = sig.channel.clone().unwrap_or_default().into();
                    r.flows = r.flows_data.clone();
                    rows.push(r);
                }
            }
        }
    }
}