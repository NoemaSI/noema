use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use smap::SystemMap;

use crate::smap::flatgraph::model::Model;

mod chips;
mod geometry;
mod render;
mod rows;

pub(crate) const RIB_W: f32 = 140.0;
pub(crate) const INTERIOR: f32 = 128.0;

pub struct FlatGraph {
    pub(crate) model: Arc<Model>,
    pub(crate) open: HashSet<SharedString>,
    pub(crate) focus: Option<SharedString>,
    pub(crate) behavior: bool,
    pub(crate) light: bool,
    pub(crate) query: SharedString,
    pub(crate) input: Entity<InputState>,
}

impl FlatGraph {
    pub fn new(map: Option<SystemMap>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = Model::load(map.unwrap_or_else(|| SystemMap {
            name: "?".into(),
            description: String::new(),
            elements: vec![],
            functions: vec![],
            flows: vec![],
            states: vec![],
            transitions: vec![],
            requirements: vec![],
            relations: vec![],
            attributes: Default::default(),
        }));
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("search machines, signals, channels…"));
        cx.subscribe(&input, |me, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                let v = me.input.read(cx).value().to_string();
                me.query = v.to_lowercase().into();
                cx.notify();
            }
        })
        .detach();
        Self {
            model,
            open: HashSet::new(),
            focus: None,
            behavior: false,
            light: true,
            query: "".into(),
            input,
        }
    }

    pub(crate) fn toggle(&mut self, id: &str) {
        let id: SharedString = id.into();
        if self.open.contains(&id) {
            self.open.remove(&id);
        } else {
            self.open.insert(id);
        }
    }

    pub(crate) fn element_matches(&self, name: &str) -> bool {
        if self.query.is_empty() {
            return true;
        }
        let Some(e) = self.model.el(name) else { return false };
        let mut hay = format!("{} {}", e.name, e.description).to_lowercase();
        for s in &e.signals {
            hay.push(' ');
            hay.push_str(&s.name.to_lowercase());
            hay.push(' ');
            hay.push_str(&s.channel.clone().unwrap_or_default().to_lowercase());
        }
        for p in &e.ports {
            hay.push(' ');
            hay.push_str(&p.name.to_lowercase());
            for s in &p.signals {
                hay.push(' ');
                hay.push_str(&s.name.to_lowercase());
                hay.push(' ');
                hay.push_str(&s.channel.clone().unwrap_or_default().to_lowercase());
            }
        }
        for f in self.model.fn_alloc(name) {
            hay.push(' ');
            hay.push_str(&f.name.to_lowercase());
            if !f.description.is_empty() {
                hay.push(' ');
                hay.push_str(&f.description.to_lowercase());
            }
        }
        hay.contains(self.query.as_str())
    }

    pub(crate) fn flow_op(&self, flow: &str) -> f32 {
        match &self.focus {
            Some(f) if f.as_str() == flow => 1.0,
            Some(_) => 0.15,
            None => 1.0,
        }
    }
}
