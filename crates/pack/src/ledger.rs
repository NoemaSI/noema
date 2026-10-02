//! The research ledger: every idea, every verdict, one file.
//!
//! ```text
//! workspace/<pack>/ledger.jsonl
//! ```
//!
//! One JSON line per event. The loop's memory is not the LLM's
//! context window — it is this file. Each generation appends one
//! `evaluated` entry (the outcome of the implemented mechanism) and
//! one `proposed` entry per idea that was recorded but not tried, so
//! the theorist sees not just what happened but what was *deferred*,
//! and can resurrect an old idea instead of reinventing it.
//!
//! Everything here is author-safe: hypotheses, predictions, verdicts,
//! and visible-validation fitness only. Hidden-set scores never enter
//! the ledger (they live, sealed, in the generation reports), so the
//! ledger digest can be inlined into prompts verbatim.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How a generation chose to spend its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// A structurally new mechanism, screened on a small budget.
    Explore,
    /// A refit or refinement of the champion, at full budget.
    Exploit,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Explore => "explore",
            Mode::Exploit => "exploit",
        }
    }
}

/// What kind of event the entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The generation's implemented mechanism and its outcome.
    Evaluated,
    /// An idea the theorist proposed but that was not implemented.
    Proposed,
}

/// What the harness decided about the mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Became the new champion.
    Promoted,
    /// Evaluated but did not beat the champion.
    Rejected,
    /// Lowering, compilation, or simulation failed.
    Failed,
    /// Recorded for the future; never implemented (yet).
    Untested,
}

/// One research event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub generation: u32,

    /// Canonical mechanism hash — the dedup key across generations.
    pub mechanism_hash: String,

    pub role: Role,
    pub verdict: Verdict,
    pub mode: Mode,

    /// The author rationale (why this mechanism should fit better).
    pub hypothesis: String,

    /// What the theory predicts: which curve feature improves.
    #[serde(default)]
    pub prediction: Option<String>,

    /// What result would falsify the hypothesis.
    #[serde(default)]
    pub falsifier: Option<String>,

    /// The theorist's complexity estimate for the idea.
    #[serde(default)]
    pub cost: Option<String>,

    /// Report status when evaluated ("Success", "SimulationFailed").
    #[serde(default)]
    pub status: Option<String>,

    /// Visible-validation fitness when evaluated.
    #[serde(default)]
    pub fitness: Option<f64>,

    /// Fitness change versus the champion at decision time
    /// (negative = better, matching the loss convention).
    #[serde(default)]
    pub delta: Option<f64>,

    /// Diagnostics when the verdict is `failed`.
    #[serde(default)]
    pub error: Option<String>,

    /// Free-text annotation (e.g. why an idea was skipped).
    #[serde(default)]
    pub note: Option<String>,
}

/// Append one entry as a JSON line.
pub fn append(path: &Path, entry: &LedgerEntry) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let line = serde_json::to_string(entry)?;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;

    writeln!(file, "{line}").with_context(|| format!("appending {}", path.display()))
}

/// Read every entry; an absent ledger is an empty history.
pub fn read(path: &Path) -> Vec<LedgerEntry> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };

    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The author-safe history digest, oldest entry dropped first beyond
/// `limit`, inlined verbatim into theorist prompts.
pub fn digest(entries: &[LedgerEntry], limit: usize) -> String {
    let recent: Vec<&LedgerEntry> = entries
        .iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    if recent.is_empty() {
        return "(ledger empty: this is the first generation)".to_string();
    }

    recent
        .iter()
        .map(|entry| {
            let role = match entry.role {
                Role::Evaluated => "evaluated",
                Role::Proposed => "proposed",
            };

            let verdict = match entry.verdict {
                Verdict::Promoted => "PROMOTED",
                Verdict::Rejected => "rejected",
                Verdict::Failed => "FAILED",
                Verdict::Untested => "untested",
            };

            let outcome = match (entry.fitness, entry.delta) {
                (Some(fitness), Some(delta)) => {
                    format!(" fitness={:.6} delta={:+.6}", fitness, delta)
                }
                (Some(fitness), None) => format!(" fitness={:.6}", fitness),
                _ => String::new(),
            };

            let error = match &entry.error {
                Some(error) => {
                    let error = truncate(error, 600);

                    format!(" error={error:?}")
                }
                None => String::new(),
            };

            let note = match &entry.note {
                Some(note) => format!(" ({})", truncate(note, 120)),
                None => String::new(),
            };

            format!(
                "gen {} [{}] hash {} {} {}{}{}{} — {}",
                entry.generation,
                entry.mode.label(),
                short_hash(&entry.mechanism_hash),
                role,
                verdict,
                outcome,
                error,
                note,
                truncate(&entry.hypothesis, 200),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

pub fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }

    let truncated: String = text.chars().take(limit).collect();

    format!("{truncated}…")
}

/// Condense a raw evaluation failure into the lines an author (human
/// or theorist) can act on. Simulation failures arrive as nested
/// Debug-formatted strings wrapping a shell transcript: repeated
/// `\"`/`\n` escapes, a command wrapper, and a python traceback that
/// buries the solver's own diagnosis (the `NaN detected …` /
/// `IDA…` lines). The digest's character budget must not be spent on
/// that scaffolding, so it is stripped here, before anything is
/// stored or truncated.
pub fn sanitize(error: &str) -> String {
    // The error was Debug-formatted one or two levels deep: undo the
    // escape layers (each pass strictly shortens, so this terminates).
    let mut text = error.to_string();

    while text.contains("\\n") || text.contains("\\\"") {
        let unescaped = text.replace("\\n", "\n").replace("\\\"", "\"");

        if unescaped == text {
            break;
        }

        text = unescaped;
    }

    // Keep the subject prefix ("D: SimulationFailed…") if present.
    let prefix = match text.find("SimulationFailed") {
        Some(at) if at <= 32 => text[..at].trim_matches(|c| c == ':' || c == ' ').to_string(),
        _ => String::new(),
    };

    // The solver speaks after the wrapper's stderr marker, if any.
    let body = text.rsplit("--- stderr ---").next().unwrap_or(&text);

    let mut kept: Vec<&str> = Vec::new();

    for line in body.lines() {
        let line = line.trim();

        if line.is_empty()
            || line.starts_with("Traceback")
            || line.starts_with("File \"")
            || line.starts_with("--- stdout ---")
        {
            continue;
        }

        // Python frames are bare code lines between File entries.
        if line.starts_with("return ") || line.starts_with("result =") {
            continue;
        }

        let informative = ["WARNING", "NaN", "Error in", "IDA", "SUNDIALS", "residual", "failed"]
            .iter()
            .any(|keyword| line.contains(keyword));

        if informative && !kept.contains(&line) {
            kept.push(line);
        }
    }

    let condensed = if kept.is_empty() {
        // Nothing recognized (IR errors, "no train curves", …): the
        // first line already is the diagnosis.
        text.lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or(&text)
            .to_string()
    } else {
        kept.join(" | ")
    };

    if prefix.is_empty() || condensed.starts_with(&prefix) {
        condensed
    } else {
        format!("{prefix}: {condensed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(generation: u32, role: Role, verdict: Verdict) -> LedgerEntry {
        LedgerEntry {
            generation,
            mechanism_hash: "0123456789abcdef".to_string(),
            role,
            verdict,
            mode: Mode::Explore,
            hypothesis: "biphasic dissociation explains the slow tail".to_string(),
            prediction: Some("validation curves fit at 100 nM".to_string()),
            falsifier: Some("residuals still drift after 60 s".to_string()),
            cost: Some("one extra state".to_string()),
            status: None,
            fitness: None,
            delta: None,
            error: None,
            note: None,
        }
    }

    #[test]
    fn entries_round_trip_through_jsonl() {
        let path = std::env::temp_dir()
            .join("skill-ledger-test")
            .join("ledger.jsonl");

        let _ = std::fs::remove_file(&path);

        let mut evaluated = entry(3, Role::Evaluated, Verdict::Promoted);

        evaluated.status = Some("Success".to_string());
        evaluated.fitness = Some(0.42);
        evaluated.delta = Some(-0.03);

        append(&path, &evaluated).unwrap();

        let mut proposed = entry(3, Role::Proposed, Verdict::Untested);

        proposed.note = Some("deferred: second choice this generation".to_string());

        append(&path, &proposed).unwrap();

        let entries = read(&path);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].verdict, Verdict::Promoted);
        assert_eq!(entries[0].fitness, Some(0.42));
        assert_eq!(entries[1].role, Role::Proposed);
        assert_eq!(entries[1].fitness, None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reading_a_missing_ledger_is_empty_history() {
        let entries = read(&std::env::temp_dir().join("skill-ledger-does-not-exist.jsonl"));

        assert!(entries.is_empty());
    }

    #[test]
    fn digest_is_author_safe_and_bounded() {
        let entries: Vec<LedgerEntry> = (0..20)
            .map(|generation| {
                let mut evaluated = entry(generation, Role::Evaluated, Verdict::Rejected);

                evaluated.fitness = Some(1.0);
                evaluated.delta = Some(0.1);

                evaluated
            })
            .collect();

        let history = digest(&entries, 10);

        let lines: Vec<&str> = history.lines().collect();

        assert_eq!(lines.len(), 10);
        assert!(lines[0].contains("gen 10"));
        assert!(lines[9].contains("gen 19"));
        assert!(history.contains("rejected"));
        assert!(history.contains("biphasic dissociation"));

        assert_eq!(digest(&[], 5), "(ledger empty: this is the first generation)");
    }

    #[test]
    fn digest_truncates_long_text() {
        let mut evaluated = entry(1, Role::Evaluated, Verdict::Failed);

        evaluated.hypothesis = "x".repeat(500);
        evaluated.error = Some("y".repeat(700));

        let digest = digest(&[evaluated], 5);

        assert!(digest.contains('…'));
        assert!(digest.contains(&"y".repeat(600)), "error keeps its 600-char budget");
        assert!(!digest.contains(&"y".repeat(601)));
        assert!(digest.chars().count() < 1000);
    }
}

#[cfg(test)]
mod sanitize_tests {
    use super::sanitize;

    #[test]
    fn solver_diagnosis_survives_the_wrapper() {
        // Shape of a real casadi failure as stored in report.error:
        // nested Debug formatting, shell wrapper, python traceback.
        let raw = r#"D: SimulationFailed("fitted candidate does not replay: Failed(\"`python <casadi artifact trace simulation>` failed (exit Some(1))\n--- stdout ---\n\n--- stderr ---\nCasADi - 2026-10-01 10:29:40 WARNING(\"integrator:daeF failed: NaN detected for output alg, at (row 3, col 0).\") [.../casadi/core/oracle_function.cpp:408]\nThe residual function failed at the first call. \nTraceback (most recent call last):\n  File \"<string>\", line 109, in <module>\n    result = integrate(times[0], times, values_at(times[0]), model[\"x0\"], None)\n  File \"/opt/casadi.py\", line 24049, in __call__\n    return self.call(kwargs)\nRuntimeError: Error in Function::call for 'integrator' [IdasInterface]\n.../idas_interface.cpp:599: IDACalcIC returned \"IDA_FIRST_RES_FAIL\".\n\")")"#;

        let clean = sanitize(raw);

        assert!(clean.contains("NaN detected for output alg"), "{clean}");
        assert!(clean.contains("IDA_FIRST_RES_FAIL"), "{clean}");
        assert!(clean.starts_with("D:"), "{clean}");
        assert!(!clean.contains("Traceback"), "{clean}");
        assert!(!clean.contains("casadi.py"), "{clean}");
        assert!(clean.chars().count() < 400, "{clean}");
    }

    #[test]
    fn plain_errors_pass_through() {
        assert_eq!(
            sanitize("subject D has no train curves"),
            "subject D has no train curves",
        );
    }
}
