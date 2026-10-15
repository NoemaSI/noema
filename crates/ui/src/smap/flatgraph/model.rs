use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use smap::{FlowKind, SignalRole, SystemMap};

pub use smap::{
    Direction, Element, ElementKind, Function, FunctionStatus, InterfaceKind, Signal,
};

/// Derived, read-only model used by the view. Wraps the shared
/// `smap::SystemMap` IR with the derived orderings the flat graph
/// needs.
#[derive(Debug, Clone)]
pub struct Model {
    pub map: Arc<SystemMap>,
    /// element rows in reading order (boundary band first, sinks last)
    pub order: Vec<String>,
    pub flows_at: HashMap<String, HashMap<String, Dir>>,
    pub arch_group: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    In,
    Out,
    Both,
}

pub fn el_port(s: &str) -> (String, String) {
    let mut it = s.splitn(2, '.');
    (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string())
}

/// Display string for an element kind (snake_case, custom kinds inline).
pub fn kind_str(kind: &ElementKind) -> String {
    match kind {
        ElementKind::Custom(s) => s.clone(),
        _ => serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
    }
}

/// Display string for an interface/flow kind.
pub fn iface_str(kind: &InterfaceKind) -> String {
    match kind {
        InterfaceKind::Custom(s) => s.clone(),
        _ => serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
    }
}

pub fn status_str(status: FunctionStatus) -> &'static str {
    match status {
        FunctionStatus::Given => "given",
        FunctionStatus::Observed => "observed",
        FunctionStatus::Unknown => "unknown",
    }
}

pub fn dir_str(dir: Direction) -> &'static str {
    match dir {
        Direction::In => "in",
        Direction::Out => "out",
        Direction::InOut => "inout",
    }
}

impl Model {
    pub fn load(map: SystemMap) -> Arc<Self> {
        let order = Self::derive_order(&map);
        let flows_at = Self::derive_flows_at(&map);
        let arch_group = Self::derive_archetypes(&map);
        Arc::new(Self {
            map: Arc::new(map),
            order,
            flows_at,
            arch_group,
        })
    }

    pub fn el(&self, name: &str) -> Option<&Element> {
        self.map.elements.iter().find(|e| e.name == name)
    }

    /// Sources (external, out-only) -> hardware in topological order with
    /// district bands inserted -> sinks (external, in-only).
    fn derive_order(json: &SystemMap) -> Vec<String> {
        let is_source = |e: &Element| {
            e.external && !e.ports.is_empty() && e.ports.iter().all(|p| p.direction == Direction::Out)
        };
        let is_sink = |e: &Element| {
            e.external && !e.ports.is_empty() && e.ports.iter().all(|p| p.direction == Direction::In)
        };
        let hw: Vec<&Element> = json
            .elements
            .iter()
            .filter(|e| !e.external && e.kind == ElementKind::Hardware)
            .collect();
        let hw_names: HashSet<&str> = hw.iter().map(|e| e.name.as_str()).collect();

        // adjacency over hardware elements only
        let mut indeg: HashMap<&str, usize> = hw_names.iter().map(|n| (*n, 0)).collect();
        let mut succ: HashMap<&str, Vec<&str>> = HashMap::new();
        for f in &json.flows {
            for ed in &f.edges {
                let (a, _) = el_port(&ed.from);
                let (b, _) = el_port(&ed.to);
                let ka = hw_names.get(a.as_str()).copied();
                let kb = hw_names.get(b.as_str()).copied();
                if let (Some(ka), Some(kb)) = (ka, kb) {
                    *indeg.get_mut(&kb).unwrap() += 1;
                    succ.entry(ka).or_default().push(kb);
                }
            }
        }
        let idx: HashMap<&str, usize> =
            hw.iter().enumerate().map(|(i, e)| (e.name.as_str(), i)).collect();
        let mut ready: Vec<&str> = hw_names
            .iter()
            .filter(|n| *indeg.get(*n).unwrap() == 0)
            .copied()
            .collect();
        ready.sort_by_key(|n| idx[n]);
        let mut topo: Vec<String> = Vec::new();
        while let Some(n) = ready.first().copied() {
            ready.remove(0);
            topo.push(n.to_string());
            for s in succ.get(n).cloned().unwrap_or_default() {
                *indeg.get_mut(s).unwrap() -= 1;
                if indeg[s] == 0 {
                    ready.push(s);
                    ready.sort_by_key(|n| idx[n]);
                }
            }
        }
        for n in hw_names.iter().filter(|n| *indeg.get(*n).unwrap() > 0) {
            topo.push(n.to_string());
        }

        let mut out: Vec<String> = Vec::new();
        // sources ordered by the flow they feed (material first), matching the
        // prototype's boundary order
        let mut emitted: HashSet<String> = HashSet::new();
        for f in &json.flows {
            for ed in &f.edges {
                let (a, _) = el_port(&ed.from);
                if !emitted.contains(&a) {
                    if let Some(e) = json.elements.iter().find(|x| x.name == a) {
                        if is_source(e) {
                            emitted.insert(a.clone());
                            out.push(a.clone());
                        }
                    }
                }
            }
        }
        for e in &json.elements {
            if is_source(e) && !emitted.contains(&e.name) {
                out.push(e.name.clone());
            }
        }
        let mut emitted_parent: HashSet<String> = HashSet::new();
        for n in &topo {
            if let Some(e) = json.elements.iter().find(|x| &x.name == n) {
                if let Some(p) = &e.parent {
                    if !emitted_parent.contains(p) {
                        emitted_parent.insert(p.clone());
                        out.push(p.clone());
                    }
                }
            }
            out.push(n.clone());
        }
        for e in &json.elements {
            if is_sink(e) {
                out.push(e.name.clone());
            }
        }
        // any leftover elements (neither source/sink/hardware) appended at end
        for e in &json.elements {
            if !out.contains(&e.name) && e.kind != ElementKind::System {
                out.push(e.name.clone());
            }
        }
        out
    }

    fn derive_flows_at(json: &SystemMap) -> HashMap<String, HashMap<String, Dir>> {
        let mut m: HashMap<String, HashMap<String, Dir>> = HashMap::new();
        for f in &json.flows {
            for ed in &f.edges {
                let (a, _) = el_port(&ed.from);
                let (b, _) = el_port(&ed.to);
                let e = m.entry(a.clone()).or_default();
                *e.entry(f.name.clone()).or_insert(Dir::Out) = match e.get(&f.name) {
                    Some(Dir::In) => Dir::Both,
                    _ => Dir::Out,
                };
                let e = m.entry(b.clone()).or_default();
                *e.entry(f.name.clone()).or_insert(Dir::In) = match e.get(&f.name) {
                    Some(Dir::Out) => Dir::Both,
                    _ => Dir::In,
                };
            }
        }
        m
    }

    fn derive_archetypes(json: &SystemMap) -> HashMap<String, Vec<String>> {
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for e in &json.elements {
            if e.kind != ElementKind::Hardware {
                continue;
            }
            let mut key: Vec<String> = e
                .ports
                .iter()
                .map(|p| format!("{}{}", dir_str(p.direction), iface_str(&p.kind)))
                .collect();
            key.sort();
            let mut sg: Vec<String> = e.signals.iter().map(|s| s.name.clone()).collect();
            sg.sort();
            let mut pg: Vec<String> = e
                .ports
                .iter()
                .flat_map(|p| {
                    p.signals.iter().map(|s| {
                        s.name
                            .trim_end_matches("_setpoint")
                            .chars()
                            .map(|c| if c.is_ascii_digit() { 'N' } else { c })
                            .collect::<String>()
                    })
                })
                .collect();
            pg.sort();
            pg.dedup();
            let k = format!("{:?}|{:?}|{:?}", key, sg, pg);
            groups.entry(k).or_default().push(e.name.clone());
        }
        groups.retain(|_, v| v.len() > 1);
        groups
    }

    pub fn is_district(&self, name: &str) -> bool {
        self.map.elements.iter().any(|e| e.parent.as_deref() == Some(name))
    }

    pub fn children(&self, name: &str) -> Vec<&Element> {
        self.map.elements.iter().filter(|e| e.parent.as_deref() == Some(name)).collect()
    }

    pub fn subtree(&self, name: &str) -> Vec<String> {
        let mut out = vec![name.to_string()];
        for c in self.children(name) {
            out.extend(self.subtree(&c.name));
        }
        out
    }

    pub fn flows_at(&self, name: &str) -> Vec<(String, Dir)> {
        let mut v: Vec<(String, Dir)> = self
            .flows_at
            .get(name)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
        // stable order following json flow order
        let order: Vec<&String> = self.map.flows.iter().map(|f| &f.name).collect();
        v.sort_by_key(|(k, _)| order.iter().position(|o| *o == k).unwrap_or(99));
        v
    }

    pub fn fn_alloc(&self, name: &str) -> Vec<&Function> {
        self.map.functions.iter().filter(|f| f.allocated_to.iter().any(|a| a == name)).collect()
    }

    pub fn fn_by_name(&self, name: &str) -> Option<&Function> {
        self.map.functions.iter().find(|f| f.name == name)
    }

    pub fn unknown_fn_touching(&self, el: &str, flow: &str) -> bool {
        self.fn_alloc(el).iter().any(|f| {
            f.status == FunctionStatus::Unknown && (f.reads.iter().any(|r| r == flow) || f.writes.iter().any(|w| w == flow))
        })
    }

    pub fn edges_at(&self, port_ref: &str) -> Vec<(String, String, bool)> {
        // returns (flow_name, far_ref, dir_to)
        let mut out = Vec::new();
        for f in &self.map.flows {
            for ed in &f.edges {
                if ed.from == port_ref {
                    out.push((f.name.clone(), ed.to.clone(), true));
                }
                if ed.to == port_ref {
                    out.push((f.name.clone(), ed.from.clone(), false));
                }
            }
        }
        out
    }

    pub fn routes(&self, fin: &str, fout: &str) -> bool {
        self.map.functions.iter().any(|f| {
            f.reads.iter().any(|r| r == fin) && f.writes.iter().any(|w| w == fout)
        })
    }

    pub fn count_signals(&self, e: &Element) -> usize {
        e.signals.len()
            + e.ports.iter().map(|p| Self::pair_signals(&p.signals).len()).sum::<usize>()
    }

    /// Pair `name` (actual) with `name_setpoint`: returns (name, has_act, has_sp, primary sig).
    pub fn pair_signals(sigs: &[Signal]) -> Vec<(String, bool, bool, Signal)> {
        let mut used: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        for s in sigs {
            if s.role != SignalRole::Actual {
                continue;
            }
            used.insert(s.name.clone());
            let sp = sigs.iter().find(|x| x.role == SignalRole::Setpoint && x.name == format!("{}_setpoint", s.name));
            if let Some(sp) = sp {
                used.insert(sp.name.clone());
                out.push((s.name.clone(), true, true, s.clone()));
            } else {
                out.push((s.name.clone(), true, false, s.clone()));
            }
        }
        for s in sigs {
            if !used.contains(&s.name) {
                out.push((s.name.clone(), false, true, s.clone()));
            }
        }
        out
    }

    pub fn lane(&self, flow: &str) -> usize {
        self.map.flows.iter().position(|f| f.name == flow).unwrap_or(0)
    }

    pub fn flow_width(&self, flow: &str) -> f32 {
        match self.map.flows.iter().find(|f| f.name == flow).map(|f| &f.kind) {
            Some(FlowKind::Material) => 3.0,
            _ => 1.5,
        }
    }

    pub fn flow_dash(&self, flow: &str) -> Option<[f32; 2]> {
        self.map
            .flows
            .iter()
            .find(|f| f.name == flow)
            .map(|f| f.kind == InterfaceKind::Environment)
            .and_then(|dashed| if dashed { Some([2.0, 5.0]) } else { None })
    }
}

/// Derived variable envelope for unknown mechanisms (∴ — inferred from
/// topology + signal classes, not present in the file). Kept verbatim from
/// the HTML prototype.
pub fn envelope(fn_name: &str) -> Option<(&'static str, &'static str)> {
    match fn_name {
        "ShapeStage1" => Some((
            "intake properties ×4 · feeder setpoint · zone temps ×2 · motor rpm/amperage · material pressure/temp",
            "Stage1Product — observed downstream: 15 quality dimensions on Combiner.output",
        )),
        "Combine" => Some((
            "three parallel streams · operation temps ×3 (the only internal observables)",
            "Stage1Product — recirculated, blended",
        )),
        "FinishStage2" => Some((
            "combined stream · temperature ladders ×5 / ×6 · pressure",
            "Stage2Product — 15 quality dimensions on Machine5.output",
        )),
        "HoldTemperatures" => Some((
            "— unmapped: no controller in the map",
            "— unmapped",
        )),
        _ => None,
    }
}