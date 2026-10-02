//! Diagnostic plots as images: the author sees the same picture the
//! harness measures.
//!
//! [`multi_diagnostic_plot_png`] simulates a mechanism per subject
//! (each subject gets its own frozen parameters), overlays the
//! measured curves, and renders the result to PNG through the
//! repository's pixi environment (matplotlib). Each subject gets one
//! row — its own auto-scaled overlay panel plus a residual strip — so
//! a small-signal subject's gaps stay as visible as a big-signal
//! subject's. The PNG lands in the generation directory as `plot.png`
//! and is attached to the theorist's and the referee's prompts as an
//! image. Axis labels come from the pack's lexicon.

use std::process::Command;

use anyhow::{Context, Result};
use base64::Engine as _;
use serde::Serialize;

use crate::candidate::{CandidateModel, CandidateSpec};
use crate::data::Curve;
use crate::experiment::{Condition, Experiment, InputDrive, simulated_channel};
use crate::lexicon::Lexicon;
use crate::lower::lower;
use crate::mechanism::MechanismModel;
use crate::mechanism_ir::Mechanism;

/// Name of the diagnostic image written into the generation directory.
pub const PLOT_PNG_NAME: &str = "plot.png";

/// Name of the candidate's own diagnostic image (its fitted mechanism
/// simulated against the measured curves), rendered after evaluation
/// for the referee's promotion decision. `plot.png` in the same
/// directory stays the champion's trajectories — the theorist's input.
pub const CANDIDATE_PLOT_PNG_NAME: &str = "candidate_plot.png";

/// Simulation samples per condition (measured curves keep their
/// full resolution as an overlay).
pub const PLOT_POINTS: usize = 240;

/// One rendered curve: a polyline plus its legend entry.
#[derive(Debug, Clone, Serialize)]
struct PlotCurve {
    x: Vec<f64>,
    y: Vec<f64>,
    label: String,
    dotted: bool,

    /// Points, not a polyline: the parity diagnostic draws
    /// one scatter series per measured channel.
    #[serde(default)]
    scatter: bool,
}

/// One subject's row: an overlay panel plus its residual strip.
#[derive(Debug, Serialize)]
struct Panel {
    title: String,
    curves: Vec<PlotCurve>,
    residuals: Vec<PlotCurve>,

    /// Axis-label overrides (the parity plot's axes are
    /// measured/predicted, not time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    xlabel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ylabel: Option<String>,
}

#[derive(Debug, Serialize)]
struct PlotSpec {
    title: String,
    ylabel: String,
    panels: Vec<Panel>,
}

/// One subject's share of a composite figure: its id (the row title),
/// its frozen fit, and its measured curves.
pub struct SubjectPanel<'a> {
    pub id: String,
    pub theta: &'a [(String, f64)],
    pub curves: &'a [Curve],
}

/// The mechanism as an in-memory candidate source, so the plot can
/// reuse the exact compile/simulate path the harness evaluates on.
struct PlotCandidate {
    name: String,
    modelica: String,
}

impl CandidateSpec for PlotCandidate {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn modelica(&self) -> String {
        self.modelica.clone()
    }
}

/// Base64 (standard alphabet) for prompt attachment.
pub fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// One subject per panel: simulate `mechanism` at each condition
/// with that subject's own frozen `theta`, overlay the measured
/// curves, and render one stacked row per subject — an auto-scaled
/// overlay panel with a residual strip (sim − measured) beneath it.
///
/// Every `theta` is a frozen fit (name/value pairs); parameters
/// missing from it fall back to their declared starts.
pub fn multi_diagnostic_plot_png(
    mechanism: &Mechanism,
    lexicon: &Lexicon,
    panels: &[SubjectPanel<'_>],
    conditions: &[Condition],
) -> Result<Vec<u8>> {
    let lowered = lower(mechanism, lexicon, "diagnostic plot")
        .map_err(|errors| anyhow::anyhow!("plot lowering failed:\n{errors}"))?;

    let spec = PlotCandidate {
        name: mechanism.name.clone(),
        modelica: lowered.modelica,
    };

    let model = CandidateModel::from_spec(&spec, lexicon)
        .context("compiling mechanism for plot")?;

    let input = lexicon.primary_input();
    let output = lexicon.primary_output();

    // Drives for replaying a curve on the simulation grid: the
    // curve's recorded traces, else its staircase on the primary
    // channel; every other declared channel is held at the condition
    // level (or zero for tags).
    let drives_for = |curve: &Curve, condition: &Condition| -> Vec<InputDrive> {
        let mut drives = if curve.drives.is_empty() {
            vec![InputDrive::from_curve(&input.name, curve)]
        } else {
            curve.drives.clone()
        };

        for channel in &lexicon.inputs {
            if !drives.iter().any(|drive| drive.channel == channel.name) {
                drives.push(InputDrive::constant(
                    &channel.name,
                    condition.level().unwrap_or(0.0),
                ));
            }
        }

        drives
    };

    let condition_label = |condition: &Condition| match condition.level() {
        Some(_) => format!("{}={} {}", input.name, condition.display(), input.units),
        None => format!("{}={}", input.name, condition.display()),
    };

    let mut rendered = Vec::new();

    for panel in panels {
        let theta_map: std::collections::HashMap<&str, f64> = panel
            .theta
            .iter()
            .map(|(name, value)| (name.as_str(), *value))
            .collect();

        let values: Vec<f64> = model
            .parameters()
            .iter()
            .map(|parameter| {
                theta_map
                    .get(parameter.name.as_str())
                    .copied()
                    .unwrap_or(parameter.start)
            })
            .collect();

        let mut plot_curves = Vec::new();
        let mut residuals = Vec::new();

        for condition in conditions {
            let measured: Vec<&Curve> = panel
                .curves
                .iter()
                .filter(|curve| curve.condition.matches(condition))
                .collect();

            // The measured span, never more: a fixed seed would extend the
            // simulation far past short records (600 was a binding
            // assumption); it is only a fallback for conditions
            // without any measured curve.
            let t_end = measured
                .iter()
                .flat_map(|curve| curve.raw.t.iter().copied())
                .fold(0.0_f64, f64::max)
                .max(1.0)
                .max(if measured.is_empty() { 600.0 } else { 0.0 });

            let t: Vec<f64> = (0..=PLOT_POINTS)
                .map(|step| t_end * step as f64 / PLOT_POINTS as f64)
                .collect();

            // Diagnostic overlay: replay the protocol staircase when
            // the measured curves carry one (sessions can differ;
            // the latest detected step keeps the first phase
            // visible for every overlay, with that curve's own
            // post-step drive level).
            let protocol = measured
                .iter()
                .filter(|curve| curve.t_step.is_some())
                .max_by(|a, b| {
                    a.t_step
                        .unwrap_or(f64::MIN)
                        .total_cmp(&b.t_step.unwrap_or(f64::MIN))
                })
                .or_else(|| measured.first());

            let drives = match protocol {
                Some(curve) => drives_for(curve, condition),
                None => vec![InputDrive::constant(
                    &input.name,
                    condition.level().unwrap_or(0.0),
                )],
            };

            let experiment = Experiment {
                subject: panel.id.clone(),
                condition: condition.clone(),
                t: t.clone(),
                inputs: drives,
                outputs: vec![],
            };

            let simulated = model
                .simulate(&values, &experiment)
                .map_err(|error| anyhow::anyhow!("plot simulation at {}: {error:?}", condition_label(condition)))?;

            let signal = simulated_channel(&simulated, &output.name)
                .ok_or_else(|| anyhow::anyhow!("model does not output `{}`", output.name))?
                .to_vec();

            plot_curves.push(PlotCurve {
                x: t,
                y: signal,
                label: format!("sim {}", condition_label(condition)),
                dotted: false,
                scatter: false,
            });

            for curve in measured {
                plot_curves.push(PlotCurve {
                    x: curve.raw.t.clone(),
                    y: curve.raw.y.clone(),
                    label: format!(
                        "measured {} #{}",
                        condition_label(condition),
                        curve.session + 1,
                    ),
                    dotted: true,
                    scatter: false,
                });

                // Residual strip: replay the sim on the measured
                // curve's own time grid and subtract, so each residual
                // point pairs one simulated and one measured sample.
                let replay = Experiment {
                    subject: panel.id.clone(),
                    condition: curve.condition.clone(),
                    t: curve.raw.t.clone(),
                    inputs: drives_for(curve, condition),
                    outputs: vec![],
                };

                match model.simulate(&values, &replay) {
                    Ok(simulated) => {
                        let Some(predicted) = simulated_channel(&simulated, &output.name) else {
                            continue;
                        };

                        residuals.push(PlotCurve {
                            x: curve.raw.t.clone(),
                            y: predicted
                                .iter()
                                .zip(&curve.raw.y)
                                .map(|(predicted, observed)| predicted - observed)
                                .collect(),
                            label: condition_label(condition),
                            dotted: false,
                            scatter: false,
                        })
                    }
                    // A residual replay failing must not lose the
                    // whole panel; the overlay above already carries
                    // the fit.
                    Err(_) => continue,
                }
            }
        }

        let theta_text: Vec<String> = values
            .iter()
            .zip(model.parameters())
            .map(|(value, parameter)| format!("{}={value:.2e}", parameter.name))
            .collect();

        rendered.push(Panel {
            title: format!("{} — {}", panel.id, theta_text.join(", ")),
            curves: plot_curves,
            residuals,
            xlabel: None,
            ylabel: None,
        });
    }

    render_png(&PlotSpec {
        title: mechanism.name.clone(),
        ylabel: format!("{} ({})", output.name, output.units),
        panels: rendered,
    })
}

/// Parity diagnostic for law packs: the "time" axis is a covariate
/// placeholder, so the time-series overlay says nothing (hundreds of
/// flat lines, one legend entry per row). The informative picture is
/// predicted vs measured, one scatter panel per output channel, with
/// the diagonal and a residual strip. Rows are sampled (deterministic
/// stride) to bound the simulation cost of the figure.
pub fn parity_diagnostic_plot_png(
    mechanism: &Mechanism,
    lexicon: &Lexicon,
    panels: &[SubjectPanel<'_>],
) -> Result<Vec<u8>> {
    let lowered = lower(mechanism, lexicon, "parity plot")
        .map_err(|errors| anyhow::anyhow!("plot lowering failed:\n{errors}"))?;

    let model = CandidateModel::from_spec(
        &PlotCandidate {
            name: mechanism.name.clone(),
            modelica: lowered.modelica,
        },
        lexicon,
    )
    .context("compiling mechanism for plot")?;

    const PARITY_MAX_ROWS: usize = 150;

    let mut points: std::collections::BTreeMap<String, (Vec<f64>, Vec<f64>)> =
        std::collections::BTreeMap::new();

    let mut totals: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for panel in panels {
        let theta_map: std::collections::HashMap<&str, f64> = panel
            .theta
            .iter()
            .map(|(name, value)| (name.as_str(), *value))
            .collect();

        let values: Vec<f64> = model
            .parameters()
            .iter()
            .map(|parameter| {
                theta_map
                    .get(parameter.name.as_str())
                    .copied()
                    .unwrap_or(parameter.start)
            })
            .collect();

        // One law row = one condition tag; one simulation predicts
        // every output channel of that row.
        let mut rows: std::collections::BTreeMap<String, Vec<&Curve>> =
            std::collections::BTreeMap::new();

        for curve in panel.curves {
            rows.entry(curve.condition.display())
                .or_default()
                .push(curve);
        }

        let total = rows.len();

        let step = (total / PARITY_MAX_ROWS).max(1);

        // One simulation per row (all channels at once). Each
        // simulate is an independent `pixi run python` subprocess —
        // start-up dominated and process-safe — so the rows are
        // simulated across a small pool of threads instead of one
        // at a time.
        let jobs: Vec<Vec<&Curve>> = rows
            .values()
            .enumerate()
            .filter(|(index, _)| index % step == 0)
            .map(|(_, group)| group.clone())
            .collect();

        let threads = jobs.len().clamp(1, 6);

        let outcomes: Vec<Vec<(&Vec<&Curve>, Option<crate::experiment::Simulated>)>> =
            std::thread::scope(|scope| {
                let mut handles = Vec::new();

                let model = &model;
                let values = &values;
                let panel_id = &panel.id;

                for chunk in jobs.chunks(jobs.len().div_ceil(threads)) {
                    let chunk: &[Vec<&Curve>] = chunk;

                    handles.push(scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|group| {
                                let first = group[0];

                                let mut drives = first.drives.clone();

                                for channel in &lexicon.inputs {
                                    if !drives.iter().any(|drive| drive.channel == channel.name)
                                    {
                                        drives.push(InputDrive::constant(&channel.name, 0.0));
                                    }
                                }

                                let experiment = Experiment {
                                    subject: panel_id.clone(),
                                    condition: first.condition.clone(),
                                    t: if first.raw.t.len() >= 2 {
                                        first.raw.t.clone()
                                    } else {
                                        vec![0.0, 1.0]
                                    },
                                    inputs: drives,
                                    outputs: vec![],
                                };

                                (group, model.simulate(&values, &experiment).ok())
                            })
                            .collect::<Vec<_>>()
                    }));
                }

                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect()
            });

        for (group, simulated) in outcomes.into_iter().flatten() {
            let Some(simulated) = simulated else { continue };

            for curve in group {
                let channel = curve
                    .output_channel
                    .clone()
                    .unwrap_or_else(|| lexicon.primary_output().name.clone());

                let Some(measured) = curve.raw.y.first().copied() else {
                    continue;
                };

                let Some(Some(predicted)) =
                    simulated_channel(&simulated, &channel).map(|trace| trace.first().copied())
                else {
                    continue;
                };

                *totals.entry(channel.clone()).or_default() += 1;

                let entry = points.entry(channel).or_default();

                entry.0.push(measured);
                entry.1.push(predicted);
            }
        }
    }

    let mut rendered = Vec::new();

    for channel in &lexicon.outputs {
        let Some((measured, predicted)) = points.get(&channel.name) else {
            continue;
        };

        if measured.is_empty() {
            continue;
        }

        let lo = measured
            .iter()
            .chain(predicted)
            .fold(f64::INFINITY, |a, b| a.min(*b));

        let hi = measured
            .iter()
            .chain(predicted)
            .fold(f64::NEG_INFINITY, |a, b| a.max(*b));

        let rmse = (measured
            .iter()
            .zip(predicted)
            .map(|(measured, predicted)| (predicted - measured).powi(2))
            .sum::<f64>()
            / measured.len() as f64)
            .sqrt();

        rendered.push(Panel {
            title: format!(
                "{}: predicted vs measured (n={}/{}, rmse={rmse:.3})",
                channel.name,
                measured.len(),
                totals.get(&channel.name).copied().unwrap_or(0),
            ),
            curves: vec![
                PlotCurve {
                    x: vec![lo, hi],
                    y: vec![lo, hi],
                    label: "y = x".to_string(),
                    dotted: true,
                    scatter: false,
                },
                PlotCurve {
                    x: measured.clone(),
                    y: predicted.clone(),
                    label: format!("{} ({})", channel.name, channel.units),
                    dotted: false,
                    scatter: true,
                },
            ],
            residuals: vec![PlotCurve {
                x: measured.clone(),
                y: measured
                    .iter()
                    .zip(predicted)
                    .map(|(measured, predicted)| predicted - measured)
                    .collect(),
                label: "pred−meas".to_string(),
                dotted: false,
                scatter: true,
            }],
            xlabel: Some(format!("measured {} ({})", channel.name, channel.units)),
            ylabel: Some(format!("predicted ({})", channel.units)),
        });
    }

    if rendered.is_empty() {
        anyhow::bail!("parity plot: the mechanism predicted nothing on the given rows");
    }

    render_png(&PlotSpec {
        title: format!("{} — parity", mechanism.name),
        ylabel: String::new(),
        panels: rendered,
    })
}
/// Single-subject convenience: one panel titled by the mechanism name.
pub fn diagnostic_plot_png(
    mechanism: &Mechanism,
    lexicon: &Lexicon,
    theta: &[(String, f64)],
    curves: &[Curve],
    conditions: &[Condition],
) -> Result<Vec<u8>> {
    multi_diagnostic_plot_png(
        mechanism,
        lexicon,
        &[SubjectPanel {
            id: mechanism.name.clone(),
            theta,
            curves,
        }],
        conditions,
    )
}

/// One measured-only polyline for a data preview (no model).
pub struct DataCurve {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub label: String,
}

/// Data-only figure for pack authoring: measured curves, one panel
/// per subject, no simulation and no residuals — the interview's
/// "is this what you measured?" check, rendered on the same path as
/// the diagnostic plots.
pub fn data_preview_png(
    title: &str,
    ylabel: &str,
    panels: &[(String, Vec<DataCurve>)],
) -> Result<Vec<u8>> {
    let rendered = panels
        .iter()
        .map(|(panel_title, curves)| Panel {
            title: panel_title.clone(),
            curves: curves
                .iter()
                .map(|curve| PlotCurve {
                    x: curve.x.clone(),
                    y: curve.y.clone(),
                    label: curve.label.clone(),
                    dotted: false,
                    scatter: false,
                })
                .collect(),
            residuals: vec![],
            xlabel: None,
            ylabel: None,
        })
        .collect();

    render_png(&PlotSpec {
        title: title.to_string(),
        ylabel: ylabel.to_string(),
        panels: rendered,
    })
}

/// Render a plot spec with matplotlib through `pixi run python`.
fn render_png(spec: &PlotSpec) -> Result<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("skill-plot-{}", std::process::id()));

    std::fs::create_dir_all(&dir).context("creating plot temp dir")?;

    let spec_path = dir.join("plot.json");
    let png_path = dir.join("plot.png");

    std::fs::write(&spec_path, serde_json::to_string(spec)?)
        .context("writing plot spec")?;

    let output = Command::new("pixi")
        .current_dir(crate::workspace_root())
        .args(["run", "python", "-c", MATPLOTLIB_SCRIPT])
        .arg(&spec_path)
        .arg(&png_path)
        .output()
        .context("spawning `pixi run python` for the plot — is pixi on PATH?")?;

    if !output.status.success() {
        anyhow::bail!(
            "matplotlib render failed (exit {:?}):\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    std::fs::read(&png_path).with_context(|| format!("reading {}", png_path.display()))
}

const MATPLOTLIB_SCRIPT: &str = r#"
import json, sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

spec = json.load(open(sys.argv[1]))
panels = spec["panels"]
n = len(panels)

fig = plt.figure(figsize=(10, 3.4 * n), dpi=110)
grid = fig.add_gridspec(n, 1, hspace=0.5)
fig.suptitle(spec["title"], fontsize=11)

for i, panel in enumerate(panels):
    inner = grid[i].subgridspec(2, 1, height_ratios=[3, 1], hspace=0.06)
    ax = fig.add_subplot(inner[0])

    for curve in panel["curves"]:
        if curve.get("scatter"):
            ax.scatter(
                curve["x"], curve["y"],
                s=9, alpha=0.55, linewidths=0, label=curve["label"],
            )
        else:
            ax.plot(
                curve["x"], curve["y"],
                linestyle="--" if curve["dotted"] else "-",
                linewidth=1.0 if curve["dotted"] else 1.6,
                label=curve["label"],
            )

    ax.set_title(panel["title"], fontsize=9)
    ax.set_ylabel(panel.get("ylabel") or spec["ylabel"], fontsize=8)

    if len(panel["curves"]) <= 20:
        ax.legend(fontsize=7)

    ax.grid(alpha=0.3)

    if panel["residuals"]:
        axr = fig.add_subplot(inner[1], sharex=ax)

        for curve in panel["residuals"]:
            if curve.get("scatter"):
                axr.scatter(
                    curve["x"], curve["y"],
                    s=7, alpha=0.55, linewidths=0, label=curve["label"],
                )
            else:
                axr.plot(
                    curve["x"], curve["y"],
                    linestyle="-", linewidth=0.9, label=curve["label"],
                )

        axr.axhline(0, color="black", linewidth=0.6)
        axr.set_ylabel("sim−meas", fontsize=7)
        axr.tick_params(labelsize=7)
        axr.grid(alpha=0.3)
        plt.setp(ax.get_xticklabels(), visible=False)
        last = axr
    else:
        ax.tick_params(labelsize=8)
        last = ax

    if i == n - 1:
        last.set_xlabel(panel.get("xlabel") or "time", fontsize=8)

fig.savefig(sys.argv[2])
"#;
// ---------------------------------------------------------------- plot CLI

/// `skill plot`: the old `binding plot` for any pack. Simulate one
/// generation's mechanism (its own `mechanism.json`, the frozen theta
/// from its `report.json`) at chosen conditions and open an
/// interactive Plotly page with the measured curves overlaid.
///
/// ```text
/// skill plot --pack=packs/bactgrowth_auto/pack.rs --subject=D --gen=2 \
///     --condition=0,31.25,250 [--t-end=30] [--out=plot.html]
/// ```
///
/// Without `--condition=`, every condition the subject was measured
/// at is plotted; without `--gen=`, the champion generation.
pub fn plot_command(args: &[String]) -> Result<(), String> {
    use crate::workspace::Workspace;

    let flag = |name: &str| {
        args.iter()
            .find_map(|arg| arg.strip_prefix(&format!("--{name}=")))
            .map(str::to_string)
    };

    let pack_path = flag("pack").ok_or(concat!(
        "usage: skill plot --pack=<pack.rs> [--subject=<id>] [--gen=N] ",
        "[--condition=a,b] [--t-end=X] [--workspace=dir] [--out=file.html]\n",
        "       (list the subject ids: skill inspect --pack=<pack.rs>)"
    ))?;

    let handle = crate::pack_loader::load_pack(std::path::Path::new(&pack_path))
        .map_err(|error| format!("{error:#}"))?;

    let lexicon = handle.pack.lexicon();

    let subjects = handle
        .pack
        .subjects()
        .map_err(|error| format!("loading subjects: {error:#}"))?;

    let wanted = flag("subject");

    let subject = match &wanted {
        Some(id) => subjects.iter().find(|s| &s.id == id).ok_or_else(|| {
            format!(
                "no subject `{id}` — the pack has: {}",
                subjects
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })?,
        None => subjects.first().ok_or("the pack has no subjects")?,
    };

    let workspace = Workspace::new(
        flag("workspace")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                crate::workspace_root()
                    .join("workspace")
                    .join(handle.pack.name())
            }),
    );

    let generation = match flag("gen").map(|text| text.parse::<u32>()) {
        Some(Ok(generation)) => Some(generation),
        Some(Err(_)) => return Err("`--gen=` wants a number".to_string()),
        None => workspace
            .champion()
            .map(|champion| champion.generation)
            .or_else(|| workspace.latest_gen()),
    }
    .ok_or("workspace has no generations to plot")?;

    // The generation's own artifact; a workspace seeded by hand has
    // only the baseline mechanism.
    let mechanism_path = workspace.mechanism_path(generation);

    let mechanism_path = if mechanism_path.exists() {
        mechanism_path
    } else {
        handle.pack.baseline().mechanism
    };

    let text = std::fs::read_to_string(&mechanism_path)
        .map_err(|error| format!("reading {}: {error}", mechanism_path.display()))?;

    let mechanism: Mechanism = serde_json::from_str(&text)
        .map_err(|error| format!("parsing {}: {error}", mechanism_path.display()))?;

    let lowered = lower(&mechanism, &lexicon, "interactive plot")
        .map_err(|errors| format!("lowering gen {generation:04}:\n{errors}"))?;

    let spec = PlotCandidate {
        name: mechanism.name.clone(),
        modelica: lowered.modelica,
    };

    let model = CandidateModel::from_spec(&spec, &lexicon)
        .map_err(|error| format!("compiling gen {generation:04}: {error:?}"))?;

    // Frozen theta of the evaluated generation, in the compiled
    // model's parameter order; declared starts fill the gaps.
    let theta_map: std::collections::HashMap<String, f64> = workspace
        .read_report(generation)
        .and_then(|report| {
            report
                .per_subject
                .iter()
                .find(|s| s.subject == subject.id)
                .map(|s| s.theta.iter().cloned().collect())
        })
        .unwrap_or_default();

    let values: Vec<f64> = model
        .parameters()
        .iter()
        .map(|parameter| {
            theta_map
                .get(&parameter.name)
                .copied()
                .unwrap_or(parameter.start)
        })
        .collect();

    let input = lexicon.primary_input();
    let output = lexicon.primary_output();

    let conditions: Vec<Condition> = match flag("condition") {
        Some(spec_text) => spec_text
            .split(',')
            .map(|token| crate::author::parse_condition(token.trim()))
            .collect(),
        None => {
            let mut seen: Vec<String> = Vec::new();
            let mut known: Vec<Condition> = Vec::new();

            for curve in &subject.curves {
                let display = curve.condition.display();

                if !seen.contains(&display) {
                    seen.push(display);
                    known.push(curve.condition.clone());
                }
            }

            known
        }
    };

    let t_end_override = flag("t-end").and_then(|text| text.parse::<f64>().ok());

    let mut traces = Vec::new();

    for condition in &conditions {
        let measured: Vec<&Curve> = subject
            .curves
            .iter()
            .filter(|curve| curve.condition.matches(condition))
            .collect();

        let t_end = t_end_override.unwrap_or_else(|| {
            measured
                .iter()
                .flat_map(|curve| curve.raw.t.iter().copied())
                .fold(0.0_f64, f64::max)
                .max(1.0)
                .max(if measured.is_empty() { 600.0 } else { 0.0 })
        });

        let t: Vec<f64> = (0..=PLOT_POINTS)
            .map(|step| t_end * step as f64 / PLOT_POINTS as f64)
            .collect();

        // Same replay convention as the diagnostic PNG: recorded
        // drives verbatim, else the staircase/constant on the
        // primary channel; other channels held at the condition level.
        let drives = match measured.iter().find(|curve| !curve.drives.is_empty()) {
            Some(curve) => {
                let mut drives = curve.drives.clone();

                for channel in &lexicon.inputs {
                    if !drives.iter().any(|drive| drive.channel == channel.name) {
                        drives.push(InputDrive::constant(
                            &channel.name,
                            condition.level().unwrap_or(0.0),
                        ));
                    }
                }

                drives
            }
            None => vec![InputDrive::constant(
                &input.name,
                condition.level().unwrap_or(0.0),
            )],
        };

        let experiment = Experiment {
            subject: subject.id.clone(),
            condition: condition.clone(),
            t: t.clone(),
            inputs: drives,
            outputs: vec![],
        };

        let simulated = model
            .simulate(&values, &experiment)
            .map_err(|error| format!("simulating {}: {error:?}", condition.display()))?;

        let signal = simulated_channel(&simulated, &output.name)
            .ok_or_else(|| format!("model does not output `{}`", output.name))?
            .to_vec();

        traces.push(serde_json::json!({
            "x": t, "y": signal, "mode": "lines",
            "name": format!("sim {}", condition.display()),
        }));

        for curve in &measured {
            traces.push(serde_json::json!({
                "x": curve.raw.t, "y": curve.raw.y, "mode": "markers",
                "marker": { "size": 3, "opacity": 0.6 },
                "name": format!("measured {} #{}", condition.display(), curve.session + 1),
            }));
        }
    }

    let page = INTERACTIVE_TEMPLATE.replace(
        "__DATA__",
        &serde_json::to_string(&serde_json::json!({
            "model": mechanism.name,
            "subject": subject.id,
            "generation": generation,
            "theta": theta_map,
            "xlabel": "time",
            "ylabel": format!("{} ({})", output.name, output.units),
            "traces": traces,
        }))
        .map_err(|error| error.to_string())?,
    );

    let out = flag("out")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace.gen_dir(generation).join("plot.html"));

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    std::fs::write(&out, page).map_err(|error| format!("writing {}: {error}", out.display()))?;

    println!("skill: plot wrote {}", out.display());

    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };

    if std::process::Command::new(opener).arg(&out).spawn().is_err() {
        println!("skill: could not launch `{opener}` — open the file manually");
    }

    Ok(())
}

const INTERACTIVE_TEMPLATE: &str = r###"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>skill plot</title>
<script src="https://cdn.plot.ly/plotly-2.35.2.min.js"></script>
</head>
<body>
<div id="plot" style="width:100vw;height:96vh"></div>
<script>
const D = __DATA__;
const theta = Object.entries(D.theta).map(([k, v]) => k + "=" + v.toExponential(3)).join(",  ");
Plotly.newPlot("plot", D.traces, {
  title: D.model + " — subject " + D.subject + " (gen " + D.generation + ")  " + theta,
  xaxis: { title: D.xlabel },
  yaxis: { title: D.ylabel },
  legend: { orientation: "h" },
}, { responsive: true });
</script>
</body>
</html>
"###;
