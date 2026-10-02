//! Cross-pack curve scoring: the in-silico sensorgram (Phase D step
//! 3, PLAN_PROTEINBASE.md M3).
//!
//! A law pack B predicts pack A's per-subject parameters from
//! covariates. A fit in θ-space is not yet a law: the acceptance
//! test is what the predicted θ does inside A's mechanism. For every
//! subject held out of B's split, this mode evaluates B's champion
//! law, feeds the predicted θ into A's champion mechanism, simulates
//! the subject's REAL hidden experiments (A's measured drives), and
//! scores the predicted curves against the real curves. That rmse —
//! curves reconstructed for subjects whose curves were never
//! measured — is the headline number; everything else is θ-space
//! bookkeeping.
//!
//! Honesty rule: A hides doses (every subject has hidden curves),
//! B hides binder ids entirely. A subject is scored only if it is
//! hidden in BOTH splits — then neither the law's fit nor the
//! mechanism's fit ever touched that subject — so the score is a
//! genuine unseen-subject generalization of the whole two-pack
//! pipeline. Subjects in B's train/validation rows are skipped and
//! counted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::bench;
use crate::candidate::CandidateModel;
use crate::experiment::Condition;

use crate::mechanism::{MechanismModel, squared_error};
use crate::pack_loader::load_pack;
use crate::plugin::load_candidate;
use crate::split;
use crate::workspace::Workspace;

/// One scored subject: predicted-curve error on its hidden curves.
#[derive(Debug, Clone)]
pub struct SubjectScore {
    pub id: String,
    pub rmse: f64,

    /// Data span (max-min) over the subject's hidden curves: the
    /// scale-free denominator.
    pub span: f64,
    pub norm_rmse: f64,
    pub points: usize,

    /// Parameters whose law prediction fell outside A's declared
    /// bounds and was clamped (count per parameter name).
    pub clamps: Vec<(String, bool)>,
}

/// One predicted θ per subject, in A's parameter-name space.
type PredictedTheta = BTreeMap<String, f64>;

/// Score pack A's hidden curves from pack B's law.
///
/// * `law_gen` / `a_gen`: which champion to use; `None` = champion
///   (law side) / latest (A side).
pub fn curvecheck(
    law_pack_path: &Path,
    law_workspace: &Path,
    law_gen: Option<u32>,
    a_pack_path: &Path,
    a_workspace: &Path,
    a_gen: Option<u32>,
) -> Result<Vec<SubjectScore>> {
    // ---- pack B: the law ------------------------------------------------
    let law_handle = load_pack(law_pack_path)?;

    let law_pack = &*law_handle.pack;
    let lexicon_b = law_pack.lexicon();

    let law_ws = Workspace::new(law_workspace.to_path_buf());

    let law_gen = match law_gen {
        Some(generation) => generation,
        None => law_ws
            .champion()
            .context("law workspace has no champion; pass --law-gen")?
            .generation,
    };

    let law_report = law_ws
        .read_report(law_gen)
        .with_context(|| format!("law workspace gen {law_gen:04} has no report.json"))?;

    let law_theta: PredictedTheta = law_report
        .per_subject
        .iter()
        .find(|subject| subject.status == "Success")
        .or_else(|| law_report.per_subject.first())
        .context("law report has no subject fits")?
        .theta
        .clone()
        .into_iter()
        .collect();

    let law_model = CandidateModel::from_spec(
        load_candidate(&law_ws.plugin_path(law_gen))?
            .as_ref(),
        &lexicon_b,
    )
    .with_context(|| format!("law candidate of gen {law_gen:04} failed to compile"))?;

    // One evaluation experiment per row tag: the constant feature
    // drives are identical across a row's output curves, so the
    // first curve carrying each tag is representative.
    let mut row_experiments: BTreeMap<String, crate::experiment::Experiment> =
        BTreeMap::new();

    for subject in law_pack.subjects()? {
        for curve in &subject.curves {
            if let Condition::Tag(tag) = &curve.condition {
                row_experiments
                    .entry(tag.clone())
                    .or_insert_with(|| bench::experiment(&subject.id, curve, &lexicon_b));
            }
        }
    }

    // Which output channels are log10 targets (units say so).
    let log10_channel = |name: &str| -> bool {
        lexicon_b
            .outputs
            .iter()
            .find(|channel| channel.name == name)
            .map(|channel| channel.units.starts_with("log10"))
            .unwrap_or(false)
    };

    let law_parameter_names: Vec<String> = law_model
        .parameters()
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect();

    let theta_b: Vec<f64> = law_parameter_names
        .iter()
        .map(|name| {
            law_theta.get(name).copied().unwrap_or_else(|| {
                law_model
                    .parameters()
                    .iter()
                    .find(|parameter| &parameter.name == name)
                    .map(|parameter| parameter.start)
                    .unwrap_or(0.0)
            })
        })
        .collect();

    // Predicted θ (A's units) for one subject id.
    let predict = |id: &str| -> Result<PredictedTheta> {
        let experiment = row_experiments
            .get(id)
            .with_context(|| format!("law pack carries no row for subject {id}"))?;

        let simulated = law_model
            .simulate(&theta_b, experiment)
            .map_err(|error| anyhow::anyhow!("law simulation failed for {id}: {error:?}"))?;

        let mut predicted = PredictedTheta::new();

        for channel in &lexicon_b.outputs {
            let value = crate::experiment::simulated_channel(&simulated, &channel.name)
                .and_then(|trace| trace.first().copied())
                .with_context(|| format!("law predicts no `{}` channel", channel.name))?;

            predicted.insert(
                channel.name.clone(),
                if log10_channel(&channel.name) {
                    10f64.powf(value)
                } else {
                    value
                },
            );
        }

        Ok(predicted)
    };

    // ---- pack A: the mechanism ---------------------------------------
    let a_handle = load_pack(a_pack_path)?;

    let a_pack = &*a_handle.pack;
    let lexicon_a = a_pack.lexicon();

    let a_ws = Workspace::new(a_workspace.to_path_buf());

    let a_gen = match a_gen {
        Some(generation) => generation,
        None => a_ws
            .latest_gen()
            .context("A workspace is empty; pass --a-gen")?,
    };

    let a_model = CandidateModel::from_spec(
        load_candidate(&a_ws.plugin_path(a_gen))?.as_ref(),
        &lexicon_a,
    )
    .with_context(|| format!("A candidate of gen {a_gen:04} failed to compile"))?;

    let a_parameters = a_model.parameters();

    // The row tags B held out: only these subjects are unseen by
    // the law; everyone else's predicted θ is an in-sample value.
    let law_hidden: Vec<String> = law_pack
        .subjects()?
        .iter()
        .flat_map(|subject| {
            crate::split::split_roles(&subject.curves, &law_pack.split_policy())
                .hidden
                .into_iter()
                .filter_map(|curve| match curve.condition {
                    Condition::Tag(tag) => Some(tag),
                    Condition::Level(_) => None,
                })
        })
        .collect();

    let dataset = split::split_subjects(a_pack.subjects()?, &a_pack.split_policy());

    let mut scores = Vec::new();

    for subject in &dataset.evaluation {
        if !law_hidden.iter().any(|tag| tag == &subject.id) {
            continue;
        }
        let mut predicted = predict(&subject.id)?;

        let theta: Vec<f64> = a_parameters
            .iter()
            .map(|parameter| {
                let raw = predicted.get(&parameter.name).copied().with_context(|| {
                    format!(
                        "law pack does not predict `{}`, which A's mechanism needs",
                        parameter.name,
                    )
                })?;

                let clamped = raw.clamp(parameter.min, parameter.max);

                if let Some(entry) = predicted.get_mut(&parameter.name) {
                    *entry = clamped;
                }

                Ok(clamped)
            })
            .collect::<Result<_>>()?;

        let clamps = a_parameters
            .iter()
            .map(|parameter| {
                let raw = predicted.get(&parameter.name).copied().unwrap_or(0.0);

                (parameter.name.clone(), (raw - parameter.min).abs() < f64::EPSILON
                    || (raw - parameter.max).abs() < f64::EPSILON)
            })
            .collect();

        let mut squared = 0.0;
        let mut points = 0usize;
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;

        for curve in &subject.held_out_curves {
            let experiment = bench::experiment(&subject.id, curve, &lexicon_a);

            let simulated = a_model
                .simulate(&theta, &experiment)
                .map_err(|error| {
                    anyhow::anyhow!(
                        "A mechanism failed on predicted theta for {}: {error:?}",
                        subject.id,
                    )
                })?;

            let (sse, n) = squared_error(&simulated, &experiment)
                .map_err(|error| anyhow::anyhow!("scoring {}: {error:?}", subject.id))?;

            squared += sse;
            points += n;

            for series in &experiment.outputs {
                for value in &series.y {
                    low = low.min(*value);
                    high = high.max(*value);
                }
            }
        }

        let rmse = (squared / points as f64).sqrt();
        let span = high - low;

        scores.push(SubjectScore {
            id: subject.id.clone(),
            rmse,
            span,
            norm_rmse: if span > 0.0 { rmse / span } else { rmse },
            points,
            clamps,
        });
    }

    Ok(scores)
}

/// CLI entry: `skill curvecheck --law-pack=… --law-workspace=…
/// [--law-gen=N] --pack=… --workspace=… [--a-gen=N] [--out=csv]`.
pub fn run(args: &[String]) -> Result<()> {
    let flag = |prefix: &str| -> Option<String> {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .map(str::to_string)
    };

    let law_pack = PathBuf::from(flag("--law-pack=").context("curvecheck needs --law-pack=<pack.rs>")?);

    let law_workspace =
        PathBuf::from(flag("--law-workspace=").context("curvecheck needs --law-workspace=<dir>")?);

    let a_pack = PathBuf::from(flag("--pack=").context("curvecheck needs --pack=<pack.rs> (A)")?);

    let a_workspace = PathBuf::from(flag("--workspace=").context("curvecheck needs --workspace=<dir> (A)")?);

    let law_gen = flag("--law-gen=").map(|text| text.parse::<u32>()).transpose()?;

    let a_gen = flag("--a-gen=").map(|text| text.parse::<u32>()).transpose()?;

    let scores = curvecheck(
        &law_pack,
        &law_workspace,
        law_gen,
        &a_pack,
        &a_workspace,
        a_gen,
    )?;

    let rmse: Vec<f64> = scores.iter().map(|score| score.rmse).collect();
    let norm: Vec<f64> = scores.iter().map(|score| score.norm_rmse).collect();

    let clamped: usize = scores
        .iter()
        .map(|score| score.clamps.iter().filter(|(_, was)| *was).count())
        .sum();

    println!(
        "skill: curvecheck on {} unseen subjects — median rmse {:.4}, median rmse/span {:.4} \
         ({} bound clamps)",
        scores.len(),
        median(&rmse).unwrap_or(f64::NAN),
        median(&norm).unwrap_or(f64::NAN),
        clamped,
    );

    if let Some(out) = flag("--out=") {
        let path = PathBuf::from(&out);

        write_csv(&scores, &path)?;

        println!("skill: per-subject scores -> {out}");
    }

    Ok(())
}

/// Median helper (input order arbitrary).
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let mut values = values.to_vec();

    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    Some(values[values.len() / 2])
}

/// Write the per-subject scores as CSV.
pub fn write_csv(scores: &[SubjectScore], path: &Path) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)
        .with_context(|| format!("creating curvecheck csv {}", path.display()))?;

    writer.write_record(["id", "rmse", "span", "norm_rmse", "points", "clamped"])?;

    for score in scores {
        let clamped: Vec<&str> = score
            .clamps
            .iter()
            .filter(|(_, was)| *was)
            .map(|_| "1")
            .collect();

        writer.write_record(&[
            score.id.clone(),
            format!("{:.6}", score.rmse),
            format!("{:.6}", score.span),
            format!("{:.6}", score.norm_rmse),
            score.points.to_string(),
            if clamped.is_empty() {
                String::new()
            } else {
                clamped.len().to_string()
            },
        ])?;
    }

    writer.flush()?;

    Ok(())
}