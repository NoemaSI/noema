//! The fixed optimizer driver.
//!
//! The optimizer is a standardized measurement instrument, not part of
//! the scientific search: bounded nonlinear least squares against the
//! fixed sum-of-squared-residuals loss, executed in
//! [`fit.py`](../../fit.py) directly against the candidate's compiled
//! CasADi artifact, using the exact same integrator call the harness
//! uses for scoring. The optimizer may move theta and nothing else.
//!
//! The JSON wire keys are part of the fixed `fit.py` protocol and
//! never change with the pack — they are domain-neutral:
//! each experiment carries its `"subject"`, `"condition"` (display
//! label), observation `"t"` grid, `"inputs"` (one recorded
//! piecewise-constant drive trace per channel:
//! `{"channel", "t", "values"}`) and `"outputs"` (one measured
//! series per channel: `{"channel", "y"}`); the request top level
//! carries `"artifact_dir"`, `"module"`, `"parameters"`, and
//! `"max_nfev"` (the optimizer's budget, a fairness knob identical
//! for every pack). `fit.py` still accepts the legacy
//! `"candidate"`/`"concentration"`/`"t_step"` shape so the original
//! binding harness keeps working.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, anyhow};
use serde_json::json;

use crate::candidate::CandidateModel;
use crate::experiment::Experiment;
use crate::mechanism::MechanismModel;

/// The fitted parameter vector and its fixed loss value.
#[derive(Debug, Clone)]
pub struct FitOutcome {
    pub theta: Vec<f64>,
    pub loss: f64,
}

/// Fit every visible experiment jointly against one theta.
pub fn fit(
    candidate: &CandidateModel,
    experiments: &[Experiment],
    max_nfev: usize,
) -> Result<FitOutcome> {
    let compiled = candidate.compiled();

    let directory = compiled
        .artifact
        .parent()
        .ok_or_else(|| anyhow!("artifact has no parent directory"))?;

    let module = compiled
        .artifact
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| anyhow!("invalid artifact filename"))?;

    let parameters: Vec<_> = candidate
        .parameters()
        .into_iter()
        .map(|parameter| {
            json!({
                "name": parameter.name,
                "start": parameter.start,
                "min": parameter.min,
                "max": parameter.max,
            })
        })
        .collect();

    let experiments_json: Vec<_> = experiments
        .iter()
        .map(|experiment| {
            json!({
                // Fixed, domain-neutral wire keys.
                "subject": experiment.subject,
                "condition": experiment.condition.display(),
                "t": experiment.t,
                "inputs": experiment
                    .inputs
                    .iter()
                    .map(|drive| json!({
                        "channel": drive.channel,
                        "t": drive.t,
                        "values": drive.values,
                    }))
                    .collect::<Vec<_>>(),
                "outputs": experiment
                    .outputs
                    .iter()
                    .map(|series| json!({
                        "channel": series.channel,
                        "y": series.y,
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let request = json!({
        "artifact_dir": directory,
        "module": module,
        "parameters": parameters,
        "max_nfev": max_nfev,
        "experiments": experiments_json,
    });

    let mut child = Command::new("pixi")
        .current_dir(crate::workspace_root())
        .arg("run")
        .arg("python")
        .arg("fit.py")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| anyhow!("spawning fitter: {error}"))?;

    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("failed to open fitter stdin"))?
        .write_all(request.to_string().as_bytes())
        .map_err(|error| anyhow!("sending fit request: {error}"))?;

    let output = child
        .wait_with_output()
        .map_err(|error| anyhow!("waiting for fitter: {error}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "fitter failed (exit {:?})\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        ));
    }

    let response: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| {
            anyhow!(
                "invalid fitter response: {error}\n{}",
                String::from_utf8_lossy(&output.stdout),
            )
        })?;

    match response.get("status").and_then(|value| value.as_str()) {
        Some("success") => Ok(FitOutcome {
            theta: response["theta"]
                .as_array()
                .ok_or_else(|| anyhow!("fitter response missing theta"))?
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .ok_or_else(|| anyhow!("non-numeric theta value"))
                })
                .collect::<Result<_>>()?,
            loss: response["loss"]
                .as_f64()
                .ok_or_else(|| anyhow!("fitter response missing loss"))?,
        }),

        _ => Err(anyhow!(
            "fit failed: {}",
            response["message"]
                .as_str()
                .unwrap_or("unknown fitter error"),
        )),
    }
}