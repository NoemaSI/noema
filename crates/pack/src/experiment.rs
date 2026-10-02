//! The domain-independent experiment abstraction.
//!
//! One [`Experiment`] is one measured response: the drives of every
//! input channel as recorded piecewise-constant traces, the
//! conditioning value the split roles on, and the measured time
//! series of every output channel. Everything downstream — fitting,
//! scoring, baselines, and the LLM loop — only ever sees this shape,
//! never the pack's record structure.
//!
//! The replay contract is minimal and universal: the drive of each
//! channel is data (recorded samples, held with zero-order hold
//! between them), the model stays time-invariant, and the harness
//! replays. What a trace *means* — a washout step to zero, a valve
//! opening, a temperature ramp approximated by stairs — is the pack's
//! business.

use serde::{Deserialize, Serialize};

/// The conditioning value of one experiment: what the split roles
/// on, and what generalization is tested across.
///
/// [`Condition::Level`] is a numeric drive level (matched through
/// [`crate::data::quantize`], so dilution-series noise cannot
/// fragment the split); [`Condition::Tag`] is a named regime (a
/// recipe, a shift, an operating mode) matched by exact string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    Level(f64),
    Tag(String),
}

impl Condition {
    pub fn level(&self) -> Option<f64> {
        match self {
            Self::Level(value) => Some(*value),
            Self::Tag(_) => None,
        }
    }

    /// Role-matching equality: numeric levels compare through
    /// `quantize`, tags by exact name.
    pub fn matches(&self, other: &Condition) -> bool {
        match (self, other) {
            (Self::Level(a), Self::Level(b)) => {
                crate::data::quantize(*a) == crate::data::quantize(*b)
            }
            (Self::Tag(a), Self::Tag(b)) => a == b,
            _ => false,
        }
    }

    /// Short label for prompts, tables, and plot legends.
    pub fn display(&self) -> String {
        match self {
            Self::Level(value) => {
                if *value == value.trunc() && value.abs() < 1e15 {
                    format!("{}", *value as i64)
                } else {
                    format!("{value}")
                }
            }
            Self::Tag(tag) => tag.clone(),
        }
    }
}

impl From<f64> for Condition {
    fn from(value: f64) -> Self {
        Self::Level(value)
    }
}

/// One input channel's recorded drive: piecewise-constant samples
/// (zero-order hold: `values[i]` is held from `t[i]` until `t[i+1]`,
/// the last value held to the end of the record). Recorded data,
/// never a fitted parameter — the harness replays it identically in
/// the fitter and the scorer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputDrive {
    /// Declared channel name (see [`crate::lexicon::Lexicon::inputs`]).
    pub channel: String,

    /// Sample times, starting at or before the record's first time.
    pub t: Vec<f64>,

    /// Value held from each sample time onwards.
    pub values: Vec<f64>,
}

impl InputDrive {
    /// A channel held at one constant value.
    pub fn constant(channel: &str, value: f64) -> Self {
        Self {
            channel: channel.to_string(),
            t: vec![0.0],
            values: vec![value],
        }
    }

    /// A step protocol: `level` until `t_step`, then `after`. This is
    /// the two-sample staircase every washout-style assay reduces
    /// to; `t_step <= 0` means the record starts in the second
    /// phase.
    pub fn stepped(channel: &str, level: f64, t_step: f64, after: f64) -> Self {
        if t_step <= 0.0 {
            return Self::constant(channel, after);
        }

        Self {
            channel: channel.to_string(),
            t: vec![0.0, t_step],
            values: vec![level, after],
        }
    }

    /// The drive of a loaded [`crate::data::Curve`]: its recorded
    /// extra drives when the pack supplied them, else the primary
    /// channel's constant-or-stepped staircase from the curve's
    /// protocol fields.
    pub fn from_curve(channel: &str, curve: &crate::data::Curve) -> Self {
        if !curve.drives.is_empty() {
            return curve.drives[0].clone();
        }

        let level = curve.condition.level().unwrap_or(0.0);

        match curve.t_step {
            Some(t_step) => Self::stepped(channel, level, t_step, curve.input_after),
            None => Self::constant(channel, level),
        }
    }
}

/// One measured output channel of an experiment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputSeries {
    /// Declared channel name (see [`crate::lexicon::Lexicon::outputs`]).
    pub channel: String,

    /// Observed values at the experiment's times.
    pub y: Vec<f64>,
}

/// One measured response: drives in, observations out, at one
/// condition on one subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Experiment {
    /// Identifier of the subject the response was measured on.
    pub subject: String,

    /// The conditioning value of this experiment (the split's axis).
    pub condition: Condition,

    /// Observation times, starting at 0. All output series share this
    /// grid; drive traces carry their own sample grids.
    pub t: Vec<f64>,

    /// One recorded drive per input channel.
    pub inputs: Vec<InputDrive>,

    /// One measured series per output channel.
    pub outputs: Vec<OutputSeries>,
}

impl Experiment {
    pub fn len(&self) -> usize {
        self.t.len()
    }

    pub fn is_empty(&self) -> bool {
        self.t.is_empty()
    }

    /// The measured series of one output channel.
    pub fn output(&self, channel: &str) -> Option<&OutputSeries> {
        self.outputs.iter().find(|series| series.channel == channel)
    }

    /// The drive samples of one input channel.
    pub fn input(&self, channel: &str) -> Option<&InputDrive> {
        self.inputs.iter().find(|drive| drive.channel == channel)
    }
}

/// A simulated response: one predicted series per output channel, in
/// the order the model declares them.
pub type Simulated = Vec<(String, Vec<f64>)>;

/// The predicted series of one output channel.
pub fn simulated_channel<'a>(simulated: &'a Simulated, name: &str) -> Option<&'a [f64]> {
    simulated
        .iter()
        .find(|(channel, _)| channel == name)
        .map(|(_, values)| values.as_slice())
}

/// A model parameter the fixed optimizer may move.
///
/// The model author declares that the parameter exists and bounds it;
/// the optimizer chooses its value and understands nothing else about
/// it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub start: f64,
    pub min: f64,
    pub max: f64,
}

/// Execution status of a candidate against the harness.
///
/// Execution failure is kept strictly apart from scientific
/// performance: a candidate that cannot run never receives a score.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Rumoca rejected the candidate source; carries the real
    /// compiler diagnostic.
    CompileFailed(String),

    /// The candidate compiled but a simulation run failed; carries the
    /// real runtime diagnostic.
    SimulationFailed(String),

    /// The simulation ran but produced numerics that cannot be scored
    /// (NaN, Inf, or length mismatch).
    SimulationInvalid(String),

    /// The candidate ran; `visible_loss` is the fixed objective value
    /// on the discovery curves.
    Success { visible_loss: f64 },
}

/// Failure of one candidate simulation.
#[derive(Debug, Clone, PartialEq)]
pub enum SimError {
    Failed(String),
    Invalid(String),
}

/// Reject numerics that cannot enter the fixed loss.
pub fn sanitize(y: &[f64], expected_len: usize) -> Result<Vec<f64>, SimError> {
    if y.len() != expected_len {
        return Err(SimError::Invalid(format!(
            "expected {} samples, got {}",
            expected_len,
            y.len(),
        )));
    }

    for value in y {
        if !value.is_finite() {
            return Err(SimError::Invalid(format!(
                "non-finite value {value} in predicted signal",
            )));
        }
    }

    Ok(y.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_nan_and_length_mismatch() {
        assert!(sanitize(&[0.0, 1.0], 2).is_ok());

        assert!(matches!(
            sanitize(&[0.0, f64::NAN], 2),
            Err(SimError::Invalid(_))
        ));

        assert!(matches!(
            sanitize(&[0.0], 2),
            Err(SimError::Invalid(_))
        ));
    }

    #[test]
    fn conditions_match_and_display() {
        assert!(Condition::Level(316.2).matches(&Condition::Level(316.0)));
        assert!(!Condition::Level(316.2).matches(&Condition::Level(100.0)));
        assert!(Condition::Tag("night".to_string()).matches(&Condition::Tag("night".to_string())));
        assert!(!Condition::Tag("night".to_string()).matches(&Condition::Level(1.0)));

        assert_eq!(Condition::Level(31.6).display(), "31.6");
        assert_eq!(Condition::Level(0.0).display(), "0");
        assert_eq!(Condition::Level(100.0).display(), "100");
        assert_eq!(Condition::Tag("recipe-7".to_string()).display(), "recipe-7");
    }

    #[test]
    fn stepped_drive_collapses_a_step_before_the_record() {
        let drive = InputDrive::stepped("concentration", 5.0, -1.0, 0.0);

        assert_eq!(drive, InputDrive::constant("concentration", 0.0));

        let drive = InputDrive::stepped("concentration", 5.0, 30.0, 0.0);

        assert_eq!(drive.t, vec![0.0, 30.0]);
        assert_eq!(drive.values, vec![5.0, 0.0]);
    }
}