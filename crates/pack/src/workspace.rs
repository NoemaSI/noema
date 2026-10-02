//! The evolution workspace: generations on disk.
//!
//! ```text
//! workspace/<pack>/
//!   gen/0000/ mechanism.json  plugin.rs  report.json
//!   gen/0001/ mechanism.json  plugin.rs  report.json
//!   champion.json
//!   ledger.jsonl
//! ```
//!
//! Every generation is a directory holding the exact mechanism idea
//! (`mechanism.json`, the [`crate::mechanism_ir`] input) plus the
//! plugin source lowered from it, with the report that judged it
//! beside it — the lineage is inspectable without any database.
//! `champion.json` names the best generation so far; on a plateau the
//! next generation copies the champion again and the counter still
//! advances. `ledger.jsonl` (see [`crate::ledger`]) records every
//! proposal and verdict across generations.
//!
//! Reports carry author-safe fields plus an `evaluator_only` section
//! (hidden-set scores). The evolution context ([`crate::evolve`]) is
//! built from the author-safe fields; `evaluator_only` is stripped.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::mechanism::Complexity;

/// Evaluation of one subject inside a generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectReport {
    pub subject: String,

    /// "Success", "SimulationFailed", ...
    pub status: String,

    pub theta: Vec<(String, f64)>,
    pub train_loss: f64,

    /// Selection component (visible validation).
    pub validation_rmse: f64,

    /// Evaluator-only: hidden-set RMSE for this subject.
    pub hidden_rmse: f64,

    /// Sentinel: parameters stuck at their declared bounds.
    #[serde(default)]
    pub pinned: Vec<String>,

    /// Sentinel: model condition-response spread / data
    /// condition-response spread on validation levels (worst channel).
    #[serde(default)]
    pub modulation: Option<f64>,
}

/// Structure-transfer score for one sealed holdout subject: the
/// generation's mechanism was discovered without this subject's data;
/// the evaluator alone fits its parameters from the subject's train
/// input levels and scores the prediction on the withheld levels.
/// Never enters any author-facing structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoldoutScore {
    pub subject: String,

    /// "Success", "SimulationFailed", ...
    pub status: String,

    pub theta: Vec<(String, f64)>,

    /// Raw RMSE on the holdout's validation input levels.
    pub validation_rmse: f64,

    /// Validation RMSE normalized by the holdout's train-signal
    /// range — comparable across subjects and with the discovery
    /// fitness.
    pub normalized_validation_rmse: f64,

    /// Raw RMSE on the holdout's hidden input levels.
    pub hidden_rmse: f64,
}

/// Scores only the evaluator may read.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvaluatorOnly {
    pub hidden_rmse: Option<f64>,

    /// Evaluator-only: transfer to a sealed holdout subject
    /// (see [`HoldoutScore`]).
    #[serde(default)]
    pub holdout: Option<HoldoutScore>,
}

/// The complete outcome of evaluating one generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenReport {
    pub generation: u32,
    pub parent: Option<u32>,

    /// Content hash of the plugin source that produced this report.
    pub source_hash: String,

    /// Author rationale from the spec (logged, never scored).
    pub note: Option<String>,

    /// "Success", "CompileFailed", "SimulationFailed".
    pub status: String,

    /// Real diagnostics when the status is a failure.
    pub error: Option<String>,

    pub complexity: Option<Complexity>,

    /// Selection fitness: mean validation RMSE over successful
    /// subjects, each normalized by that subject's train-signal
    /// range; `None` unless every subject succeeded.
    pub fitness: Option<f64>,

    pub per_subject: Vec<SubjectReport>,

    pub evaluator_only: EvaluatorOnly,
}

/// The currently promoted generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Champion {
    pub generation: u32,
    pub fitness: f64,
}

pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn gen_dir(&self, generation: u32) -> PathBuf {
        self.root.join("gen").join(format!("{generation:04}"))
    }

    pub fn plugin_path(&self, generation: u32) -> PathBuf {
        self.gen_dir(generation).join("plugin.rs")
    }

    pub fn mechanism_path(&self, generation: u32) -> PathBuf {
        self.gen_dir(generation).join("mechanism.json")
    }

    fn report_path(&self, generation: u32) -> PathBuf {
        self.gen_dir(generation).join("report.json")
    }

    fn champion_path(&self) -> PathBuf {
        self.root.join("champion.json")
    }

    /// The research ledger ([`crate::ledger`]): one jsonl at the
    /// workspace root, shared by every generation.
    pub fn ledger_path(&self) -> PathBuf {
        self.root.join("ledger.jsonl")
    }

    /// Highest existing generation, or 0 when the workspace is empty.
    pub fn latest_gen(&self) -> Option<u32> {
        let entries = std::fs::read_dir(self.root.join("gen")).ok()?;

        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;

                if name.len() == 4 {
                    name.parse().ok()
                } else {
                    None
                }
            })
            .max()
    }

    pub fn has_generations(&self) -> bool {
        self.latest_gen().is_some()
    }

    /// Copy the baseline plugin into `gen/0000` unless generations
    /// already exist. Returns the seeded generation.
    pub fn seed(&self, baseline_plugin: &Path) -> Result<u32> {
        if let Some(latest) = self.latest_gen() {
            return Ok(latest);
        }

        let dir = self.gen_dir(0);

        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;

        std::fs::copy(baseline_plugin, dir.join("plugin.rs"))
            .with_context(|| format!("seeding gen/0000 from {}", baseline_plugin.display()))?;

        Ok(0)
    }

    /// Copy the baseline mechanism into `gen/0000` unless generations
    /// already exist. Returns the seeded generation.
    pub fn seed_mechanism(&self, baseline_mechanism: &Path) -> Result<u32> {
        if let Some(latest) = self.latest_gen() {
            // A gen-0 directory without its mechanism is a torn seed
            // (a run that died between creating the dir and copying
            // the file) — re-seed rather than trusting it.
            if latest > 0 || self.mechanism_path(0).exists() {
                return Ok(latest);
            }
        }

        let dir = self.gen_dir(0);

        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;

        std::fs::copy(baseline_mechanism, self.mechanism_path(0)).with_context(|| {
            format!("seeding gen/0000 from {}", baseline_mechanism.display())
        })?;

        Ok(0)
    }

    /// Copy the champion's plugin source into a fresh generation
    /// directory, returning its path (the file the LLM will edit).
    pub fn copy_champion_into(&self, champion: u32, generation: u32) -> Result<PathBuf> {
        let source = self.plugin_path(champion);

        let dir = self.gen_dir(generation);

        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;

        let target = dir.join("plugin.rs");

        std::fs::copy(&source, &target)
            .with_context(|| format!("copying {} -> {}", source.display(), target.display()))?;

        Ok(target)
    }

    /// Copy the champion's mechanism idea into a fresh generation
    /// directory, returning its path.
    ///
    /// `Ok(None)` means the champion predates the mechanism IR (a
    /// legacy workspace with only hand-written plugins); the caller
    /// should then fall back to the baseline mechanism and say so.
    pub fn copy_champion_mechanism_into(
        &self,
        champion: u32,
        generation: u32,
    ) -> Result<Option<PathBuf>> {
        let source = self.mechanism_path(champion);

        if !source.exists() {
            return Ok(None);
        }

        let dir = self.gen_dir(generation);

        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;

        let target = self.mechanism_path(generation);

        std::fs::copy(&source, &target)
            .with_context(|| format!("copying {} -> {}", source.display(), target.display()))?;

        Ok(Some(target))
    }

    pub fn champion(&self) -> Option<Champion> {
        let text = std::fs::read_to_string(self.champion_path()).ok()?;

        serde_json::from_str(&text).ok()
    }

    pub fn set_champion(&self, champion: &Champion) -> Result<()> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("creating {}", self.root.display()))?;

        std::fs::write(self.champion_path(), serde_json::to_string_pretty(champion)?)
            .with_context(|| format!("writing {}", self.champion_path().display()))
    }

    pub fn write_report(&self, report: &GenReport) -> Result<()> {
        let path = self.report_path(report.generation);

        std::fs::create_dir_all(path.parent().unwrap())
            .with_context(|| format!("creating {}", path.display()))?;

        std::fs::write(&path, serde_json::to_string_pretty(report)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn read_report(&self, generation: u32) -> Option<GenReport> {
        let text = std::fs::read_to_string(self.report_path(generation)).ok()?;

        serde_json::from_str(&text).ok()
    }
}

/// Content hash of a plugin source, for lineage bookkeeping.
pub fn source_hash(source: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();

    source.hash(&mut hasher);

    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(name: &str) -> Workspace {
        let root = std::env::temp_dir()
            .join("skill-workspace-test")
            .join(name);

        let _ = std::fs::remove_dir_all(&root);

        Workspace::new(root)
    }

    #[test]
    fn seeding_then_generations() {
        let workspace = temp_workspace("seeding");

        let baseline = std::env::temp_dir()
            .join("skill-workspace-test")
            .join("baseline_plugin.rs");

        std::fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        std::fs::write(&baseline, "// baseline\n").unwrap();

        assert!(!workspace.has_generations());

        assert_eq!(workspace.seed(&baseline).unwrap(), 0);
        assert!(workspace.plugin_path(0).exists());

        // Seeding is a no-op once generations exist.
        assert_eq!(workspace.seed(&baseline).unwrap(), 0);
    }

    #[test]
    fn champion_promotion_and_copy() {
        let workspace = temp_workspace("champion");

        std::fs::create_dir_all(workspace.gen_dir(1)).unwrap();
        std::fs::write(workspace.plugin_path(1), "// gen 1\n").unwrap();
        std::fs::create_dir_all(workspace.gen_dir(2)).unwrap();
        std::fs::write(workspace.plugin_path(2), "// gen 2 better\n").unwrap();

        assert!(workspace.champion().is_none());

        workspace
            .set_champion(&Champion { generation: 2, fitness: 0.5 })
            .unwrap();

        assert_eq!(workspace.champion().unwrap().generation, 2);

        workspace.copy_champion_into(2, 3).unwrap();

        assert_eq!(
            std::fs::read_to_string(workspace.plugin_path(3)).unwrap(),
            "// gen 2 better\n",
        );
    }

    #[test]
    fn reports_round_trip() {
        let workspace = temp_workspace("reports");

        let report = GenReport {
            generation: 3,
            parent: Some(2),
            source_hash: "abcd".to_string(),
            note: Some("tried bivalency".to_string()),
            status: "Success".to_string(),
            error: None,
            complexity: Some(Complexity {
                states: 2,
                parameters: 4,
                equations: 3,
            }),
            fitness: Some(0.42),
            per_subject: vec![SubjectReport {
                subject: "swift-swan-crystal".to_string(),
                status: "Success".to_string(),
                theta: vec![("kon".to_string(), 1e-4)],
                train_loss: 100.0,
                validation_rmse: 0.42,
                hidden_rmse: 0.9,
                pinned: vec![],
                modulation: Some(0.8),
            }],
            evaluator_only: EvaluatorOnly {
                hidden_rmse: Some(0.9),
                holdout: Some(HoldoutScore {
                    subject: "rapid-orca-dust".to_string(),
                    status: "Success".to_string(),
                    theta: vec![("kon".to_string(), 2e-4)],
                    validation_rmse: 0.8,
                    normalized_validation_rmse: 0.05,
                    hidden_rmse: 1.2,
                }),
            },
        };

        workspace.write_report(&report).unwrap();

        let read = workspace.read_report(3).expect("report readable");

        assert_eq!(read.fitness, Some(0.42));
        assert_eq!(read.evaluator_only.hidden_rmse, Some(0.9));

        let holdout = read.evaluator_only.holdout.expect("holdout round-trips");

        assert_eq!(holdout.subject, "rapid-orca-dust");
        assert_eq!(holdout.normalized_validation_rmse, 0.05);
    }

    #[test]
    fn legacy_reports_without_a_holdout_still_read() {
        let workspace = temp_workspace("legacy-holdout");

        // A report written before the holdout field existed.
        let dir = workspace.gen_dir(2);

        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("report.json"),
            r#"{"generation":2,"parent":null,"source_hash":"ab",
               "note":null,"status":"Success","error":null,
               "complexity":null,"fitness":null,"per_subject":[],
               "evaluator_only":{"hidden_rmse":null}}"#,
        )
        .unwrap();

        let read = workspace.read_report(2).expect("legacy report readable");

        assert!(read.evaluator_only.holdout.is_none());
    }

    #[test]
    fn latest_gen_tracks_the_counter() {
        let workspace = temp_workspace("counter");

        assert_eq!(workspace.latest_gen(), None);

        std::fs::create_dir_all(workspace.gen_dir(7)).unwrap();

        assert_eq!(workspace.latest_gen(), Some(7));
    }
}