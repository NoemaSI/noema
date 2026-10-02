//! The mechanism IR: the typed structure the theorist authors instead
//! of code.
//!
//! A `Mechanism` is a reaction graph — entities, states, processes,
//! algebraic definitions, and an observable — serialized as JSON. The
//! harness validates it, lowers it deterministically to Modelica
//! ([`crate::lower`]), and fits it; the LLM never writes Modelica or
//! Rust at all. This is the same boundary as everywhere else, tightened:
//!
//! > The model author controls structure *as data*. The harness
//! > controls reality.
//!
//! The only domain vocabulary the IR knows is the pack's
//! [`crate::lexicon::Lexicon`]: the declared input channels (any
//! number — each `driven_by` and `input` expression names one), the
//! declared output channels (each observable emits one), and
//! (descriptively) the entity roles the domain speaks about.
//!
//! Verified semantics this IR is built around (see the `modelica`
//! crate): the CasADi artifact bakes state initial conditions (`x0`)
//! at compile time, so `State::initial` is a compile-time constant by
//! construction — the optimizer can never move it. Parameter-dependent
//! totals (a binding capacity, a pool size) are expressed with the
//! conservation idiom: keep the occupied state dynamic and define the
//! free fraction as an algebraic variable minus a parameter.

use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lexicon::Lexicon;

/// A complete mechanistic hypothesis as executable data.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Mechanism {
    /// Modelica model identifier (the artifact is named after it).
    pub name: String,

    /// Optional compartment labels; entities may reference them.
    #[serde(default)]
    pub compartments: Vec<String>,

    /// Physical participants: the driven reservoir, carriers,
    /// complexes, conformers.
    #[serde(default)]
    pub entities: Vec<Entity>,

    /// Occupiable states, each owned by one entity.
    #[serde(default)]
    pub states: Vec<State>,

    /// Every fitted knob, in theta order. All rates, capacities,
    /// scales, and drift terms reference parameters declared here.
    #[serde(default)]
    pub parameters: Vec<ParameterDecl>,

    /// Auxiliary variables defined by assignment; order is inferred.
    #[serde(default)]
    pub algebraic: Vec<Algebraic>,

    /// Kinetic processes moving occupancy between states.
    #[serde(default)]
    pub processes: Vec<Process>,

    /// How state occupancies produce a measured output channel. Several
    /// observables may be declared (one per declared output channel);
    /// `observable` is the conventional single one.
    #[serde(default)]
    pub observable: Option<Observable>,

    /// Additional observables for multi-output mechanisms (each
    /// names one declared output channel).
    #[serde(default)]
    pub observables: Vec<Observable>,
}

impl Mechanism {
    /// Every declared observable: the single `observable` first, then
    /// the extra `observables`.
    pub fn all_observables(&self) -> Vec<&Observable> {
        self.observable.iter().chain(&self.observables).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// Slaved to one input channel's recorded drive trace (a
    /// reservoir).
    Driven,

    /// Everything else: free, tethered, or any other non-driven
    /// participant. What a dynamic entity *is* (a ligand in solution,
    /// a receptor on a sensor surface) is descriptive only — carry it
    /// in [`Entity::role`] using the pack lexicon's vocabulary.
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Entity {
    pub name: String,
    pub kind: EntityKind,

    #[serde(default)]
    pub compartment: Option<String>,

    /// Only on the driven entity; must name one of the lexicon's
    /// declared input channels.
    #[serde(default)]
    pub driven_by: Option<String>,

    /// Optional descriptive role (e.g. `"immobilized"`), taken from
    /// the pack lexicon's role vocabulary. Purely descriptive: the
    /// lowerer treats every dynamic entity identically.
    #[serde(default)]
    pub role: Option<String>,
}

impl Entity {
    pub fn is_driven(&self) -> bool {
        self.kind == EntityKind::Driven
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct State {
    /// Owning entity name.
    pub entity: String,
    pub name: String,

    /// Compile-time constant initial occupancy. The fitted optimizer
    /// can never change it — express parameter-dependent totals with
    /// the algebraic conservation idiom instead.
    pub initial: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ParameterDecl {
    pub name: String,
    pub start: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Algebraic {
    pub name: String,
    pub expr: Expr,
}

/// One reactant or product: a state reference plus its stoichiometry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StoichRef {
    /// `"Entity.state"`.
    pub state: String,

    #[serde(default = "one")]
    pub stoich: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Process {
    pub name: String,
    #[serde(default)]
    pub from: Vec<StoichRef>,
    #[serde(default)]
    pub to: Vec<StoichRef>,
    pub rate: RateLaw,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RateLaw {
    /// Flux = parameter * (every `from` state raised to its stoich).
    /// Zeroth order (empty `from`) is allowed: a constant source.
    MassAction { parameter: String },

    /// Flux = expr, in amount/time. Use for transport between
    /// compartments and anything mass action cannot say.
    Custom { expr: Expr },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Observable {
    /// Declared output channel this observable emits (a name from
    /// [`crate::lexicon::Lexicon::outputs`]).
    #[serde(default = "default_channel")]
    pub channel: String,

    /// Weighted sum over states and/or algebraic variables.
    #[serde(default)]
    pub contributions: Vec<Contribution>,

    /// Optional overall gain, referencing a parameter.
    #[serde(default)]
    pub scale: Option<String>,

    /// Optional baseline artifact term.
    #[serde(default)]
    pub drift: Option<DriftSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Contribution {
    /// A `"Entity.state"` reference or an algebraic variable name.
    pub target: String,

    #[serde(default = "one")]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriftSpec {
    Offset { parameter: String },
    Linear { rate: String },
    Exponential { amplitude: String, tau: String },
}

/// A tiny typed expression AST. Everything renders to Modelica
/// deterministically; nothing is ever `eval`ed from a raw string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Expr {
    Var { name: String },
    Param { name: String },
    Input { name: String },
    Const { value: f64 },
    Sum { terms: Vec<Expr> },
    Product { factors: Vec<Expr> },
    Ratio { numerator: Box<Expr>, denominator: Box<Expr> },
    Neg { operand: Box<Expr> },
    Pow { base: Box<Expr>, exponent: Box<Expr> },
    Exp { operand: Box<Expr> },
    Ln { operand: Box<Expr> },
}

fn one() -> f64 {
    1.0
}

fn default_channel() -> String {
    "signal".to_string()
}

/// The Modelica variable name of a state reference: `"PL.bound"` ->
/// `"PL_bound"`.
pub fn state_var_name(state_ref: &str) -> String {
    state_ref.replace('.', "_")
}

/// Validate a mechanism against the IR contract and the pack's
/// lexicon. All problems are collected so a repair round can fix them
/// in one pass.
pub fn validate(mechanism: &Mechanism, lexicon: &Lexicon) -> Result<(), String> {
    let mut errors = Vec::new();

    let channel_list = |channels: &[crate::lexicon::Channel]| -> String {
        channels
            .iter()
            .map(|channel| format!("{:?}", channel.name))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let require = |errors: &mut Vec<String>, ok: bool, message: String| {
        if !ok {
            errors.push(message);
        }
    };

    require(
        &mut errors,
        is_identifier(&mechanism.name),
        format!("`name` {:?} is not a Modelica identifier", mechanism.name),
    );

    // Compartments: labels only.
    let compartments: HashSet<&str> = mechanism
        .compartments
        .iter()
        .map(String::as_str)
        .collect();

    require(
        &mut errors,
        compartments.len() == mechanism.compartments.len(),
        "`compartments` contains duplicates".to_string(),
    );

    for compartment in &mechanism.compartments {
        require(
            &mut errors,
            is_identifier(compartment),
            format!("compartment {compartment:?} is not an identifier"),
        );
    }

    // Entities.
    let mut driven_count = 0usize;

    let mut entity_names = HashSet::new();

    for entity in &mechanism.entities {
        require(
            &mut errors,
            is_identifier(&entity.name),
            format!("entity {entity:?} is not an identifier"),
        );

        require(
            &mut errors,
            entity_names.insert(entity.name.as_str()),
            format!("duplicate entity name {:?}", entity.name),
        );

        match entity.kind {
            EntityKind::Driven => {
                driven_count += 1;

                require(
                    &mut errors,
                    entity
                        .driven_by
                        .as_deref()
                        .is_some_and(|name| lexicon.input(name).is_some()),
                    format!(
                        "driven entity {:?} must set \"driven_by\" to a declared \
                         input channel (declared: {})",
                        entity.name,
                        channel_list(&lexicon.inputs),
                    ),
                );
            }
            _ => require(
                &mut errors,
                entity.driven_by.is_none(),
                format!(
                    "entity {:?} is not driven but declares driven_by",
                    entity.name,
                ),
            ),
        }

        if let Some(compartment) = &entity.compartment {
            require(
                &mut errors,
                compartments.contains(compartment.as_str()),
                format!(
                    "entity {:?} references undeclared compartment {compartment:?}",
                    entity.name,
                ),
            );
        }
    }

    require(
        &mut errors,
        driven_count == 1,
        format!(
            "exactly one entity must be driven (found {driven_count}); \
             it carries the experiment's drive for the channel it names",
        ),
    );

    // States.
    let mut state_refs = HashSet::new();

    let mut driven_states = 0usize;

    let mut dynamic_states = 0usize;

    for state in &mechanism.states {
        let reference = format!("{}.{}", state.entity, state.name);

        require(
            &mut errors,
            is_identifier(&state.name) && entity_names.contains(state.entity.as_str()),
            format!(
                "state {reference:?} must name a declared entity and be an identifier",
            ),
        );

        require(
            &mut errors,
            state_refs.insert(reference.clone()),
            format!("duplicate state {reference:?}"),
        );

        require(
            &mut errors,
            state.initial.is_finite(),
            format!("state {reference:?} has a non-finite initial value"),
        );

        if let Some(entity) = mechanism
            .entities
            .iter()
            .find(|entity| entity.name == state.entity)
        {
            if entity.kind == EntityKind::Driven {
                driven_states += 1;
            } else {
                dynamic_states += 1;
            }
        }
    }

    require(
        &mut errors,
        driven_states <= 1,
        "the driven entity must have exactly one state (the reservoir)".to_string(),
    );

    require(
        &mut errors,
        dynamic_states >= 1,
        "at least one dynamic (non-driven) state is required".to_string(),
    );

    // Parameters: the theta namespace.
    let mut parameter_names = HashSet::new();

    for parameter in &mechanism.parameters {
        require(
            &mut errors,
            is_identifier(&parameter.name),
            format!("parameter {parameter:?} is not an identifier"),
        );

        require(
            &mut errors,
            !is_reserved(&parameter.name, lexicon),
            format!(
                "parameter {:?} collides with a reserved Modelica name",
                parameter.name,
            ),
        );

        require(
            &mut errors,
            parameter_names.insert(parameter.name.as_str()),
            format!("duplicate parameter name {:?}", parameter.name),
        );

        require(
            &mut errors,
            parameter.min.is_finite()
                && parameter.max.is_finite()
                && parameter.start.is_finite()
                && parameter.min < parameter.max
                && parameter.start >= parameter.min
                && parameter.start <= parameter.max,
            format!(
                "parameter {:?} needs min < max and min <= start <= max \
                 (got [{}, {}], start {})",
                parameter.name, parameter.min, parameter.max, parameter.start,
            ),
        );
    }

    require(
        &mut errors,
        !mechanism.parameters.is_empty(),
        "declare at least one parameter: a mechanism without fitted \
         knobs cannot be fitted"
            .to_string(),
    );

    // Algebraic variables.
    let mut algebraic_names = HashSet::new();

    for algebraic in &mechanism.algebraic {
        require(
            &mut errors,
            is_identifier(&algebraic.name),
            format!("algebraic {:?} is not an identifier", algebraic.name),
        );

        require(
            &mut errors,
            !is_reserved(&algebraic.name, lexicon),
            format!(
                "algebraic {:?} collides with a reserved Modelica name",
                algebraic.name,
            ),
        );

        require(
            &mut errors,
            algebraic_names.insert(algebraic.name.as_str()),
            format!("duplicate algebraic name {:?}", algebraic.name),
        );
    }

    // Variable-name collisions (state vars are flattened with `_`).
    let mut var_names = HashSet::new();

    for state in &mechanism.states {
        require(
            &mut errors,
            var_names.insert(state_var_name(&format!(
                "{}.{}",
                state.entity, state.name,
            ))),
            format!(
                "state variable for {:?}.{:?} collides with another variable name",
                state.entity, state.name,
            ),
        );
    }

    for algebraic in &mechanism.algebraic {
        require(
            &mut errors,
            var_names.insert(algebraic.name.clone()),
            format!(
                "algebraic {:?} collides with a state variable name",
                algebraic.name,
            ),
        );
    }

    // Expression scoping.
    let scope = ExprScope {
        state_refs: &state_refs,
        algebraic: &algebraic_names,
        parameters: &parameter_names,
        inputs: &lexicon.inputs,
    };

    for algebraic in &mechanism.algebraic {
        let mut self_reference = HashSet::new();

        collect_vars(&algebraic.expr, &mut self_reference);

        require(
            &mut errors,
            !self_reference.contains(&algebraic.name),
            format!("algebraic {:?} references itself", algebraic.name),
        );

        walk_expr(
            &algebraic.expr,
            &scope,
            &format!("algebraic {:?}", algebraic.name),
            &mut errors,
        );
    }

    // Processes.
    let mut process_names = HashSet::new();

    for process in &mechanism.processes {
        require(
            &mut errors,
            is_identifier(&process.name),
            format!("process {process:?} is not an identifier"),
        );

        require(
            &mut errors,
            process_names.insert(process.name.as_str()),
            format!("duplicate process name {:?}", process.name),
        );

        for stoich in process.from.iter().chain(&process.to) {
            // A participant is a state ("Entity.state") or an
            // algebraic variable (plain name). Only non-driven states
            // accumulate mass; the rest are passive reservoirs.
            let resolved = state_refs.contains(stoich.state.as_str())
                || algebraic_names.contains(stoich.state.as_str());

            require(
                &mut errors,
                resolved,
                format!(
                    "process {:?} references unknown participant {:?} \
                     (use \"Entity.state\" or an algebraic name)",
                    process.name, stoich.state,
                ),
            );

            require(
                &mut errors,
                stoich.stoich.is_finite() && stoich.stoich > 0.0,
                format!(
                    "process {:?} stoichiometry for {:?} must be > 0",
                    process.name, stoich.state,
                ),
            );
        }

        match &process.rate {
            RateLaw::MassAction { parameter } => {
                require(
                    &mut errors,
                    parameter_names.contains(parameter.as_str()),
                    format!(
                        "process {:?} mass-action parameter {parameter:?} is not declared",
                        process.name,
                    ),
                );

                let total: f64 = process.from.iter().map(|r| r.stoich).sum();

                require(
                    &mut errors,
                    total <= 2.0,
                    format!(
                        "process {:?} mass-action order {total} exceeds bimolecular",
                        process.name,
                    ),
                );
            }
            RateLaw::Custom { expr } => walk_expr(
                expr,
                &scope,
                &format!("process {:?}", process.name),
                &mut errors,
            ),
        }
    }

    // Observables: one per declared output channel.
    let observables = mechanism.all_observables();

    require(
        &mut errors,
        !observables.is_empty(),
        "declare at least one observable".to_string(),
    );

    let mut observable_channels = HashSet::new();

    for observable in observables {
        require(
            &mut errors,
            lexicon.output(&observable.channel).is_some(),
            format!(
                "observable channel {:?} is not a declared output channel \
                 (declared: {})",
                observable.channel,
                channel_list(&lexicon.outputs),
            ),
        );

        require(
            &mut errors,
            observable_channels.insert(observable.channel.as_str()),
            format!("duplicate observable for channel {:?}", observable.channel),
        );

        require(
            &mut errors,
            !observable.contributions.is_empty(),
            format!(
                "observable {:?} needs at least one contribution",
                observable.channel,
            ),
        );

        for contribution in &observable.contributions {
            let resolved = state_refs.contains(contribution.target.as_str())
                || algebraic_names.contains(contribution.target.as_str());

            require(
                &mut errors,
                resolved,
                format!(
                    "observable contribution target {:?} is neither a state \
                     nor an algebraic variable",
                    contribution.target,
                ),
            );

            require(
                &mut errors,
                contribution.weight.is_finite() && contribution.weight != 0.0,
                format!(
                    "observable weight for {:?} must be finite and nonzero",
                    contribution.target,
                ),
            );
        }

        if let Some(scale) = &observable.scale {
            require(
                &mut errors,
                parameter_names.contains(scale.as_str()),
                format!("observable scale {scale:?} is not a declared parameter"),
            );
        }

        if let Some(drift) = &observable.drift {
            let parameters: Vec<&str> = match drift {
                DriftSpec::Offset { parameter } => vec![parameter.as_str()],
                DriftSpec::Linear { rate } => vec![rate.as_str()],
                DriftSpec::Exponential { amplitude, tau } => {
                    vec![amplitude.as_str(), tau.as_str()]
                }
            };

            for parameter in parameters {
                require(
                    &mut errors,
                    parameter_names.contains(parameter),
                    format!("observable drift parameter {parameter:?} is not declared"),
                );
            }
        }
    }

    // Every declared output channel must be predicted: the lowered
    // model declares `output Real <ch>` for each observable, and the
    // candidate contract (and the fixed loss) demand the full set.
    for channel in &lexicon.outputs {
        require(
            &mut errors,
            observable_channels.contains(channel.name.as_str()),
            format!(
                "output channel {:?} has no observable — every declared \
                 output channel must be predicted",
                channel.name,
            ),
        );
    }

    // Coverage: no dead states.
    let mut referenced: HashSet<String> = HashSet::new();

    for process in &mechanism.processes {
        for stoich in process.from.iter().chain(&process.to) {
            referenced.insert(stoich.state.clone());
        }
    }

    for algebraic in &mechanism.algebraic {
        collect_vars(&algebraic.expr, &mut referenced);
    }

    for observable in mechanism.all_observables() {
        for contribution in &observable.contributions {
            referenced.insert(contribution.target.clone());
        }
    }

    for state in &mechanism.states {
        let reference = format!("{}.{}", state.entity, state.name);

        require(
            &mut errors,
            referenced.contains(&reference),
            format!(
                "state {reference:?} is never referenced by a process, \
                 algebraic definition, or the observable — remove it",
            ),
        );
    }

    // Algebraic definitions must be acyclic; produce the lowering order.
    if errors.is_empty() {
        if let Err(message) = algebraic_order(mechanism) {
            errors.push(message);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors
            .iter()
            .enumerate()
            .map(|(index, error)| format!("{}. {error}", index + 1))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

struct ExprScope<'a> {
    state_refs: &'a HashSet<String>,
    algebraic: &'a HashSet<&'a str>,
    parameters: &'a HashSet<&'a str>,
    inputs: &'a [crate::lexicon::Channel],
}

fn walk_expr(expr: &Expr, scope: &ExprScope, context: &str, errors: &mut Vec<String>) {
    let require = |errors: &mut Vec<String>, ok: bool, message: String| {
        if !ok {
            errors.push(message);
        }
    };

    match expr {
        Expr::Var { name } => require(
            errors,
            scope.state_refs.contains(name) || scope.algebraic.contains(name.as_str()),
            format!("{context}: unknown variable {name:?} (use \"Entity.state\" or an algebraic name)"),
        ),
        Expr::Param { name } => require(
            errors,
            scope.parameters.contains(name.as_str()),
            format!("{context}: unknown parameter {name:?}"),
        ),
        Expr::Input { name } => require(
            errors,
            scope.inputs.iter().any(|channel| &channel.name == name),
            format!(
                "{context}: unknown input channel {name:?} (declared: {})",
                scope
                    .inputs
                    .iter()
                    .map(|channel| channel.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ),
        Expr::Const { value } => require(
            errors,
            value.is_finite(),
            format!("{context}: non-finite constant"),
        ),
        Expr::Sum { terms } | Expr::Product { factors: terms } => {
            for term in terms {
                walk_expr(term, scope, context, errors);
            }
        }
        Expr::Ratio { numerator, denominator }
        | Expr::Pow { base: numerator, exponent: denominator } => {
            walk_expr(numerator, scope, context, errors);
            walk_expr(denominator, scope, context, errors);
        }
        Expr::Neg { operand }
        | Expr::Exp { operand }
        | Expr::Ln { operand } => walk_expr(operand, scope, context, errors),
    }
}

/// Collect every `Var` name referenced by an expression.
fn collect_vars(expr: &Expr, into: &mut HashSet<String>) {
    match expr {
        Expr::Var { name } => {
            into.insert(name.clone());
        }
        Expr::Param { .. } | Expr::Input { .. } | Expr::Const { .. } => {}
        Expr::Sum { terms } | Expr::Product { factors: terms } => {
            for term in terms {
                collect_vars(term, into);
            }
        }
        Expr::Ratio { numerator, denominator }
        | Expr::Pow { base: numerator, exponent: denominator } => {
            collect_vars(numerator, into);
            collect_vars(denominator, into);
        }
        Expr::Neg { operand }
        | Expr::Exp { operand }
        | Expr::Ln { operand } => collect_vars(operand, into),
    }
}

/// Topological order of the algebraic definitions (dependencies first).
pub fn algebraic_order(mechanism: &Mechanism) -> Result<Vec<usize>, String> {
    let names: Vec<&str> = mechanism
        .algebraic
        .iter()
        .map(|algebraic| algebraic.name.as_str())
        .collect();

    let mut dependencies: Vec<HashSet<usize>> = vec![HashSet::new(); names.len()];

    for (index, algebraic) in mechanism.algebraic.iter().enumerate() {
        let mut vars = HashSet::new();

        collect_vars(&algebraic.expr, &mut vars);

        for (other, name) in names.iter().enumerate() {
            if other != index && vars.contains(*name) {
                dependencies[index].insert(other);
            }
        }
    }

    let mut order = Vec::with_capacity(names.len());

    let mut remaining: HashSet<usize> = (0..names.len()).collect();

    while !remaining.is_empty() {
        let ready: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|index| {
                dependencies[*index]
                    .iter()
                    .all(|dependency| !remaining.contains(dependency))
            })
            .collect();

        if ready.is_empty() {
            return Err(
                "algebraic definitions contain a dependency cycle".to_string(),
            );
        }

        for index in ready {
            remaining.remove(&index);

            order.push(index);
        }
    }

    Ok(order)
}

/// A copy of the mechanism with every collection in canonical order,
/// so two mechanisms that differ only in declaration order hash equal.
pub fn canonical(mechanism: &Mechanism) -> Mechanism {
    let mut sorted = mechanism.clone();

    sorted.compartments.sort();
    sorted.compartments.dedup();

    sorted.entities.sort_by(|a, b| a.name.cmp(&b.name));

    sorted.states.sort_by(|a, b| {
        (a.entity.as_str(), a.name.as_str()).cmp(&(b.entity.as_str(), b.name.as_str()))
    });

    sorted
        .parameters
        .sort_by(|a, b| a.name.cmp(&b.name));

    sorted.algebraic.sort_by(|a, b| a.name.cmp(&b.name));

    sorted.processes.sort_by_key(|process| {
        serde_json::to_string(process).unwrap_or_default()
    });

    if let Some(observable) = &mut sorted.observable {
        sort_contributions(&mut observable.contributions);
    }

    for observable in &mut sorted.observables {
        sort_contributions(&mut observable.contributions);
    }

    sorted.observables.sort_by(|a, b| a.channel.cmp(&b.channel));

    sorted
}

fn sort_contributions(contributions: &mut Vec<Contribution>) {
    contributions.sort_by(|a, b| {
        (a.target.as_str(), a.weight)
            .partial_cmp(&(b.target.as_str(), b.weight))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Content hash of the canonical form: the ledger's dedup key.
pub fn mechanism_hash(mechanism: &Mechanism) -> String {
    let canonical = canonical(mechanism);

    let text = serde_json::to_string(&canonical).unwrap_or_default();

    let mut hasher = DefaultHasher::new();

    text.hash(&mut hasher);

    format!("{:016x}", hasher.finish())
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();

    matches!(characters.next(), Some(first) if first.is_ascii_alphabetic())
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Names the lowerer emits or Modelica reserves; user namespaces
/// (parameters, algebraic variables) must avoid them. The declared
/// channel names (lexicon-specific — they become `input`/`output`
/// declarations) join the list at check time.
const RESERVED: &[&str] = &[
    "signal",
    "time",
    "der",
    "exp",
    "log",
    "model",
    "end",
    "equation",
    "parameter",
    "real",
    "input",
    "output",
    "initial",
    "when",
    "if",
    "then",
    "else",
    "and",
    "or",
    "not",
    "for",
    "while",
    "loop",
    "break",
    "return",
    "function",
    "algorithm",
    "constant",
    "discrete",
    "flow",
    "stream",
    "connector",
    "record",
    "block",
    "package",
    "type",
    "extends",
    "partial",
    "replaceable",
    "redeclare",
    "each",
    "inner",
    "outer",
    "assert",
    "connect",
];

fn is_reserved(name: &str, lexicon: &Lexicon) -> bool {
    let lowered = name.to_ascii_lowercase();

    lexicon
        .inputs
        .iter()
        .chain(&lexicon.outputs)
        .any(|channel| channel.name.to_ascii_lowercase() == lowered)
        || RESERVED.contains(&lowered.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lexicon() -> Lexicon {
        Lexicon {
            inputs: vec![crate::lexicon::Channel {
                name: "concentration".to_string(),
                units: "nM".to_string(),
                description: "analyte concentration".to_string(),
            }],
            outputs: vec![crate::lexicon::Channel {
                name: "signal".to_string(),
                units: "nm".to_string(),
                description: "BLI response shift".to_string(),
            }],
            roles: vec!["immobilized".to_string()],
            notes: String::new(),
        }
    }

    /// The shipped 1:1 baseline of the example pack.
    fn baseline() -> Mechanism {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("packs/binding/mechanism.json");

        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

        serde_json::from_str(&text).expect("baseline mechanism parses")
    }

    #[test]
    fn baseline_validates_and_hashes_stably() {
        let mechanism = baseline();

        validate(&mechanism, &lexicon()).expect("baseline mechanism is valid");

        let first = mechanism_hash(&mechanism);

        // The same mechanism with collections re-ordered hashes equal.
        let mut reordered = mechanism.clone();

        reordered.parameters.reverse();
        reordered.processes.reverse();
        reordered.states.reverse();

        assert_eq!(first, mechanism_hash(&reordered));

        // A structural change changes the hash.
        reordered.parameters.push(ParameterDecl {
            name: "k_extra".to_string(),
            start: 1.0,
            min: 0.0,
            max: 10.0,
        });

        assert_ne!(first, mechanism_hash(&reordered));
    }

    #[test]
    fn driven_rules_are_enforced() {
        let mut mechanism = baseline();

        // Second driven entity.
        mechanism.entities.push(Entity {
            name: "L2".to_string(),
            kind: EntityKind::Driven,
            compartment: None,
            driven_by: Some("concentration".to_string()),
            role: None,
        });

        assert!(validate(&mechanism, &lexicon()).is_err());

        let mut mechanism = baseline();

        // Driven without driven_by.
        mechanism.entities[0].driven_by = None;

        assert!(validate(&mechanism, &lexicon()).is_err());

        let mechanism = baseline();

        // The conservation idiom (parameter-dependent capacity via an
        // algebraic, not via an initial condition) must validate.
        assert!(validate(&mechanism, &lexicon()).is_ok());
    }

    #[test]
    fn unknown_references_are_rejected() {
        let mut mechanism = baseline();

        mechanism.processes[0].from[0].state = "P.missing".to_string();

        let message = validate(&mechanism, &lexicon()).unwrap_err();

        assert!(message.contains("unknown participant"), "{message}");

        let mut mechanism = baseline();

        mechanism.algebraic[0].expr = Expr::Param {
            name: "nope".to_string(),
        };

        let message = validate(&mechanism, &lexicon()).unwrap_err();

        assert!(message.contains("unknown parameter"), "{message}");
    }

    #[test]
    fn algebraic_participants_are_accepted() {
        // The baseline consumes the algebraic `P_free` as a reactant:
        // the conservation idiom only works if processes may reference
        // algebraic variables, not just states.
        let mechanism = baseline();

        let association = &mechanism.processes[0];

        assert!(
            association
                .from
                .iter()
                .any(|participant| participant.state == "P_free"),
            "baseline association should consume the P_free algebraic",
        );

        assert!(validate(&mechanism, &lexicon()).is_ok());

        // But a plain name that resolves to nothing is still rejected.
        let mut mechanism = baseline();

        mechanism.processes[0].from[0].state = "ghost_pool".to_string();

        assert!(validate(&mechanism, &lexicon()).is_err());
    }

    #[test]
    fn reserved_names_are_rejected() {
        let mut mechanism = baseline();

        mechanism.parameters.push(ParameterDecl {
            name: "time".to_string(),
            start: 1.0,
            min: 0.0,
            max: 2.0,
        });

        let message = validate(&mechanism, &lexicon()).unwrap_err();

        assert!(message.contains("reserved"), "{message}");

        let mut mechanism = baseline();

        mechanism.algebraic.push(Algebraic {
            name: "signal".to_string(),
            expr: Expr::Const { value: 1.0 },
        });

        assert!(validate(&mechanism, &lexicon()).is_err());

        // The driven input's name is reserved too.
        let mut mechanism = baseline();

        mechanism.parameters.push(ParameterDecl {
            name: "concentration".to_string(),
            start: 1.0,
            min: 0.0,
            max: 2.0,
        });

        assert!(validate(&mechanism, &lexicon()).is_err());
    }

    #[test]
    fn dead_states_are_rejected() {
        let mut mechanism = baseline();

        mechanism.states.push(State {
            entity: "PL".to_string(),
            name: "ghost".to_string(),
            initial: 0.0,
        });

        let message = validate(&mechanism, &lexicon()).unwrap_err();

        assert!(message.contains("never referenced"), "{message}");
    }

    #[test]
    fn algebraic_cycles_are_rejected() {
        let mut mechanism = baseline();

        // P_free = a; a = P_free.
        mechanism.algebraic[0].expr = Expr::Var {
            name: "a".to_string(),
        };

        mechanism.algebraic.push(Algebraic {
            name: "a".to_string(),
            expr: Expr::Var {
                name: "P_free".to_string(),
            },
        });

        let message = validate(&mechanism, &lexicon()).unwrap_err();

        assert!(message.contains("cycle"), "{message}");

        // Self-reference.
        let mut mechanism = baseline();

        mechanism.algebraic[0].expr = Expr::Var {
            name: "P_free".to_string(),
        };

        let message = validate(&mechanism, &lexicon()).unwrap_err();

        assert!(message.contains("itself"), "{message}");
    }

    #[test]
    fn algebraic_order_is_topological() {
        let mut mechanism = baseline();

        mechanism.algebraic.push(Algebraic {
            name: "second".to_string(),
            expr: Expr::Var {
                name: "P_free".to_string(),
            },
        });

        // P_free is first in the vector but second in dependency order.
        let second = mechanism.algebraic.pop().unwrap();

        mechanism.algebraic.insert(0, second);

        let order = algebraic_order(&mechanism).expect("acyclic");

        let names: Vec<&str> = order
            .iter()
            .map(|&index| mechanism.algebraic[index].name.as_str())
            .collect();

        assert_eq!(names, vec!["P_free", "second"]);
    }

    #[test]
    fn identifiers_are_checked() {
        assert!(is_identifier("Binding1to1"));
        assert!(is_identifier("kon_2"));
        assert!(!is_identifier("2kon"));
        assert!(!is_identifier("kon-off"));
        assert!(!is_identifier(""));
    }
}