//! The problem pack: everything domain-specific about one discovery
//! task, behind one trait.
//!
//! The skill crate fixes the loop (theory → lower → fit → score →
//! judge → promote), the IR, the optimizer drive, and the scoring. A
//! pack supplies the reality the loop searches against:
//!
//! - the measured [`Subject`]s (data ingestion is the pack's job),
//! - the train/validation/hidden [`SplitPolicy`],
//! - the [`Lexicon`] (names and units of the driven input and the
//!   measured output),
//! - the baseline mechanism and (optionally) a hand-written plugin,
//! - the domain text spliced into the LLM prompts.
//!
//! A pack is authored as a plugin source file exposing
//!
//! ```ignore
//! pub fn load_pack() -> Box<dyn skill::pack::ProblemPack>
//! ```
//!
//! and loaded by [`crate::pack_loader`]. Pack code runs at the harness
//! trust level — it selects reality, never scores candidates — but it
//! cannot touch the loop: scoring, splitting, and fitting consume the
//! pack's *data* through fixed harness code paths.

use std::path::PathBuf;

use anyhow::Result;

use crate::data::Curve;
use crate::experiment::Condition;
use crate::lexicon::Lexicon;

/// One measurable thing with a public identity: a designed binder, a
/// catalyst batch, a cell line. The loop fits one mechanism
/// structurally across all subjects; per-subject thetas are the
/// optimizer's business.
#[derive(Debug, Clone)]
pub struct Subject {
    /// Stable identifier (used in reports, plots, and ledgers).
    pub id: String,

    /// Human-readable name shown in prompts and tables.
    pub title: String,

    /// Public metadata pairs (e.g. `("sequence", "PSTV...")`,
    /// `("binding_strength", "Strong")`) rendered into the LLM
    /// prompts. Everything here is visible to the LLM; anything
    /// hidden stays out of the pack's public view entirely.
    pub detail: Vec<(String, String)>,

    /// All measured curves of this subject. The harness — not the
    /// pack — assigns train/validation/hidden roles via the split
    /// policy, so a pack cannot leak held-out data into the loop.
    pub curves: Vec<Curve>,
}

/// Which diagnostic figure the loop renders for this pack's
/// mechanisms: time-series overlays (the honest picture when the
/// independent axis really is time) or predicted-vs-measured parity
/// scatter (law packs, where the axis is a covariate placeholder and
/// the overlay is unreadable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagnosticMode {
    #[default]
    TimeSeries,
    Parity,
}

/// The fixed conditioning roles of one benchmark. Numeric levels are
/// matched through [`crate::data::quantize`]-equivalent tolerance
/// ([`Condition::Level`]); tags match exactly. The harness enforces:
/// the optimizer only ever sees `train`; `validation` decides
/// promotion; `hidden` is reported but never fitted or used for
/// selection.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitPolicy {
    pub train: Vec<Condition>,
    pub validation: Vec<Condition>,
    pub hidden: Vec<Condition>,
}

impl SplitPolicy {
    /// A level-conditioned policy (the single-scalar-drive case every
    /// dose–response style benchmark uses).
    pub fn levels(train: &[f64], validation: &[f64], hidden: &[f64]) -> Self {
        Self {
            train: train.iter().map(|level| Condition::Level(*level)).collect(),
            validation: validation
                .iter()
                .map(|level| Condition::Level(*level))
                .collect(),
            hidden: hidden.iter().map(|level| Condition::Level(*level)).collect(),
        }
    }

    /// A tag-conditioned policy: the split axis is a named regime per
    /// curve (a binder id held out of a law's training set, a recipe,
    /// a protocol family). Tags match exactly.
    pub fn tags(train: &[String], validation: &[String], hidden: &[String]) -> Self {
        let tagged = |values: &[String]| -> Vec<Condition> {
            values
                .iter()
                .map(|value| Condition::Tag(value.clone()))
                .collect()
        };

        Self {
            train: tagged(train),
            validation: tagged(validation),
            hidden: tagged(hidden),
        }
    }
}

/// Generation-0 seed of the evolution workspace. Paths may be
/// relative to the pack's home directory (the directory holding the
/// pack source file), resolved by the driver.
#[derive(Debug, Clone)]
pub struct Baseline {
    /// The mechanism IR (JSON) the loop starts from.
    pub mechanism: PathBuf,

    /// Optional hand-written candidate plugin: the escape hatch for
    /// mechanisms the IR cannot express. When present, the driver may
    /// load it through [`crate::plugin::load_candidate`].
    pub plugin: Option<PathBuf>,
}

/// The problem a skill instance is instantiated for.
pub trait ProblemPack: Send + Sync {
    /// Workspace namespace (e.g. `"binding"`): the loop's default
    /// workspace directory is `workspace/<name>/`.
    fn name(&self) -> String;

    /// One-paragraph statement of the discovery task, injected into
    /// the theorist and referee prompts.
    fn problem_statement(&self) -> String;

    /// Names and units of the driven input and measured output.
    fn lexicon(&self) -> Lexicon;

    /// Load every subject with its measured curves. Called once per
    /// run by the driver.
    fn subjects(&self) -> Result<Vec<Subject>>;

    /// The fixed train/validation/hidden conditions.
    fn split_policy(&self) -> SplitPolicy;

    /// The generation-0 mechanism (and optional hand-written plugin).
    fn baseline(&self) -> Baseline;

    /// Domain text appended after the static IR-specification section
    /// of the theorist prompt (assay physics, structure families, a
    /// worked IR example).
    fn theorist_preamble(&self) -> String {
        String::new()
    }

    /// Domain text appended to the static IR-repair preamble.
    fn doctor_preamble(&self) -> String {
        String::new()
    }

    /// Which diagnostic figure the loop shows the theorist and the
    /// referee for this pack's mechanisms. Time-series overlays are
    /// the honest picture only when the independent axis really is
    /// time; law packs (where it is a covariate placeholder) declare
    /// [`DiagnosticMode::Parity`] and get predicted-vs-measured
    /// scatter instead.
    fn diagnostic_mode(&self) -> DiagnosticMode {
        DiagnosticMode::TimeSeries
    }

    /// Domain text appended to the static referee rules (how to read
    /// the diagnostic plots in this domain).
    fn referee_preamble(&self) -> String {
        String::new()
    }

    /// One-paragraph intro above the subject table in the prompts
    /// (what a row is, which metadata columns matter).
    fn subject_table_intro(&self) -> String {
        String::new()
    }

    /// CSV of the training observations handed to the theorist. The
    /// generic default downsamples each training curve and emits
    /// `subject,curve,input,time_s,signal` (the `input` column is the
    /// conditioning label); packs with idiosyncratic conventions may
    /// override.
    fn observation_csv(&self, subjects: &[Subject], max_points: usize) -> String {
        let policy = self.split_policy();

        let mut text = String::from("subject,curve,input,time_s,signal\n");

        for subject in subjects {
            for (curve_index, curve) in subject
                .curves
                .iter()
                .filter(|curve| {
                    policy
                        .train
                        .iter()
                        .any(|condition| curve.condition.matches(condition))
                })
                .enumerate()
            {
                let points = curve.raw.t.len();

                let stride = (points / max_points.max(1)).max(1);

                for index in (0..points).step_by(stride) {
                    text.push_str(&format!(
                        "{},{},{},{},{}\n",
                        subject.id,
                        curve_index,
                        curve.condition.display(),
                        curve.raw.t[index],
                        curve.raw.y[index],
                    ));
                }
            }
        }

        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::RawCurve;

    struct Toy;

    impl ProblemPack for Toy {
        fn name(&self) -> String {
            "toy".to_string()
        }

        fn problem_statement(&self) -> String {
            "Find the toy.".to_string()
        }

        fn lexicon(&self) -> Lexicon {
            Lexicon {
                inputs: vec![crate::lexicon::Channel {
                    name: "dose".to_string(),
                    units: "uM".to_string(),
                    description: "electrode dose".to_string(),
                }],
                outputs: vec![crate::lexicon::Channel {
                    name: "signal".to_string(),
                    units: "mV".to_string(),
                    description: "electrode potential".to_string(),
                }],
                roles: vec![],
                notes: String::new(),
            }
        }

        fn subjects(&self) -> Result<Vec<Subject>> {
            Ok(vec![])
        }

        fn split_policy(&self) -> SplitPolicy {
            SplitPolicy::levels(&[0.0, 10.0], &[30.0], &[100.0])
        }

        fn baseline(&self) -> Baseline {
            Baseline {
                mechanism: PathBuf::from("mechanism.json"),
                plugin: None,
            }
        }
    }

    #[test]
    fn observation_csv_selects_train_inputs_only() {
        let subject = Subject {
            id: "s1".to_string(),
            title: "Subject 1".to_string(),
            detail: vec![],
            curves: vec![
                Curve {
                    condition: crate::experiment::Condition::Level(10.0),
                    raw: RawCurve {
                        t: (0..40).map(|i| i as f64).collect(),
                        y: (0..40).map(|i| i as f64 * 0.5).collect(),
                    },
                    session: 0,
                    t_step: None,
                    input_after: 0.0,
                    drives: vec![],
                    output_channel: None,
                },
                Curve {
                    condition: crate::experiment::Condition::Level(100.0),
                    raw: RawCurve {
                        t: vec![0.0, 1.0],
                        y: vec![9.0, 9.0],
                    },
                    session: 0,
                    t_step: None,
                    input_after: 0.0,
                    drives: vec![],
                    output_channel: None,
                },
            ],
        };

        let csv = Toy.observation_csv(&[subject], 10);

        assert!(csv.starts_with("subject,curve,input,time_s,signal\n"));
        assert!(csv.contains(",10,"));
        assert!(!csv.contains(",100,"), "hidden inputs must not leak: {csv}");
    }
}