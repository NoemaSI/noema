//! Candidate model loading: the skill world's mirror of the warehouse
//! plugin loader.
//!
//! A candidate mechanism arrives either as a Modelica source file or as
//! a [`CandidateSpec`] handed over by a candidate plugin (see
//! [`crate::plugin`]). The harness verifies the fixed external
//! contract, compiles it with Rumoca through the shared [`modelica`]
//! crate, and caches the artifact under the content hash of the source
//! — re-compiling only when the source actually changed, exactly like
//! the plugin dylib cache.
//!
//! The contract (verified before any compilation):
//!
//! - the model identifier matches the declared name,
//! - the interface is exactly the pack's declared channels:
//!   `input Real <channel>;` for every lexicon input and
//!   `output Real <channel>;` for every lexicon output,
//! - everything below that boundary is the model author's own choice.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Result, anyhow, bail};

use crate::experiment::{Experiment, Parameter, SimError, Simulated, sanitize};
use crate::lexicon::Lexicon;
use crate::mechanism::{Complexity, MechanismModel};

/// What a candidate plugin hands the harness: the *source* of a
/// mechanism, never an execution. The harness owns compile → simulate →
/// fit (harness controls reality); the author owns structure.
///
/// A plugin source file implements this trait and exposes
///
/// ```ignore
/// pub fn make_candidate() -> Box<dyn skill::candidate::CandidateSpec>
/// ```
///
/// so the whole LLM-editable canvas stays inspectable Rust with the
/// Modelica mechanism embedded as data (see `packs/binding/baseline_plugin.rs`).
pub trait CandidateSpec {
    /// Modelica model identifier (the artifact is named after it).
    fn name(&self) -> String;

    /// Complete `.mo` source, contract-checked by the harness before
    /// any compilation.
    fn modelica(&self) -> String;

    /// Author rationale, logged into the generation report and passed
    /// to the next generation's context. Never scored.
    fn note(&self) -> Option<String> {
        None
    }
}

/// A compiled candidate mechanism, ready to simulate and fit.
pub struct CandidateModel {
    name: String,
    compiled: modelica::CompiledModel,
    parameters: Vec<Parameter>,
    complexity: Complexity,
    note: Option<String>,

    /// The pack's declared output channels, in lexicon order: what
    /// [`MechanismModel::simulate`] reads out of every trace.
    output_channels: Vec<String>,
}

impl CandidateModel {
    /// Load, contract-check, and compile (or reuse the cached
    /// compilation of) a candidate source file.
    pub fn load(source_path: &Path, lexicon: &Lexicon) -> Result<Self> {
        let name = source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| anyhow!("invalid candidate filename"))?
            .to_string();

        let source = std::fs::read_to_string(source_path)
            .map_err(|error| anyhow!("reading {}: {error}", source_path.display()))?;

        Self::assemble(name, &source, Some(source_path.to_path_buf()), None, lexicon)
    }

    /// Compile a mechanism supplied in-memory by a candidate plugin.
    pub fn from_spec(spec: &dyn CandidateSpec, lexicon: &Lexicon) -> Result<Self> {
        Self::assemble(spec.name(), &spec.modelica(), None, spec.note(), lexicon)
    }

    fn assemble(
        name: String,
        source: &str,
        source_path: Option<PathBuf>,
        note: Option<String>,
        lexicon: &Lexicon,
    ) -> Result<Self> {
        validate_contract(&name, source, lexicon)?;

        let artifact_dir = cache_dir(source.as_bytes(), &name);

        let mo_file = format!("{name}.mo");

        std::fs::create_dir_all(&artifact_dir)
            .map_err(|error| anyhow!("creating candidate cache: {error}"))?;

        std::fs::write(artifact_dir.join(&mo_file), source)
            .map_err(|error| anyhow!("caching candidate source: {error}"))?;

        let artifact = artifact_dir
            .join(&name)
            .join(format!("{name}_casadi_mx.py"));

        // Content-hash cache: identical source, identical artifact
        // directory — skip straight to inspection.
        let dynamics = if artifact.exists() {
            modelica::inspect(&artifact)
                .map_err(|error| anyhow!("inspecting cached artifact: {error}"))?
        } else {
            modelica::compile(&artifact_dir, &mo_file, &name)
                .map_err(|error| anyhow!("{error}"))?
                .dynamics
        };

        let parameters = dynamics
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter_name)| {
                let (min, max) = declared_bounds(source, parameter_name);

                Parameter {
                    name: parameter_name.clone(),
                    start: dynamics.p0.get(index).copied().unwrap_or(0.0),
                    min,
                    max,
                }
            })
            .collect();

        let complexity = Complexity {
            states: dynamics.states.len(),
            parameters: dynamics.parameters.len(),
            equations: count_equations(source, &name),
        };

        let compiled = modelica::CompiledModel {
            source: source_path.unwrap_or_else(|| artifact_dir.join(&mo_file)),
            target: "casadi-mx".to_string(),
            artifact,
            dynamics,
        };

        Ok(Self {
            name,
            compiled,
            parameters,
            complexity,
            note,
            output_channels: lexicon
                .outputs
                .iter()
                .map(|channel| channel.name.clone())
                .collect(),
        })
    }

    /// The author rationale, if any (logged, never scored).
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The compiled artifact, for the fixed Python-side fitter.
    pub fn compiled(&self) -> &modelica::CompiledModel {
        &self.compiled
    }
}

impl MechanismModel for CandidateModel {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn parameters(&self) -> Vec<Parameter> {
        self.parameters.clone()
    }

    fn simulate(
        &self,
        theta: &[f64],
        experiment: &Experiment,
    ) -> Result<Simulated, SimError> {
        // Replay the experiment's recorded drive traces (piecewise-
        // constant between their samples; the fitter's fit.py performs
        // the identical replay). Which traces those are — a washout
        // staircase, a valve schedule — is the pack's data, not this
        // engine's rule.
        let drives: Vec<modelica::DriveSamples<'_>> = self
            .compiled
            .dynamics
            .inputs
            .iter()
            .map(|channel| {
                let drive = experiment.input(channel).ok_or_else(|| {
                    SimError::Invalid(format!(
                        "experiment carries no drive for model input `{channel}`",
                    ))
                })?;

                Ok(modelica::DriveSamples {
                    channel,
                    t: &drive.t,
                    values: &drive.values,
                })
            })
            .collect::<Result<_, SimError>>()?;

        let trace = self
            .compiled
            .simulate_traces(theta, &drives, &experiment.t)
            .map_err(|error| SimError::Failed(error.to_string()))?;

        self.output_channels
            .iter()
            .map(|channel| {
                let values = trace
                    .trace_of(&self.compiled.dynamics, channel)
                    .ok_or_else(|| {
                        SimError::Invalid(format!(
                            "compiled candidate provides no `{channel}` variable",
                        ))
                    })?;

                sanitize(&values, experiment.t.len()).map(|values| (channel.clone(), values))
            })
            .collect()
    }

    fn complexity(&self) -> Complexity {
        self.complexity
    }
}

/// Verify the fixed external contract of a candidate source: every
/// measured output channel in full, and an input interface the pack
/// actually records. A candidate may consume a SUBSET of the pack's
/// input channels — choosing which covariates to read is structure
/// selection (a law's feature set, a binding model ignoring a
/// redundant channel) — but it may never invent a channel: the only
/// drives that exist are the ones the pack measured.
fn validate_contract(name: &str, source: &str, lexicon: &Lexicon) -> Result<()> {
    if !source.contains(&format!("model {name}")) {
        bail!("candidate must declare `model {name}` (the file stem)");
    }

    let mut cursor = source;

    while let Some(rest) = cursor.find("input Real ") {
        let tail = &cursor[rest + "input Real ".len()..];

        let name_end = tail
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(tail.len());

        let input = &tail[..name_end];

        if !lexicon.inputs.iter().any(|channel| channel.name == input) {
            bail!(
                "candidate declares `input Real {input}`, which the pack does not record \
                 (inputs must be lexicon channels)",
            );
        }

        cursor = &tail[name_end..];
    }

    for channel in &lexicon.outputs {
        if !source.contains(&format!("output Real {}", channel.name)) {
            bail!(
                "candidate must declare `output Real {}` (the pack's output channel)",
                channel.name,
            );
        }
    }

    Ok(())
}

/// Cache directory for one candidate source revision: mirrors the
/// plugin loader's `$TMPDIR/skill-packs/<hash>/` convention.
fn cache_dir(source: &[u8], name: &str) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    static RUNTIME_TAG: OnceLock<String> = OnceLock::new();

    let tag = RUNTIME_TAG.get_or_init(|| {
        // Mix the rumoca version into the key so a toolchain upgrade
        // never reuses stale artifacts.
        std::process::Command::new("rumoca")
            .arg("--version")
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_else(|_| "unknown".to_string())
    });

    let mut hasher = DefaultHasher::new();

    source.hash(&mut hasher);
    name.hash(&mut hasher);
    tag.hash(&mut hasher);

    std::env::temp_dir()
        .join("skill-candidates")
        .join(format!("{:016x}", hasher.finish()))
}

/// Fit bounds declared in the candidate source, e.g.
/// `parameter Real kon(start=1e-3, min=1e-12, max=1e6);`.
///
/// Undeclared bounds stay wide: the parameter is then effectively free
/// within ±1e9.
fn declared_bounds(source: &str, name: &str) -> (f64, f64) {
    let default = (-1e9, 1e9);

    let pattern = format!("parameter Real {name}");

    let Some(at) = source.find(&pattern) else {
        return default;
    };

    let rest = &source[at + pattern.len()..];

    let Some(close) = rest.find(')') else {
        return default;
    };

    let declaration = &rest[..close];

    let min = parse_bound(declaration, "min").unwrap_or(default.0);
    let max = parse_bound(declaration, "max").unwrap_or(default.1);

    (min, max)
}

fn parse_bound(declaration: &str, key: &str) -> Option<f64> {
    let mut offset = 0;

    while let Some(found) = declaration[offset..].find(key) {
        let at = offset + found;

        let preceded_by_word = at > 0
            && declaration[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');

        let after = &declaration[at + key.len()..];

        let value = after.trim_start().strip_prefix('=')?.trim_start();

        if preceded_by_word {
            offset = at + key.len();
            continue;
        }

        let token: String = value
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
            .collect();

        return token.parse().ok();
    }

    None
}

/// Structural equation count for the complexity policy: statements in
/// the equation section, comments ignored.
fn count_equations(source: &str, name: &str) -> usize {
    let start = match source.find("equation") {
        Some(at) => at + "equation".len(),
        None => return 0,
    };

    let end = source
        .rfind(&format!("end {name}"))
        .unwrap_or(source.len());

    if end <= start {
        return 0;
    }

    source[start..end]
        .split(';')
        .map(|statement| {
            statement
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<String>()
        })
        .filter(|statement| statement.trim().contains('='))
        .count()
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

    const SOURCE: &str = r#"
model Binding1to1
  input Real concentration;
  output Real signal;

  parameter Real kon(start=1e-3, min=1e-12, max=1e6);
  parameter Real koff(start=0.01, min=1e-12, max=1e6);
  parameter Real rmax(start=100.0);

  Real pl(start=0);

equation
  der(pl) = kon * concentration * (rmax - pl) - koff * pl;
  signal = pl;
end Binding1to1;
"#;

    #[test]
    fn contract_is_enforced() {
        validate_contract("Binding1to1", SOURCE, &lexicon()).expect("valid contract");

        assert!(validate_contract("Other", SOURCE, &lexicon()).is_err());
        assert!(validate_contract("Binding1to1", "model X end X;", &lexicon()).is_err());

        // A source declaring the wrong input name is rejected.
        let wrong_input = SOURCE.replace("concentration", "dose");

        assert!(validate_contract("Binding1to1", &wrong_input, &lexicon()).is_err());
    }

    #[test]
    fn input_subsets_are_structure_selection() {
        let mut wide = lexicon();

        wide.inputs.push(crate::lexicon::Channel {
            name: "temperature".to_string(),
            units: "C".to_string(),
            description: "assay temperature".to_string(),
        });

        // A candidate that reads only `concentration` is a legitimate
        // (smaller) structure: the contract accepts the subset.
        validate_contract("Binding1to1", SOURCE, &wide).expect("subset accepted");
    }

    #[test]
    fn bounds_are_read_from_declarations() {
        assert_eq!(declared_bounds(SOURCE, "kon"), (1e-12, 1e6));
        assert_eq!(declared_bounds(SOURCE, "koff"), (1e-12, 1e6));

        // Declared without bounds: wide open.
        assert_eq!(declared_bounds(SOURCE, "rmax"), (-1e9, 1e9));

        // Not a parameter at all.
        assert_eq!(declared_bounds(SOURCE, "pl"), (-1e9, 1e9));
    }

    #[test]
    fn equations_are_counted() {
        assert_eq!(count_equations(SOURCE, "Binding1to1"), 2);
    }
}