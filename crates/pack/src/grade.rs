//! `skill grade`: score a discovered mechanism against a known-world
//! ground truth (`toy/truth/*.json`, produced by the generators in
//! `toy/`). The truth file is deliberately not readable by the pack —
//! only this command and the human see it.
//!
//! The verdict is factual, not scientific: structure sizes (dynamic
//! states, parameters, processes) compared to truth, the champion's
//! honest generalization numbers, and the sentinel state. Whether the
//! discovered mechanism is the SAME mechanism is decided by reading
//! both equation sets side by side — the command prints exactly what
//! to compare.

use std::path::PathBuf;

use crate::mechanism_ir::Mechanism;
use crate::pack_loader::PackHandle;
use crate::workspace::Workspace;

/// `skill grade --truth=<truth.json> --pack=<pack.rs> [--gen=N]
/// [--subject=<id>…] [--workspace=<dir>]`
pub fn grade(handle: &PackHandle, args: &[String]) -> Result<(), String> {
    let flag = |prefix: &str| {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .map(str::to_string)
    };

    let truth_path = flag("--truth=").ok_or(
        "usage: skill grade --truth=<truth.json> --pack=<pack.rs> [--gen=N] [--subject=<id>…]",
    )?;

    let truth: serde_json::Value = {
        let text = std::fs::read_to_string(&truth_path)
            .map_err(|error| format!("reading {truth_path}: {error}"))?;

        serde_json::from_str(&text).map_err(|error| format!("parsing {truth_path}: {error}"))?
    };

    // Workspace lineage resolution, same rules as `skill report`.
    let mut subject_ids: Vec<String> = Vec::new();

    for arg in args {
        for prefix in ["--subject=", "--subjects="] {
            if let Some(value) = arg.strip_prefix(prefix) {
                subject_ids.extend(value.split(',').map(|id| id.trim().to_string()));
            }
        }
    }

    let workspace = if let Some(path) = flag("--workspace=").map(PathBuf::from) {
        Workspace::new(path)
    } else if !subject_ids.is_empty() {
        Workspace::new(
            crate::workspace_root()
                .join("workspace")
                .join(handle.pack.name())
                .join(subject_ids.join("+")),
        )
    } else {
        Workspace::new(
            crate::workspace_root()
                .join("workspace")
                .join(handle.pack.name()),
        )
    };

    let generation = match flag("--gen=").map(|text| text.parse::<u32>()) {
        Some(Ok(generation)) => Some(generation),
        Some(Err(_)) => return Err("`--gen=` wants a number".to_string()),
        None => workspace
            .champion()
            .map(|champion| champion.generation)
            .or_else(|| workspace.latest_gen()),
    }
    .ok_or_else(|| format!("no generations in {}", workspace.root().display()))?;

    let mechanism_path = workspace.mechanism_path(generation);

    let text = std::fs::read_to_string(&mechanism_path)
        .map_err(|error| format!("reading {}: {error}", mechanism_path.display()))?;

    let mechanism: Mechanism = serde_json::from_str(&text)
        .map_err(|error| format!("parsing {}: {error}", mechanism_path.display()))?;

    let report = workspace.read_report(generation);

    // Truth facts.
    let structural = &truth["structural"];
    let truth_states = structural["dynamic_states"].as_u64().unwrap_or(0);
    let truth_parameters = structural["parameters"].as_u64().unwrap_or(0);

    println!(
        "skill: world `{}` tier {} — truth: {truth_states} dynamic states, \
         {truth_parameters} parameters",
        truth["world"].as_str().unwrap_or("?"),
        truth["tier"].as_str().unwrap_or("?"),
    );

    if let Some(equations) = truth["equations"].as_str() {
        println!("skill:   truth equations: {equations}");
    }

    if let Some(signature) = truth["signature"].as_str() {
        println!("skill:   truth signature: {signature}");
    }

    // Discovered facts.
    let driven: Vec<&str> = mechanism
        .entities
        .iter()
        .filter(|entity| entity.kind == crate::mechanism_ir::EntityKind::Driven)
        .map(|entity| entity.name.as_str())
        .collect();

    let dynamic_states: Vec<String> = mechanism
        .states
        .iter()
        .filter(|state| {
            mechanism
                .entities
                .iter()
                .find(|entity| entity.name == state.entity)
                .is_none_or(|entity| entity.kind != crate::mechanism_ir::EntityKind::Driven)
        })
        .map(|state| format!("{}.{}", state.entity, state.name))
        .collect();

    println!(
        "skill: champion gen {generation:04} mechanism `{}` — {} dynamic states ({}), \
         {} parameters, {} processes, {} algebraic, driven: {}",
        mechanism.name,
        dynamic_states.len(),
        dynamic_states.join(", "),
        mechanism.parameters.len(),
        mechanism.processes.len(),
        mechanism.algebraic.len(),
        if driven.is_empty() { "none".to_string() } else { driven.join(", ") },
    );

    println!(
        "skill:   parameters: {}",
        mechanism
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    );

    if let Some(report) = &report {
        if let Some(fitness) = report.fitness {
            println!(
                "skill:   honest numbers: fitness {fitness:.4} | hidden rmse {}",
                report
                    .evaluator_only
                    .hidden_rmse
                    .map(|hidden| format!("{hidden:.4}"))
                    .unwrap_or_else(|| "—".to_string()),
            );
        }

        let pinned: Vec<&String> = report
            .per_subject
            .iter()
            .flat_map(|subject| subject.pinned.iter())
            .collect();

        if !pinned.is_empty() {
            println!("skill:   sentinels: {} pinned parameter(s)", pinned.len());
        }
    }

    // Factual verdict: sizes, not science.
    let verdict = match dynamic_states.len() as u64 {
        n if n == truth_states => "state count MATCHES truth — read both equation sets",
        n if n < truth_states => "COARSER than truth — a hidden compartment is still merged",
        _ => "FINER than truth — states were split or added beyond the truth",
    };

    println!("skill: VERDICT: {verdict}");
    println!("skill:   compare by hand: {truth_path} vs {}", mechanism_path.display());

    Ok(())
}