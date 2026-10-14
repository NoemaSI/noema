//! The system map IR: what a system *is*, one layer above what a
//! system *does*.
//!
//! `skill::mechanism_ir` answers *how a system moves*: states,
//! processes, rate laws, fitted numbers. A [`SystemMap`] answers the
//! question that comes first: given a pile of data (a SCADA log, an
//! assay CSV, a shift report), what is the system, where are its
//! boundaries, what passes across its interfaces, what functions must
//! happen — and therefore **which mechanisms are left to find**.
//!
//! The vocabulary follows the NASA Systems Engineering Handbook's
//! *architecture (system)* (Appendix B) and *logical decomposition*
//! (§4.3): constituent parts, relationships, connections, functional
//! flows, interfaces, containers, modes. It is deliberately
//! domain-neutral: a cell line, a tablet press, and a detergent line
//! are all just elements and flows.
//!
//! The same boundary as everywhere else holds:
//!
//! > The map author (the agent) controls structure *as data*. The
//! > harness controls reality.
//!
//! A map is a typed, attributed, hierarchical directed graph:
//!
//! - [`Element`] — anything that exists (hardware, software, human,
//!   process, container, environment), nested via `parent`, split
//!   inside/outside the system boundary via `external`.
//! - [`Port`] — where an element touches the world; flows terminate
//!   at ports, never at elements.
//! - [`Flow`] — something exchanged (material, energy, data,
//!   control, …) routed as port-to-port edges.
//! - [`Function`] — something that must happen, allocated to
//!   elements. `status: "unknown"` marks a **mechanism gap**: a
//!   discovery target for the `skill` loop.
//! - [`State`]/[`Transition`] — modes and behaviour.
//! - [`Requirement`]/[`Relation`] — intent and traceability.
//!
//! References are by name (`"Machine1.raw"`), never by numeric id,
//! matching the mechanism IR: agent-authored JSON that reads like the
//! data it came from.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Open-ended metadata attached to every IR object.
pub type Attributes = serde_json::Map<String, serde_json::Value>;

/// What kind of thing an element is. Domain-neutral: anything the
/// specific domain calls a "machine", "drug", or "cell line" is
/// `Hardware`/`Custom` with a descriptive kind string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    /// The system itself (exactly one, the hierarchy root).
    System,

    /// An internal grouping of elements.
    Subsystem,

    Hardware,
    Software,
    Human,
    Organization,
    Facility,

    /// Something that happens rather than something that sits there,
    /// modeled as a constituent (a maintenance process, a dosing
    /// procedure).
    Process,
    Procedure,

    /// Something consumed or supplied (power, raw material).
    Resource,

    /// A holding vessel/buffer — NASA's "container": inventory lives
    /// in it and drains out through a mechanism.
    Container,

    /// The surrounding world an element senses or is coupled to
    /// (ambient conditions).
    Environment,

    /// A structural abstraction with no obvious physical home.
    Abstract,

    /// Domain vocabulary the core must not know about.
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    In,
    Out,
    InOut,
}

/// What kind of interaction an interface carries and, by alias, what
/// a flow exchanges. Extensible by design — the factory needs
/// `Material` and `Waste`, the assay needs `Data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceKind {
    Material,
    Energy,
    Data,

    /// A command/setpoint path (controller → actuator).
    Control,

    Information,
    Force,
    Fluid,
    Thermal,
    Human,
    Waste,

    /// Passive coupling from the surroundings.
    Environment,

    Custom(String),
}

/// Alias: a flow carries the same kinds an interface admits.
pub type FlowKind = InterfaceKind;

/// What a bound data column *is* at its interface.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalRole {
    /// A measured value (`...U.Actual`).
    #[default]
    Actual,

    /// A commanded value (`...U.Setpoint`).
    Setpoint,

    /// Computed from other signals.
    Derived,

    Custom(String),
}

/// How much the map already commits to a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FunctionStatus {
    /// Assumed as given (typically an external black box); not a
    /// discovery target.
    Given,

    /// The data plainly shows it happens; a mechanism is expected.
    Observed,

    /// The mechanism is *the open question* — what the discovery
    /// loop must answer.
    #[default]
    Unknown,
}

/// A named data column bound to an element or port — the hook that
/// keeps the map answerable from the dataset.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Signal {
    pub name: String,

    #[serde(default)]
    pub role: SignalRole,

    /// The dataset column this signal reads (dotted SCADA path, CSV
    /// header, ...). Unbound signals are hypotheses the data has yet
    /// to be wired to.
    #[serde(default)]
    pub channel: Option<String>,

    #[serde(default)]
    pub units: Option<String>,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub attributes: Attributes,
}

/// An interaction point of an element. Flows connect ports, never
/// elements, so one element can have many distinct interfaces.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Port {
    pub name: String,

    pub direction: Direction,

    pub kind: InterfaceKind,

    /// Measurements taken *at* this interface (incoming raw-material
    /// properties, outgoing product quality).
    #[serde(default)]
    pub signals: Vec<Signal>,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub attributes: Attributes,
}

/// Anything that exists in (or borders) the system.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Element {
    /// Unique identifier; no dots (port references are
    /// `"element.port"`).
    pub name: String,

    pub kind: ElementKind,

    /// Containing element — the decomposition hierarchy.
    #[serde(default)]
    pub parent: Option<String>,

    /// Outside the system boundary: a black box the map does not
    /// explain, only connects to.
    #[serde(default)]
    pub external: bool,

    #[serde(default)]
    pub ports: Vec<Port>,

    /// Measurements of the element itself (temperatures, RPM,
    /// amperage — internal observables).
    #[serde(default)]
    pub signals: Vec<Signal>,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub attributes: Attributes,
}

impl Element {
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }
}

/// One port-to-port hop of a flow. Ends are `"element.port"`
/// references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Edge {
    pub from: String,
    pub to: String,
}

/// Something exchanged across interfaces: the material, energy, or
/// information the system moves. A flow may fan out (one source, many
/// sinks) or chain (buffer in, buffer out).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Flow {
    pub name: String,

    pub kind: FlowKind,

    #[serde(default)]
    pub edges: Vec<Edge>,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub attributes: Attributes,
}

/// Something the system must do, independent of which element (or
/// which equation) does it. Allocation is the bridge from the map to
/// the mechanism IR: every `Unknown` function is a place where a
/// `Mechanism` is expected.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Function {
    pub name: String,

    /// Parent function — the functional decomposition.
    #[serde(default)]
    pub parent: Option<String>,

    /// Elements that realize it.
    #[serde(default)]
    pub allocated_to: Vec<String>,

    /// Flows the function consumes.
    #[serde(default)]
    pub reads: Vec<String>,

    /// Flows the function produces into.
    #[serde(default)]
    pub writes: Vec<String>,

    #[serde(default)]
    pub status: FunctionStatus,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub attributes: Attributes,
}

/// A mode/operating state of an element (NASA: "modes").
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct State {
    pub name: String,

    /// Owning element.
    pub owner: String,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub attributes: Attributes,
}

impl State {
    /// `"owner.name"` — the reference transitions use.
    pub fn reference(&self) -> String {
        format!("{}.{}", self.owner, self.name)
    }
}

/// A state transition; `trigger`/`guard` are descriptive in v0.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Transition {
    /// `"owner.name"` state reference.
    pub from: String,

    pub to: String,

    #[serde(default)]
    pub trigger: Option<String>,

    #[serde(default)]
    pub guard: Option<String>,
}

/// Something the system must achieve, kept traceable to the
/// structure that (might) deliver it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Requirement {
    pub id: String,

    pub text: String,

    /// Elements or functions that satisfy it.
    #[serde(default)]
    pub satisfies: Vec<String>,

    /// Verification cases (free references in v0).
    #[serde(default)]
    pub verified_by: Vec<String>,

    #[serde(default)]
    pub attributes: Attributes,
}

/// Cross-cutting links the typed fields cannot say.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Relation {
    DependsOn { from: String, to: String },
    Precedes { from: String, to: String },
    TracesTo { from: String, to: String },

    /// A relationship the core vocabulary has no name for yet.
    Custom {
        relation: String,
        from: String,
        to: String,
    },
}

impl Relation {
    pub fn endpoints(&self) -> (&str, &str) {
        match self {
            Relation::DependsOn { from, to }
            | Relation::Precedes { from, to }
            | Relation::TracesTo { from, to }
            | Relation::Custom { from, to, .. } => (from, to),
        }
    }
}

/// A complete system description as data: the architecture an agent
/// writes after looking at the data, and reads from to decide what
/// mechanism to discover next.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SystemMap {
    pub name: String,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub elements: Vec<Element>,

    #[serde(default)]
    pub functions: Vec<Function>,

    #[serde(default)]
    pub flows: Vec<Flow>,

    #[serde(default)]
    pub states: Vec<State>,

    #[serde(default)]
    pub transitions: Vec<Transition>,

    #[serde(default)]
    pub requirements: Vec<Requirement>,

    #[serde(default)]
    pub relations: Vec<Relation>,

    #[serde(default)]
    pub attributes: Attributes,
}

impl SystemMap {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a map always renders")
    }

    pub fn element(&self, name: &str) -> Option<&Element> {
        self.elements.iter().find(|element| element.name == name)
    }

    /// Resolve an `"element.port"` reference.
    pub fn port(&self, port_ref: &str) -> Option<&Port> {
        let (element, port) = port_ref.split_once('.')?;

        self.element(element)?
            .ports
            .iter()
            .find(|candidate| candidate.name == port)
    }

    pub fn flow(&self, name: &str) -> Option<&Flow> {
        self.flows.iter().find(|flow| flow.name == name)
    }

    pub fn function(&self, name: &str) -> Option<&Function> {
        self.functions.iter().find(|function| function.name == name)
    }

    pub fn state(&self, state_ref: &str) -> Option<&State> {
        self.states
            .iter()
            .find(|state| state.reference() == state_ref)
    }

    pub fn requirement(&self, id: &str) -> Option<&Requirement> {
        self.requirements
            .iter()
            .find(|requirement| requirement.id == id)
    }

    /// The single internal root of the decomposition.
    pub fn system(&self) -> Option<&Element> {
        self.elements
            .iter()
            .find(|element| !element.external && element.is_root())
    }

    /// The external black boxes the system touches.
    pub fn boundary(&self) -> Vec<&Element> {
        self.elements.iter().filter(|element| element.external).collect()
    }

    /// The map's open questions: functions whose mechanism the
    /// discovery loop must find.
    pub fn discovery_targets(&self) -> Vec<&Function> {
        self.functions
            .iter()
            .filter(|function| function.status == FunctionStatus::Unknown)
            .collect()
    }
}

/// A copy with every collection in canonical order, so two maps that
/// differ only in declaration order hash equal.
pub fn canonical(map: &SystemMap) -> SystemMap {
    let mut sorted = map.clone();

    sorted.elements.sort_by(|a, b| a.name.cmp(&b.name));

    for element in &mut sorted.elements {
        sort_signals(&mut element.signals);

        element.ports.sort_by(|a, b| a.name.cmp(&b.name));

        for port in &mut element.ports {
            sort_signals(&mut port.signals);
        }
    }

    sorted.functions.sort_by(|a, b| a.name.cmp(&b.name));

    for function in &mut sorted.functions {
        sort_unique(&mut function.allocated_to);
        sort_unique(&mut function.reads);
        sort_unique(&mut function.writes);
    }

    sorted.flows.sort_by(|a, b| a.name.cmp(&b.name));

    for flow in &mut sorted.flows {
        flow.edges
            .sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));

        flow.edges.dedup();
    }

    sorted.states.sort_by(|a, b| {
        (a.owner.as_str(), a.name.as_str()).cmp(&(b.owner.as_str(), b.name.as_str()))
    });

    sorted
        .transitions
        .sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));

    sorted.requirements.sort_by(|a, b| a.id.cmp(&b.id));

    for requirement in &mut sorted.requirements {
        sort_unique(&mut requirement.satisfies);
        sort_unique(&mut requirement.verified_by);
    }

    sorted
        .relations
        .sort_by_key(|relation| serde_json::to_string(relation).unwrap_or_default());

    sorted
}

fn sort_signals(signals: &mut Vec<Signal>) {
    signals.sort_by(|a, b| a.name.cmp(&b.name));
}

fn sort_unique(names: &mut Vec<String>) {
    names.sort();
    names.dedup();
}

/// Content hash of the canonical form: the ledger's dedup key, same
/// idiom as `mechanism_ir::mechanism_hash`.
pub fn map_hash(map: &SystemMap) -> String {
    let text = serde_json::to_string(&canonical(map)).unwrap_or_default();

    let mut hasher = DefaultHasher::new();

    text.hash(&mut hasher);

    format!("{:016x}", hasher.finish())
}

pub(crate) fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();

    matches!(characters.next(), Some(first) if first.is_ascii_alphabetic())
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ordering_survives_reordering() {
        let text = r#"{
            "name": "M",
            "elements": [
                {"name": "S", "kind": "system"},
                {"name": "A", "kind": "hardware", "parent": "S",
                 "ports": [{"name": "out", "direction": "out", "kind": "data"}]},
                {"name": "B", "kind": "hardware", "parent": "S",
                 "ports": [{"name": "in", "direction": "in", "kind": "data"}]}
            ],
            "flows": [{"name": "F", "kind": "data",
                       "edges": [{"from": "A.out", "to": "B.in"}]}]
        }"#;

        let map = SystemMap::from_json(text).expect("parses");

        let mut shuffled = map.clone();

        shuffled.elements.reverse();
        shuffled.flows[0].edges.push(Edge {
            from: "A.out".to_string(),
            to: "B.in".to_string(),
        });

        assert_eq!(map_hash(&map), map_hash(&shuffled));

        shuffled.elements[1].external = true;

        assert_ne!(map_hash(&map), map_hash(&shuffled));
    }

    #[test]
    fn identifiers_are_checked() {
        assert!(is_identifier("Machine1"));
        assert!(is_identifier("zone_1"));
        assert!(!is_identifier("1zone"));
        assert!(!is_identifier("a.b"));
        assert!(!is_identifier(""));
    }
}