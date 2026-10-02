//! The candidate-mechanism contract.
//!
//! A mechanism is an executable claim about how the world behaves: the
//! harness may only move its numerical parameters, never its
//! structure. This is the same boundary as everywhere else — model
//! author controls structure, harness controls reality — and it is
//! what makes representations *mechanistically interpretable*: the
//! structure stays inspectable (`complexity`, the source itself) while
//! the optimizer stays a blind, standardized measurement instrument.

use crate::experiment::{Experiment, Parameter, SimError, Simulated, simulated_channel};

/// Structure bookkeeping recorded by the fixed complexity policy: fit
/// quality alone rewards unlimited flexibility, so structure size is
/// always reported next to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Complexity {
    pub states: usize,
    pub parameters: usize,
    pub equations: usize,
}

/// An executable mechanism the harness can fit and score.
pub trait MechanismModel {
    fn name(&self) -> String;

    /// The parameters the fixed optimizer may move, in theta order.
    fn parameters(&self) -> Vec<Parameter>;

    /// Predict every measured output channel on the experiment's
    /// time grid, replaying the experiment's recorded drive traces
    /// (piecewise-constant between their samples — data, never
    /// fitted).
    fn simulate(
        &self,
        theta: &[f64],
        experiment: &Experiment,
    ) -> Result<Simulated, SimError>;

    fn complexity(&self) -> Complexity;
}

/// Squared error and point count of a simulated response against the
/// experiment's measured series: the fixed loss's per-experiment
/// kernel. Predictions are matched to measurements by channel name;
/// a measured channel the model fails to predict is an error.
pub fn squared_error(
    simulated: &Simulated,
    experiment: &Experiment,
) -> Result<(f64, usize), SimError> {
    let mut squared = 0.0;
    let mut points = 0usize;

    for series in &experiment.outputs {
        let predicted = simulated_channel(simulated, &series.channel).ok_or_else(|| {
            SimError::Invalid(format!(
                "model predicts no `{}` channel",
                series.channel,
            ))
        })?;

        for (predicted, observed) in predicted.iter().zip(&series.y) {
            let residual = predicted - observed;

            squared += residual * residual;
            points += 1;
        }
    }

    Ok((squared, points))
}

/// Fixed loss: unweighted sum of squared residuals over every point
/// of every measured output channel of all given experiments.
/// Specified once; nothing else uses another objective during
/// fitting.
pub fn sum_squared_residuals(
    model: &dyn MechanismModel,
    theta: &[f64],
    experiments: &[Experiment],
) -> Result<f64, SimError> {
    let mut loss = 0.0;

    for experiment in experiments {
        let simulated = model.simulate(theta, experiment)?;

        let (squared, _) = squared_error(&simulated, experiment)?;

        loss += squared;
    }

    Ok(loss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::{Condition, OutputSeries, Parameter};

    struct Constant(f64);

    impl MechanismModel for Constant {
        fn name(&self) -> String {
            "constant".to_string()
        }

        fn parameters(&self) -> Vec<Parameter> {
            vec![]
        }

        fn simulate(
            &self,
            _theta: &[f64],
            experiment: &Experiment,
        ) -> Result<Simulated, SimError> {
            let values =
                crate::experiment::sanitize(&vec![self.0; experiment.t.len()], experiment.t.len())?;

            Ok(vec![("signal".to_string(), values)])
        }

        fn complexity(&self) -> Complexity {
            Complexity {
                states: 0,
                parameters: 0,
                equations: 0,
            }
        }
    }

    #[test]
    fn fixed_loss_is_sum_of_squared_residuals() {
        let experiment = Experiment {
            subject: "x".to_string(),
            condition: Condition::Level(1.0),
            t: vec![0.0, 1.0, 2.0],
            inputs: vec![],
            outputs: vec![OutputSeries {
                channel: "signal".to_string(),
                y: vec![1.0, 2.0, 3.0],
            }],
        };

        let loss = sum_squared_residuals(&Constant(2.0), &[], &[experiment])
            .expect("constant model runs");

        // (1-2)^2 + (2-2)^2 + (3-2)^2 = 2
        assert_eq!(loss, 2.0);
    }
}