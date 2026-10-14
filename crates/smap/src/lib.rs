//! # smap — the system map
//!
//! `smap` is the representation layer that sits *above* the
//! [`skill`] mechanism IR. Where a `Mechanism` says how a system
//! moves (states, fluxes, fitted rates), a [`SystemMap`] says what
//! the system *is*: its elements and boundary, the ports where it
//! touches the world, the flows that cross them, the functions that
//! must happen, the modes it runs in, and what it must achieve.
//!
//! The intended producer is an agent looking at raw data — a SCADA
//! log with dotted column names, an assay CSV, a shift report — and
//! answering one question:
//!
//! > Given this data, what is the system, and **which mechanisms are
//! > left to find?**
//!
//! The answer is JSON conforming to [`SystemMap`]'s schema
//! ([`schema_json`]). Functions whose `status` is `"unknown"` are the
//! mechanism gaps: each one is a hand-off point where the
//! `skill` discovery loop is expected to eventually produce a fitted
//! `Mechanism`. The vocabulary is the NASA Systems Engineering
//! Handbook's — architecture (Appendix B), logical decomposition
//! (§4.3): entities, relationships, functional flows, interfaces,
//! containers, modes — so nothing in the core knows about factories,
//! assays, or cell lines.
//!
//! v0 scope is *representation only*: model, validate, hash, schema.
//! No lowering, no analysis, no simulation — those grow on top of a
//! map that can already state a system (and its questions) truthfully.
//!
//! ```text
//! data ──▶ agent authors SystemMap JSON ──▶ validate ──▶ discovery_targets()
//!                                                        │
//!                              each unknown function ──▶ skill loop (mechanism IR)
//! ```

pub mod model;
pub mod render;
pub mod validate;

use std::path::PathBuf;

pub use model::{
    Attributes, Direction, Edge, Element, ElementKind, Flow, FlowKind, Function, FunctionStatus,
    InterfaceKind, Port, Relation, Requirement, Signal, SignalRole, State, SystemMap, Transition,
    canonical, map_hash,
};
pub use render::{Layout, to_mermaid, to_mermaid_with};
pub use validate::validate;

/// The JSON schema of a system map — the contract an agent authors
/// against.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(SystemMap);

    serde_json::to_string_pretty(&schema).expect("the schema always renders")
}

/// Read a system map from disk
pub fn read_system_map(path: PathBuf) -> Result<SystemMap, anyhow::Error> {
    let text = std::fs::read_to_string(path)?;
    Ok(SystemMap::from_json(&text)?)
} 

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_renders() {
        let schema = schema_json();

        assert!(schema.contains("SystemMap"), "schema names the type");
        assert!(schema.contains("discovery") || schema.contains("status"));
    }
}
