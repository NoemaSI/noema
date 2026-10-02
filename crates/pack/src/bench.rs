//! The referee: fit against the visible curves, freeze, score the
//! held-out inputs.
//!
//! Same role as `run_scenario` in the warehouse world. Hidden curves
//! never enter the fit; they are touched exactly once, after theta is
//! frozen, and the report is the only thing that comes out.

use crate::candidate::CandidateModel;
use crate::data::Curve;
use crate::experiment::{Experiment, InputDrive, OutputSeries, Status, simulated_channel};
use crate::fit;
use crate::lexicon::Lexicon;
use crate::mechanism::{Complexity, MechanismModel, squared_error, sum_squared_residuals};
use crate::split::{BenchmarkDataset, CurveRoles};

/// Per-subject outcome of the evolution evaluation: fit on train,
/// selected by validation, scored (evaluator-only) on hidden.
#[derive(Debug, Clone)]
pub struct RoleReport {
    pub subject: String,
    pub status: Status,
    pub theta: Vec<(String, f64)>,

    /// Fixed SSE on the train curves at the frozen theta.
    pub train_loss: f64,

    /// Selection fitness: RMSE on the visible validation curves.
    pub validation_rmse: f64,

    /// Evaluator-only: per hidden curve.
    pub hidden: Vec<HeldOutScore>,

    /// Evaluator-only: aggregate hidden RMSE.
    pub hidden_rmse: f64,

    /// Sentinel: fitted parameters stuck at a declared bound — the
    /// optimizer cannot follow where the data pushes, so the bound
    /// (or the mechanism form) is wrong. Author-facing by design:
    /// the theorist owns both bounds and form.
    pub pinned: Vec<String>,

    /// Sentinel: how much the frozen mechanism varies its output
    /// across the validation conditions, relative to how much the
    /// measured data varies there (worst output channel). Far below
    /// 1 means the mechanism largely ignores the condition axis even
    /// when its RMSE looks respectable.
    pub modulation: Option<f64>,

    pub complexity: Complexity,
}

impl RoleReport {
    pub fn is_success(&self) -> bool {
        matches!(self.status, Status::Success { .. })
    }
}

/// Fit theta on the train curves only, freeze, then measure validation
/// (selection) and hidden (evaluator-only) RMSE at the frozen theta.
pub fn fit_and_score(
    candidate: &CandidateModel,
    roles: &CurveRoles,
    subject_id: &str,
    max_nfev: usize,
    lexicon: &Lexicon,
) -> RoleReport {
    let failed = |message: String| RoleReport {
        subject: subject_id.to_string(),
        status: Status::SimulationFailed(message),
        theta: Vec::new(),
        train_loss: f64::NAN,
        validation_rmse: f64::NAN,
        hidden: Vec::new(),
        hidden_rmse: f64::NAN,
        pinned: Vec::new(),
        modulation: None,
        complexity: candidate.complexity(),
    };

    if roles.train.is_empty() || roles.validation.is_empty() {
        return failed(format!(
            "subject {subject_id} has no {} curves",
            if roles.train.is_empty() { "train" } else { "validation" },
        ));
    }

    let train: Vec<Experiment> = roles
        .train
        .iter()
        .map(|curve| experiment(subject_id, curve, lexicon))
        .collect();

    let outcome = match fit::fit(candidate, &train, max_nfev) {
        Ok(outcome) => outcome,
        Err(error) => return failed(error.to_string()),
    };

    let theta = candidate
        .parameters()
        .into_iter()
        .map(|parameter| parameter.name)
        .zip(outcome.theta.clone())
        .collect();

    let train_loss = match sum_squared_residuals(candidate, &outcome.theta, &train) {
        Ok(loss) => loss,
        Err(error) => {
            return failed(format!("fitted candidate does not replay: {error:?}"));
        }
    };

    let validation: Vec<Experiment> = roles
        .validation
        .iter()
        .map(|curve| experiment(subject_id, curve, lexicon))
        .collect();

    let validation_rmse = match rmse(candidate, &outcome.theta, &validation) {
        Ok(rmse) => rmse,
        Err(error) => {
            return failed(format!("validation simulation failed: {error:?}"));
        }
    };

    // Evaluator-only pass: this is the last place hidden data is
    // touched, and its result never enters an author-facing structure.
    let mut hidden = Vec::new();
    let mut total_squared = 0.0;
    let mut total_points = 0usize;

    for curve in &roles.hidden {
        let exp = experiment(subject_id, curve, lexicon);

        let simulated = match candidate.simulate(&outcome.theta, &exp) {
            Ok(simulated) => simulated,
            Err(error) => {
                return failed(format!("held-out simulation failed: {error:?}"));
            }
        };

        let (squared, points) = match squared_error(&simulated, &exp) {
            Ok(scored) => scored,
            Err(error) => {
                return failed(format!("held-out scoring failed: {error:?}"));
            }
        };

        total_squared += squared;
        total_points += points;

        hidden.push(HeldOutScore {
            subject: subject_id.to_string(),
            condition: exp.condition.clone(),
            rmse: (squared / points.max(1) as f64).sqrt(),
        });
    }

    RoleReport {
        subject: subject_id.to_string(),
        status: Status::Success { visible_loss: train_loss },
        theta,
        train_loss,
        validation_rmse,
        hidden,
        hidden_rmse: if total_points == 0 {
            f64::NAN
        } else {
            (total_squared / total_points as f64).sqrt()
        },
        pinned: pinned_parameters(candidate, &outcome.theta),
        modulation: modulation_ratio(candidate, &outcome.theta, &validation, lexicon),
        complexity: candidate.complexity(),
    }
}

/// Sentinel 1: a fitted value sitting on its declared bound means the
/// optimizer wants to go further than the bound allows — the data is
/// telling the theorist that the bound is wrong or the functional form
/// is begging to be changed. Surfaced verbatim into the diagnostics.
fn pinned_parameters(candidate: &CandidateModel, theta: &[f64]) -> Vec<String> {
    candidate
        .parameters()
        .into_iter()
        .zip(theta)
        .filter_map(|(parameter, value)| {
            pinned_edge(&parameter.name, parameter.min, parameter.max, *value)
        })
        .collect()
}

/// Bound-proximity verdict for one fitted parameter. Kinetic
/// constants are declared with bounds spanning many orders of
/// magnitude, where linear distance drowns every small-but-free
/// value in the sentinel (a fitted kon of 2e-7 "looks pinned"
/// against a 1e-12..1e6 span). Proximity is therefore judged in
/// log10 space whenever the interval is positive and wide (≥ 2
/// decades); narrow or non-positive intervals keep the linear rule.
fn pinned_edge(name: &str, min: f64, max: f64, value: f64) -> Option<String> {
    let wide_log = min > 0.0 && max / min >= 100.0;

    let stuck = if wide_log && value > 0.0 {
        let log_span = (max / min).log10();

        if value.log10() - min.log10() <= 1e-3 * log_span {
            Some(("lower", min))
        } else if max.log10() - value.log10() <= 1e-3 * log_span {
            Some(("upper", max))
        } else {
            None
        }
    } else if value - min <= 1e-3 * (max - min) {
        Some(("lower", min))
    } else if max - value <= 1e-3 * (max - min) {
        Some(("upper", max))
    } else {
        None
    };

    stuck.map(|(edge, bound)| format!("{name} stuck at {edge} bound {bound:e}"))
}

/// Sentinel 2: condition-responsiveness, in units of the data's own
/// variation, computed on the visible validation conditions only. For
/// each output channel: group the experiments by condition, compare
/// how far the per-condition average response moves (model) against
/// how far it actually moves (data). A ratio near 0 while the data
/// varies says "this mechanism ignores the condition axis" — the
/// dose-collapse signature, generalized to any condition axis, any
/// output, any curve shape. `None` when there is nothing to compare
/// (fewer than two conditions or no channel with varying data).
fn modulation_ratio(
    candidate: &CandidateModel,
    theta: &[f64],
    validation: &[Experiment],
    lexicon: &Lexicon,
) -> Option<f64> {
    if validation.len() < 2 {
        return None;
    }

    let mean = |values: &[f64]| values.iter().copied().sum::<f64>() / values.len() as f64;

    // One simulation per experiment, reused across channels.
    let mut runs = Vec::new();

    for exp in validation {
        let simulated = candidate.simulate(theta, exp).ok()?;

        runs.push((exp, simulated));
    }

    let mut worst: Option<f64> = None;

    for channel in &lexicon.outputs {
        let mut groups: std::collections::BTreeMap<String, (Vec<f64>, Vec<f64>)> =
            std::collections::BTreeMap::new();

        for (exp, simulated) in &runs {
            let Some(model) = simulated_channel(simulated, &channel.name) else {
                continue;
            };

            let Some(observed) = exp
                .outputs
                .iter()
                .find(|series| series.channel == channel.name)
            else {
                continue;
            };

            if model.is_empty() || observed.y.is_empty() {
                continue;
            }

            let entry = groups
                .entry(exp.condition.display())
                .or_default();

            entry.0.push(mean(&observed.y));
            entry.1.push(mean(model));
        }

        if groups.len() < 2 {
            continue;
        }

        let spread = |values: Vec<f64>| {
            let max = values.iter().copied().fold(f64::MIN, f64::max);
            let min = values.iter().copied().fold(f64::MAX, f64::min);

            max - min
        };

        let data_spread = spread(groups.values().map(|(data, _)| mean(data)).collect());
        let model_spread = spread(groups.values().map(|(_, model)| mean(model)).collect());

        // Only channels whose data actually responds can witness
        // responsiveness (or its absence).
        if data_spread <= f64::EPSILON * data_spread.max(1.0) {
            continue;
        }

        let ratio = model_spread / data_spread;

        worst = Some(worst.map_or(ratio, |worst| worst.min(ratio)));
    }

    worst
}

/// RMSE of predictions against the observed curves at frozen theta.
fn rmse(
    candidate: &CandidateModel,
    theta: &[f64],
    experiments: &[Experiment],
) -> Result<f64, crate::experiment::SimError> {
    let mut squared = 0.0;
    let mut points = 0usize;

    for exp in experiments {
        let simulated = candidate.simulate(theta, exp)?;

        let (exp_squared, exp_points) = squared_error(&simulated, exp)?;

        squared += exp_squared;
        points += exp_points;
    }

    Ok((squared / points.max(1) as f64).sqrt())
}

/// Held-out score of one measured curve at the frozen theta.
#[derive(Debug, Clone)]
pub struct HeldOutScore {
    pub subject: String,
    pub condition: crate::experiment::Condition,
    pub rmse: f64,
}

/// The benchmark outcome for one candidate.
#[derive(Debug, Clone)]
pub struct BenchReport {
    pub candidate: String,
    pub status: Status,
    pub theta: Vec<(String, f64)>,

    /// The fixed loss on the discovery curves at the frozen theta.
    pub visible_loss: f64,

    /// Structure bookkeeping reported next to the fit, always: quality
    /// with unlimited flexibility is not quality.
    pub complexity: Complexity,

    pub held_out: Vec<HeldOutScore>,

    /// Root mean squared error over every held-out point of every
    /// subject.
    pub held_out_rmse: f64,
}

/// All discovery curves as experiments.
pub fn visible_experiments(
    benchmark: &BenchmarkDataset,
    lexicon: &Lexicon,
) -> Vec<Experiment> {
    benchmark
        .discovery
        .iter()
        .flat_map(|subject| {
            subject
                .visible_curves
                .iter()
                .map(move |curve| experiment(&subject.id, curve, lexicon))
        })
        .collect()
}

/// Indices of the subjects with the strongest visible response,
/// descending by max |signal| over their visible curves.
///
/// Selection uses visible data only — the split stays sealed. Flat
/// no-response curves carry no mechanistic information; the
/// discriminating benchmark runs live at the top of this order.
pub fn strongest(benchmark: &BenchmarkDataset, limit: usize) -> Vec<usize> {
    let mut ranked: Vec<(usize, f64)> = benchmark
        .discovery
        .iter()
        .enumerate()
        .map(|(index, subject)| {
            let peak = subject
                .visible_curves
                .iter()
                .flat_map(|curve| curve.raw.y.iter())
                .fold(0.0f64, |peak, value| peak.max(value.abs()));

            (index, peak)
        })
        .collect();

    ranked.sort_by(|(_, a), (_, b)| b.total_cmp(a));

    ranked
        .into_iter()
        .take(limit)
        .map(|(index, _)| index)
        .collect()
}

/// Fit the visible curves, freeze theta, then score the held-out side.
pub fn run(
    candidate: &CandidateModel,
    benchmark: &BenchmarkDataset,
    max_nfev: usize,
    lexicon: &Lexicon,
) -> BenchReport {
    let visible = visible_experiments(benchmark, lexicon);

    let outcome = match fit::fit(candidate, &visible, max_nfev) {
        Ok(outcome) => outcome,
        Err(error) => {
            return BenchReport {
                candidate: candidate.name(),
                status: Status::SimulationFailed(error.to_string()),
                theta: Vec::new(),
                visible_loss: f64::NAN,
                complexity: candidate.complexity(),
                held_out: Vec::new(),
                held_out_rmse: f64::NAN,
            };
        }
    };

    // Recompute the visible loss through the Rust path: the fitter's
    // number is confirmed by the same machinery that does the scoring,
    // not taken on faith.
    let visible_loss =
        match sum_squared_residuals(candidate, &outcome.theta, &visible) {
            Ok(loss) => loss,
            Err(error) => {
                return BenchReport {
                    candidate: candidate.name(),
                    status: Status::SimulationInvalid(format!(
                        "fitted candidate does not replay: {error:?}"
                    )),
                    theta: Vec::new(),
                    visible_loss: f64::NAN,
                    complexity: candidate.complexity(),
                    held_out: Vec::new(),
                    held_out_rmse: f64::NAN,
                };
            }
        };

    let mut held_out = Vec::new();

    let mut total_squared = 0.0;
    let mut total_points = 0usize;

    for scored in &benchmark.evaluation {
        for curve in &scored.held_out_curves {
            let experiment = experiment(&scored.id, curve, lexicon);

            let simulated = match candidate.simulate(&outcome.theta, &experiment) {
                Ok(simulated) => simulated,
                Err(error) => {
                    return BenchReport {
                        candidate: candidate.name(),
                        status: Status::SimulationInvalid(format!(
                            "held-out simulation failed: {error:?}"
                        )),
                        theta: Vec::new(),
                        visible_loss,
                        complexity: candidate.complexity(),
                        held_out: Vec::new(),
                        held_out_rmse: f64::NAN,
                    };
                }
            };

            let (squared, points) = match squared_error(&simulated, &experiment) {
                Ok(scored) => scored,
                Err(error) => {
                    return BenchReport {
                        candidate: candidate.name(),
                        status: Status::SimulationInvalid(format!(
                            "held-out scoring failed: {error:?}"
                        )),
                        theta: Vec::new(),
                        visible_loss,
                        complexity: candidate.complexity(),
                        held_out: Vec::new(),
                        held_out_rmse: f64::NAN,
                    };
                }
            };

            let rmse = (squared / points.max(1) as f64).sqrt();

            total_squared += squared;
            total_points += points;

            held_out.push(HeldOutScore {
                subject: scored.id.clone(),
                condition: experiment.condition.clone(),
                rmse,
            });
        }
    }

    let held_out_rmse = if total_points == 0 {
        f64::NAN
    } else {
        (total_squared / total_points as f64).sqrt()
    };

    let theta = candidate
        .parameters()
        .into_iter()
        .map(|parameter| parameter.name)
        .zip(outcome.theta)
        .collect();

    BenchReport {
        candidate: candidate.name(),
        status: Status::Success { visible_loss },
        theta,
        visible_loss,
        complexity: candidate.complexity(),
        held_out,
        held_out_rmse,
    }
}

/// The harness-side experiment of one curve: the pack's recorded
/// drives (or the primary channel's staircase), and the measured
/// series on the curve's output channel.
pub(crate) fn experiment(subject: &str, curve: &Curve, lexicon: &Lexicon) -> Experiment {
    let inputs = if curve.drives.is_empty() {
        vec![InputDrive::from_curve(&lexicon.primary_input().name, curve)]
    } else {
        curve.drives.clone()
    };

    let channel = curve
        .output_channel
        .clone()
        .unwrap_or_else(|| lexicon.primary_output().name.clone());

    Experiment {
        subject: subject.to_string(),
        condition: curve.condition.clone(),
        t: curve.raw.t.clone(),
        inputs,
        outputs: vec![OutputSeries {
            channel,
            y: curve.raw.y.clone(),
        }],
    }
}
#[cfg(test)]
mod tests {
    use super::pinned_edge;

    #[test]
    fn wide_log_bounds_do_not_drown_small_free_values() {
        // The binding baseline's real fitted values: none is pinned,
        // although kon and koff are orders of magnitude below the
        // span's linear midpoint.
        assert_eq!(pinned_edge("kon", 1e-12, 1e6, 2.4e-7), None);
        assert_eq!(pinned_edge("koff", 1e-12, 1e6, 0.0048), None);
        assert_eq!(pinned_edge("rmax", 1e-9, 1e9, 1079.0), None);
    }

    #[test]
    fn wide_log_bounds_still_catch_the_edges() {
        let pinned = pinned_edge("kd", 1e-12, 1e6, 1.0001e-12);
        assert!(pinned.unwrap().contains("lower bound 1e-12"));

        let pinned = pinned_edge("kd", 1e-12, 1e6, 9.9999e5);
        assert!(pinned.unwrap().contains("upper bound 1e6"));
    }

    #[test]
    fn narrow_and_nonpositive_bounds_keep_the_linear_rule() {
        assert!(pinned_edge("hill", 0.0, 4.0, 1e-4).unwrap().contains("lower"));
        assert_eq!(pinned_edge("hill", 0.0, 4.0, 2.0), None);
        assert!(pinned_edge("x", -1.0, 1.0, -1.0).unwrap().contains("lower"));
    }
}
