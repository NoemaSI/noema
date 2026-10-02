//! Lowering: the mechanism IR compiled to Modelica and a plugin.
//!
//! The theorist authors [`crate::mechanism_ir::Mechanism`] JSON; this
//! module is the only translator between that idea and executable
//! reality. It is deterministic and total: the same mechanism always
//! lowers to the same Modelica text and the same generated plugin, and
//! nothing here improvises. The LLM writes only the IR and this module
//! writes everything else.
//!
//! Lowering rules (the contract the theorist prompt documents):
//!
//! - parameters keep declaration order: that order **is** the theta
//!   order the fixed optimizer fits;
//! - the driven entity's state is the experiment's drive for the
//!   channel it names — it lowers to that declared `input Real
//!   <channel>` directly and never gains a `der` (a reservoir is not
//!   a state of the sensor);
//! - every other state becomes `Real E_s(start=…)` plus a `der`
//!   equation accumulated from process fluxes: products gain
//!   `+ flux * stoich`, reactants lose `- flux * stoich`;
//! - algebraic variables are emitted in dependency order
//!   ([`crate::mechanism_ir::algebraic_order`]) — the conservation
//!   idiom (`P_free = rmax - PL.bound`) lives here;
//! - each observable declares `output Real <channel>` for its
//!   declared output channel: the weighted contributions, optionally
//!   scaled by a parameter and optionally carrying a drift term in
//!   `time`.

use crate::lexicon::Lexicon;
use crate::mechanism::Complexity;
use crate::mechanism_ir::{
    self, Algebraic, Contribution, DriftSpec, Expr, Mechanism, Observable, Process, RateLaw,
    State,
};

/// The compile products of one mechanism.
#[derive(Debug, Clone)]
pub struct Lowered {
    /// The Modelica model (what Rumoca compiles).
    pub modelica: String,

    /// The generated plugin source exposing the model through the
    /// `CandidateSpec` contract.
    pub plugin_source: String,

    /// Logged, never scored.
    pub complexity: Complexity,
}

/// Validate and lower a mechanism. `note` is the author rationale
/// carried into the generated plugin (logged, never scored).
pub fn lower(
    mechanism: &Mechanism,
    lexicon: &Lexicon,
    note: &str,
) -> Result<Lowered, String> {
    mechanism_ir::validate(mechanism, lexicon)?;

    let modelica = render_modelica(mechanism, lexicon);

    let plugin_source = render_plugin(&modelica, &mechanism.name, lexicon, note);

    let complexity = Complexity {
        states: mechanism
            .states
            .iter()
            .filter(|state| !is_driven(mechanism, &state.entity))
            .count(),
        parameters: mechanism.parameters.len(),
        equations: mechanism.processes.len() + mechanism.algebraic.len() + 1,
    };

    Ok(Lowered {
        modelica,
        plugin_source,
        complexity,
    })
}

fn is_driven(mechanism: &Mechanism, entity: &str) -> bool {
    mechanism
        .entities
        .iter()
        .any(|candidate| candidate.name == entity && candidate.is_driven())
}

fn render_modelica(mechanism: &Mechanism, lexicon: &Lexicon) -> String {
    let mut lines = Vec::new();

    lines.push(format!("model {}", mechanism.name));

    // Declared input channels the mechanism actually references (the
    // driven entity's reservoir plus every `input` expression), in
    // lexicon order.
    let used = used_channels(mechanism);

    for channel in lexicon
        .inputs
        .iter()
        .filter(|channel| used.contains(&channel.name))
    {
        lines.push(format!("  input Real {};", channel.name));
    }

    for observable in mechanism.all_observables() {
        lines.push(format!("  output Real {};", observable.channel));
    }

    lines.push(String::new());

    for parameter in &mechanism.parameters {
        lines.push(format!(
            "  parameter Real {}(start={}, min={}, max={});",
            parameter.name,
            num(parameter.start),
            num(parameter.min),
            num(parameter.max),
        ));
    }

    if !mechanism.parameters.is_empty() {
        lines.push(String::new());
    }

    // Dynamic states only: the driven entity is the input reservoir
    // and lowers to the input, not to a variable.
    let dynamic: Vec<&State> = mechanism
        .states
        .iter()
        .filter(|state| !is_driven(mechanism, &state.entity))
        .collect();

    for state in &dynamic {
        lines.push(format!(
            "  Real {}_{}(start={});",
            state.entity,
            state.name,
            num(state.initial),
        ));
    }

    if !dynamic.is_empty() {
        lines.push(String::new());
    }

    let order = mechanism_ir::algebraic_order(mechanism).unwrap_or_default();

    let ordered: Vec<&Algebraic> = order
        .iter()
        .map(|&index| &mechanism.algebraic[index])
        .collect();

    for algebraic in &ordered {
        lines.push(format!("  Real {};", algebraic.name));
    }

    if !ordered.is_empty() {
        lines.push(String::new());
    }

    lines.push("equation".to_string());

    for algebraic in &ordered {
        lines.push(format!(
            "  {} = {};",
            algebraic.name,
            render_expr(&algebraic.expr, mechanism, lexicon),
        ));
    }

    // Flux accumulation: every dynamic state sums its gains and
    // losses across all processes.
    for state in &dynamic {
        let reference = format!("{}.{}", state.entity, state.name);

        let mut terms: Vec<(bool, String)> = Vec::new();

        for process in &mechanism.processes {
            for participant in &process.from {
                if participant.state == reference {
                    terms.push((false, flux(process, mechanism, lexicon)));
                }
            }

            for participant in &process.to {
                if participant.state == reference {
                    terms.push((true, flux(process, mechanism, lexicon)));
                }
            }
        }

        let equation = if terms.is_empty() {
            "0".to_string()
        } else {
            join_terms(&terms)
        };

        lines.push(format!(
            "  der({}_{}) = {};",
            state.entity, state.name, equation,
        ));
    }

    for observable in mechanism.all_observables() {
        lines.push(format!(
            "  {} = {};",
            observable.channel,
            render_signal(observable, mechanism, lexicon),
        ));
    }

    lines.push(format!("end {};", mechanism.name));

    lines.join("\n")
}

/// The input channels a mechanism references: the driven entity's
/// reservoir channel and every `input` expression.
fn used_channels(mechanism: &Mechanism) -> Vec<String> {
    let mut used = Vec::new();

    let push = |name: &str, used: &mut Vec<String>| {
        if !used.iter().any(|used| used == name) {
            used.push(name.to_string());
        }
    };

    fn walk(expr: &Expr, used: &mut Vec<String>) {
        match expr {
            Expr::Input { name } => {
                if !used.iter().any(|used| used == name) {
                    used.push(name.clone());
                }
            }
            Expr::Var { .. } | Expr::Param { .. } | Expr::Const { .. } => {}
            Expr::Sum { terms } | Expr::Product { factors: terms } => {
                for term in terms {
                    walk(term, used);
                }
            }
            Expr::Ratio {
                numerator,
                denominator,
            }
            | Expr::Pow {
                base: numerator,
                exponent: denominator,
            } => {
                walk(numerator, used);
                walk(denominator, used);
            }
            Expr::Neg { operand } | Expr::Exp { operand } | Expr::Ln { operand } => {
                walk(operand, used);
            }
        }
    }

    for entity in &mechanism.entities {
        if entity.is_driven() {
            if let Some(channel) = &entity.driven_by {
                push(channel, &mut used);
            }
        }
    }

    for algebraic in &mechanism.algebraic {
        walk(&algebraic.expr, &mut used);
    }

    for process in &mechanism.processes {
        if let RateLaw::Custom { expr } = &process.rate {
            walk(expr, &mut used);
        }
    }

    used
}

fn render_signal(observable: &Observable, mechanism: &Mechanism, lexicon: &Lexicon) -> String {
    let mut terms: Vec<(bool, String)> = Vec::new();

    for contribution in &observable.contributions {
        terms.push((
            contribution.weight >= 0.0,
            contribution_term(contribution, Some(mechanism), Some(lexicon)),
        ));
    }

    let mut signal = join_terms(&terms);

    if let Some(scale) = &observable.scale {
        signal = format!("({}) * {}", signal, scale);
    }

    if let Some(drift) = &observable.drift {
        signal = format!("{} + {}", signal, render_drift(drift));
    }

    signal
}

fn contribution_term(
    contribution: &Contribution,
    mechanism: Option<&Mechanism>,
    lexicon: Option<&Lexicon>,
) -> String {
    let target = render_target(&contribution.target, mechanism, lexicon);

    let magnitude = contribution.weight.abs();

    if magnitude == 1.0 {
        target
    } else {
        format!("{} * {}", num(magnitude), target)
    }
}

/// A contribution target or a process participant, rendered as the
/// Modelica variable it denotes.
fn render_target(
    reference: &str,
    mechanism: Option<&Mechanism>,
    lexicon: Option<&Lexicon>,
) -> String {
    if let (Some(mechanism), Some(_lexicon)) = (mechanism, lexicon) {
        // A driven state *is* the input channel it names.
        let entity = reference.split('.').next().unwrap_or_default();

        let driven = mechanism
            .entities
            .iter()
            .find(|candidate| candidate.name == entity && candidate.is_driven());

        if let Some(driven) = driven {
            return driven
                .driven_by
                .clone()
                .unwrap_or_else(|| _lexicon.primary_input().name.clone());
        }
    }

    mechanism_ir::state_var_name(reference)
}

fn render_drift(drift: &DriftSpec) -> String {
    match drift {
        DriftSpec::Offset { parameter } => parameter.clone(),
        DriftSpec::Linear { rate } => format!("{} * time", rate),
        DriftSpec::Exponential { amplitude, tau } => {
            format!("{} * (1 - exp(-time / {}))", amplitude, tau)
        }
    }
}

/// The flux expression of a process, in amount/time.
fn flux(process: &Process, mechanism: &Mechanism, lexicon: &Lexicon) -> String {
    match &process.rate {
        RateLaw::MassAction { parameter } => {
            let mut factors = vec![parameter.clone()];

            for participant in &process.from {
                let base = render_target(&participant.state, Some(mechanism), Some(lexicon));

                let stoich = participant.stoich;

                if stoich == 1.0 {
                    factors.push(base);
                } else {
                    factors.push(format!("{} ^ {}", base, exponent(stoich)));
                }
            }

            factors.join(" * ")
        }
        RateLaw::Custom { expr } => render_expr(expr, mechanism, lexicon),
    }
}

fn exponent(stoich: f64) -> String {
    if stoich == stoich.trunc() {
        num(stoich)
    } else {
        format!("({})", num(stoich))
    }
}

/// Signed terms joined into one expression: `a - b + c`. A leading
/// negative keeps its minus.
fn join_terms(terms: &[(bool, String)]) -> String {
    let mut text = String::new();

    for (index, (positive, term)) in terms.iter().enumerate() {
        if index == 0 {
            if *positive {
                text.push_str(term);
            } else {
                text.push_str(&format!("-{}", term));
            }
        } else {
            text.push_str(if *positive { " + " } else { " - " });
            text.push_str(term);
        }
    }

    text
}

/// Expressions whose top level is an operator; as operands of other
/// operators they need parentheses.
fn is_compound(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Sum { .. }
            | Expr::Product { .. }
            | Expr::Ratio { .. }
            | Expr::Pow { .. }
            | Expr::Neg { .. }
    )
}

fn render_atom(expr: &Expr, mechanism: &Mechanism, lexicon: &Lexicon) -> String {
    if is_compound(expr) {
        format!("({})", render_expr(expr, mechanism, lexicon))
    } else {
        render_expr(expr, mechanism, lexicon)
    }
}

fn render_expr(expr: &Expr, mechanism: &Mechanism, lexicon: &Lexicon) -> String {
    match expr {
        Expr::Var { name } => render_target(name, Some(mechanism), Some(lexicon)),
        Expr::Param { name } => name.clone(),
        Expr::Input { name } => name.clone(),
        Expr::Const { value } => num(*value),
        Expr::Sum { terms } => {
            if terms.is_empty() {
                return "0".to_string();
            }

            let mut text = String::new();

            for (index, term) in terms.iter().enumerate() {
                match term {
                    Expr::Neg { operand } => {
                        let rendered = render_atom(operand, mechanism, lexicon);

                        if index == 0 {
                            text.push_str(&format!("-{}", rendered));
                        } else {
                            text.push_str(&format!(" - {}", rendered));
                        }
                    }
                    _ => {
                        let rendered = render_atom(term, mechanism, lexicon);

                        if index == 0 {
                            text.push_str(&rendered);
                        } else {
                            text.push_str(&format!(" + {}", rendered));
                        }
                    }
                }
            }

            text
        }
        Expr::Product { factors } => {
            if factors.is_empty() {
                return "1".to_string();
            }

            let rendered: Vec<String> = factors
                .iter()
                .map(|factor| render_atom(factor, mechanism, lexicon))
                .collect();

            rendered.join(" * ")
        }
        Expr::Ratio {
            numerator,
            denominator,
        } => format!(
            "{} / {}",
            render_atom(numerator, mechanism, lexicon),
            render_atom(denominator, mechanism, lexicon),
        ),
        Expr::Neg { operand } => format!("-{}", render_atom(operand, mechanism, lexicon)),
        Expr::Pow { base, exponent } => format!(
            "{} ^ {}",
            render_atom(base, mechanism, lexicon),
            render_atom(exponent, mechanism, lexicon),
        ),
        Expr::Exp { operand } => {
            format!("exp({})", render_expr(operand, mechanism, lexicon))
        }
        Expr::Ln { operand } => {
            format!("log({})", render_expr(operand, mechanism, lexicon))
        }
    }
}

/// Modelica number formatting: exact, deterministic, no trailing
/// `.0` noise — `0`, `100`, `1e-3`.
pub fn num(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }

    if value.is_finite() && value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }

    format!("{:e}", value)
}

fn render_plugin(modelica: &str, name: &str, lexicon: &Lexicon, note: &str) -> String {
    let note_literal = format!("{:?}", note);

    PLUGIN_TEMPLATE
        .replace("@MODELICA@", modelica)
        .replace("@NAME@", name)
        .replace("@INPUT@", &lexicon.primary_input().name)
        .replace("@INPUT_UNITS@", &lexicon.primary_input().units)
        .replace("@OUTPUT@", &lexicon.primary_output().name)
        .replace("@OUTPUT_UNITS@", &lexicon.primary_output().units)
        .replace("@NOTE@", &note_literal)
}

const PLUGIN_TEMPLATE: &str = r###"//! GENERATED FILE — do not edit by hand.
//!
//! Lowered deterministically from this generation's `mechanism.json`
//! (the mechanism IR) by `skill::lower`. The IR is the author's
//! canvas; this plugin is its compilation — to change the mechanism,
//! change the IR, and the harness re-lowers it.
//!
//! The contract is exactly one function:
//!
//! ```ignore
//! pub fn make_candidate() -> Box<dyn skill::candidate::CandidateSpec>
//! ```
//!
//! Boundaries (enforced by the harness): the external interface is
//! fixed by the pack's declared channels — `input Real` for each
//! drive channel (the primary one is `@INPUT@`, @INPUT_UNITS@, driven
//! by recorded piecewise-constant traces the harness replays) and
//! `output Real @OUTPUT@` (the measured response, @OUTPUT_UNITS@) —
//! and the harness compiles this model with Rumoca, fits the declared
//! parameters against the visible curves only, and scores the held-out
//! conditions evaluator-side.

use skill::candidate::CandidateSpec;

/// The lowered mechanism. The model identifier equals
/// `CandidateSpec::name()`; parameters carry the declared bounds the
/// optimizer fits within.
const MECHANISM: &str = r##"@MODELICA@"##;

pub struct Candidate;

impl CandidateSpec for Candidate {
    fn name(&self) -> String {
        "@NAME@".to_string()
    }

    fn modelica(&self) -> String {
        MECHANISM.to_string()
    }

    fn note(&self) -> Option<String> {
        Some(@NOTE@.to_string())
    }
}

pub fn make_candidate() -> Box<dyn CandidateSpec> {
    Box::new(Candidate)
}
"###;

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

    fn baseline() -> Mechanism {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("packs/binding/mechanism.json");

        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

        serde_json::from_str(&text).expect("baseline mechanism parses")
    }

    #[test]
    fn baseline_lowers_to_the_shipped_ode() {
        let mechanism = baseline();

        let lowered = lower(&mechanism, &lexicon(), "seed note").expect("baseline lowers");

        // The conservation idiom and both flux terms, in one der.
        assert!(
            lowered.modelica.contains(
                "der(PL_bound) = kon * P_free * concentration - koff * PL_bound",
            ),
            "unexpected lowering:\n{}",
            lowered.modelica,
        );

        assert!(lowered.modelica.contains("P_free = rmax - PL_bound;"));
        assert!(lowered.modelica.contains("signal = PL_bound;"));
        assert!(lowered.modelica.contains("Real PL_bound(start=0);"));
        assert!(lowered
            .modelica
            .contains("parameter Real kon(start=1e-3, min=1e-12, max=1000000);"));

        // The driven reservoir never becomes a variable.
        assert!(!lowered.modelica.contains("L_free"));

        // Theta order is declaration order.
        let kon_position = lowered.modelica.find("parameter Real kon").unwrap();
        let koff_position = lowered.modelica.find("parameter Real koff").unwrap();
        let rmax_position = lowered.modelica.find("parameter Real rmax").unwrap();

        assert!(kon_position < koff_position && koff_position < rmax_position);

        assert_eq!(
            lowered.complexity,
            Complexity {
                states: 1,
                parameters: 3,
                equations: 4,
            },
        );
    }

    #[test]
    fn plugin_source_exposes_the_candidate_contract() {
        let lowered = lower(&baseline(), &lexicon(), "seed note").expect("baseline lowers");

        assert!(lowered
            .plugin_source
            .contains("pub fn make_candidate() -> Box<dyn CandidateSpec>"));
        assert!(lowered.plugin_source.contains("impl CandidateSpec for Candidate"));
        assert!(lowered.plugin_source.contains("#\"model Binding1to1"));
        assert!(lowered.plugin_source.contains("\"Binding1to1\".to_string()"));
        assert!(lowered
            .plugin_source
            .contains("Some(\"seed note\".to_string())"));

        // The note is a Rust string literal: quotes escaped.
        let quoted = lower(&baseline(), &lexicon(), "note with \"quotes\"").unwrap();

        assert!(quoted.plugin_source.contains("note with \\\"quotes\\\""));
    }

    #[test]
    fn custom_rate_laws_and_drift_render() {
        let mut mechanism = baseline();

        let observable = mechanism.observable.as_mut().expect("baseline observable");

        observable.drift = Some(DriftSpec::Exponential {
            amplitude: "drift_amp".to_string(),
            tau: "drift_tau".to_string(),
        });

        observable.scale = Some("rmax".to_string());

        mechanism.parameters.push(mechanism_ir::ParameterDecl {
            name: "drift_amp".to_string(),
            start: 0.1,
            min: -1.0,
            max: 1.0,
        });

        mechanism.parameters.push(mechanism_ir::ParameterDecl {
            name: "drift_tau".to_string(),
            start: 30.0,
            min: 1e-3,
            max: 1e4,
        });

        let lowered = lower(&mechanism, &lexicon(), "drift").expect("lowers");

        assert!(
            lowered
                .modelica
                .contains("signal = (PL_bound) * rmax + drift_amp * (1 - exp(-time / drift_tau));"),
            "unexpected signal:\n{}",
            lowered.modelica,
        );
    }

    #[test]
    fn custom_flux_and_expression_forms_render() {
        let mut mechanism = baseline();

        mechanism.processes[0].rate = mechanism_ir::RateLaw::Custom {
            expr: Expr::Product {
                factors: vec![
                    Expr::Param { name: "kon".to_string() },
                    Expr::Var { name: "P_free".to_string() },
                    Expr::Pow {
                        base: Box::new(Expr::Var { name: "L.free".to_string() }),
                        exponent: Box::new(Expr::Const { value: 0.5 }),
                    },
                ],
            },
        };

        mechanism.algebraic.push(Algebraic {
            name: "half_conc".to_string(),
            expr: Expr::Ratio {
                numerator: Box::new(Expr::Input {
                    name: "concentration".to_string(),
                }),
                denominator: Box::new(Expr::Const { value: 2.0 }),
            },
        });

        let lowered = lower(&mechanism, &lexicon(), "custom").expect("lowers");

        assert!(
            lowered
                .modelica
                .contains("der(PL_bound) = kon * P_free * (concentration ^ 5e-1) - koff * PL_bound"),
            "unexpected flux:\n{}",
            lowered.modelica,
        );

        assert!(lowered.modelica.contains("half_conc = concentration / 2;"));
    }

    #[test]
    fn algebraic_definitions_are_emitted_in_dependency_order() {
        let mut mechanism = baseline();

        // Declared in reverse dependency order; lowering must flip.
        mechanism.algebraic.insert(
            0,
            Algebraic {
                name: "twice_free".to_string(),
                expr: Expr::Product {
                    factors: vec![
                        Expr::Const { value: 2.0 },
                        Expr::Var { name: "P_free".to_string() },
                    ],
                },
            },
        );

        let lowered = lower(&mechanism, &lexicon(), "order").expect("lowers");

        let p_free = lowered.modelica.find("P_free = rmax - PL_bound;").unwrap();

        let twice = lowered.modelica.find("twice_free = ").unwrap();

        assert!(p_free < twice, "dependency must lower first");
    }

    #[test]
    fn invalid_mechanisms_are_rejected_before_rendering() {
        let mut mechanism = baseline();

        mechanism.parameters.clear();

        let message = lower(&mechanism, &lexicon(), "x").unwrap_err();

        assert!(message.contains("at least one parameter"), "{message}");
    }

    #[test]
    fn num_formats_deterministically() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(100.0), "100");
        assert_eq!(num(-3.0), "-3");
        assert_eq!(num(1e-3), "1e-3");
        assert_eq!(num(1e-12), "1e-12");
        assert_eq!(num(1e6), "1000000");
        assert_eq!(num(0.5), "5e-1");
    }

    #[test]
    fn dead_state_lowering_is_still_valid_modelica() {
        // A state referenced only by the observable lowers to
        // `der(...) = 0` — a constant offset the optimizer cannot move
        // (initials are compile-time), which is exactly the IR
        // contract's honest answer.
        let mut mechanism = baseline();

        mechanism.states.push(State {
            entity: "PL".to_string(),
            name: "offset".to_string(),
            initial: 0.0,
        });

        mechanism
            .observable
            .as_mut()
            .expect("baseline observable")
            .contributions
            .push(Contribution {
                target: "PL.offset".to_string(),
                weight: 1.0,
            });

        let lowered = lower(&mechanism, &lexicon(), "offset").expect("lowers");

        assert!(lowered.modelica.contains("der(PL_offset) = 0;"));
        assert!(lowered.modelica.contains("signal = PL_bound + PL_offset;"));
    }
}