//! The evolution loop's author-side contract: what the theorist sees,
//! proposes, and what happens to the proposals.
//!
//! The loop is a tournament of mechanisms, and the author is a
//! *theorist*: it reasons over diagnostics, the ledger, and the raw
//! curves, and answers with three structured mechanism ideas (the
//! [`crate::mechanism_ir`] JSON) — never code:
//!
//! ```text
//! champion gen ──copy──▶ gen/N/mechanism.json
//!                            │  theorize(): propose 3 ideas as IR, pick one
//!                            ▼
//!                     lower(): IR → Modelica → generated plugin.rs
//!                            │
//!                     evaluate (compile → fit train → score validation)
//!                            │
//!                     judge(): the referee compares the candidate's
//!                     simulated trajectories with the champion's and
//!                     decides — promote or discard
//!                            │
//!          promoted ─────────┴─ discarded
//!             champion = N       next gen copies the old champion
//! ```
//!
//! Selection is a judgment call, not a scalar: after evaluation the
//! referee ([`judge`]) sees both mechanisms' IR and both
//! simulated-vs-measured trajectory plots and decides whether the
//! candidate becomes champion. The validation RMSE stays in the
//! report and on the console for the record only.
//!
//! The theorist has no tools and one turn; all data is inlined in its
//! prompt and its output is parsed, validated, and recorded in the
//! ledger ([`crate::ledger`]) — including the two proposals that were
//! *not* implemented, so future generations can resurrect them.
//!
//! Selection pressure lives entirely on visible data (train /
//! validation input levels); hidden-set scores exist only in the
//! evaluator's [`crate::workspace::GenReport::evaluator_only`] and are
//! stripped from every structure this module hands to the author —
//! and from everything the referee sees.
//!
//! All prompt text is split in two: the static, domain-generic
//! sections produced here (IR contract, judging rules), and the pack's
//! domain text appended by the driver (`pack.theorist_preamble()` et
//! al.). The only vocabulary the static text knows is the pack's
//! lexicon.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::{self, LlmConfig};
use crate::data::Curve;
use crate::ledger::Mode;
use crate::lexicon::Lexicon;
use crate::mechanism::Complexity;
use crate::mechanism_ir::{self, Mechanism};
use crate::workspace::GenReport;

/// Reduced-sample point budget per curve in the author's `curves.csv`.
/// The raw records may carry thousands of points; 42 points keep the
/// kinetic shape readable while staying agent-digestible.
pub const OBSERVATION_CSV_POINTS: usize = 42;

/// Name of the measurement file written into the generation directory.
pub const OBSERVATION_CSV_NAME: &str = "curves.csv";

/// Header of the author's measurement CSV (the columns the theorist
/// prompt describes).
pub const OBSERVATION_CSV_HEADER: &str = "subject,curve,input,time_s,signal";

/// Number of mechanism ideas the theorist must propose per generation.
pub const PROPOSALS_PER_GENERATION: usize = 3;

/// Per-subject outcome of the parent generation, author-safe.
#[derive(Debug, Clone)]
pub struct SubjectDiagnostics {
    pub subject: String,

    /// "Success", "SimulationFailed", ...
    pub status: String,

    /// Fitted parameters of the frozen theta.
    pub theta: Vec<(String, f64)>,

    pub train_loss: f64,

    /// Visible validation RMSE — the selection component.
    pub validation_rmse: f64,

    /// Sentinel: parameters stuck at their declared bounds (the
    /// optimizer cannot follow the data — widen the bound or change
    /// the form).
    pub pinned: Vec<String>,

    /// Sentinel: mechanism condition-response spread / data
    /// condition-response spread on validation levels. Far below 1:
    /// the mechanism ignores the condition axis.
    pub modulation: Option<f64>,
}

/// Everything the author of the next generation is allowed to know
/// about the previous one.
#[derive(Debug, Clone)]
pub struct AuthorDiagnostics {
    pub generation: u32,
    pub status: String,

    /// Real compiler/runtime diagnostics when the mechanism failed to
    /// execute — never hidden behind a generic message.
    pub error: Option<String>,

    pub complexity: Option<Complexity>,

    /// Mean validation RMSE (selection fitness), if it succeeded.
    pub fitness: Option<f64>,

    pub per_subject: Vec<SubjectDiagnostics>,

    /// The parent's own rationale note.
    pub note: Option<String>,
}

impl AuthorDiagnostics {
    /// Project a full report down to author-safe fields: the
    /// `evaluator_only` section and every hidden-set number are
    /// stripped here, and must stay out of the discovery loop.
    pub fn from_report(report: &GenReport) -> Self {
        Self {
            generation: report.generation,
            status: report.status.clone(),
            error: report.error.clone(),
            complexity: report.complexity,
            fitness: report.fitness,
            per_subject: report
                .per_subject
                .iter()
                .map(|subject| SubjectDiagnostics {
                    subject: subject.subject.clone(),
                    status: subject.status.clone(),
                    theta: subject.theta.clone(),
                    train_loss: subject.train_loss,
                    validation_rmse: subject.validation_rmse,
                    pinned: subject.pinned.clone(),
                    modulation: subject.modulation,
                })
                .collect(),
            note: report.note.clone(),
        }
    }
}

/// What the loop knows about one subject: its identity as a measured
/// dataset. Every subject is explained by the same shared mechanism;
/// the mechanism is shared, the numbers are not — each subject's
/// parameters are refitted separately by the harness.
#[derive(Debug, Clone)]
pub struct SubjectBrief {
    pub id: String,

    /// Human-readable name (pack-provided).
    pub title: String,

    /// Public metadata pairs (pack-provided: sequence, strength
    /// label, batch id, ...).
    pub detail: Vec<(String, String)>,

    /// The champion's frozen fitted parameters for this subject —
    /// the numbers that make the shared structure fit this subject.
    pub fitted: Vec<(String, f64)>,
}

/// Render the subject table for a prompt: the pack's intro paragraph,
/// then identity, public metadata, and the champion's per-subject
/// numbers.
fn format_subjects(subjects: &[SubjectBrief], intro: &str) -> String {
    if subjects.is_empty() {
        return String::new();
    }

    let mut text = String::new();

    if !intro.trim().is_empty() {
        text.push_str(intro.trim());
        text.push_str(":\n");
    }

    for subject in subjects {
        let fitted = subject
            .fitted
            .iter()
            .map(|(name, value)| format!("{name}={value:.2e}"))
            .collect::<Vec<_>>()
            .join(", ");

        let detail = subject
            .detail
            .iter()
            .map(|(key, value)| format!("{key} {value}"))
            .collect::<Vec<_>>()
            .join(" | ");

        text.push_str(&format!(
            "- {} | {}{}\n  champion fit: {}\n",
            subject.id,
            subject.title,
            if detail.is_empty() {
                String::new()
            } else {
                format!(" | {detail}")
            },
            if fitted.is_empty() { "(none yet)".to_string() } else { fitted },
        ));
    }

    text.push('\n');
    text
}

/// One turn for the theorist: propose mechanism ideas against the
/// champion's idea, informed by the parent's diagnostics and the
/// ledger.
#[derive(Debug, Clone)]
pub struct GenContext {
    pub generation: u32,

    /// `workspace/<pack>/gen/NNNN/mechanism.json` — the champion's
    /// idea, copied and ready to be replaced by the chosen proposal.
    pub mechanism_path: PathBuf,

    /// Author-safe outcome of the generation this one descends from.
    pub parent: AuthorDiagnostics,

    /// `curves.csv` inside the generation directory: the measured
    /// training curves the theorist may inspect (inlined into the
    /// prompt — the theorist has no tools). `None` in multi-subject
    /// runs, where the mechanism must stay subject-agnostic.
    pub observation_csv: Option<PathBuf>,

    /// Base64 PNG of the champion's diagnostic plot (simulated vs
    /// measured at the training input levels). Attached to the prompt
    /// as an image and written to the generation directory as
    /// `plot.png`. `None` runs the theorist text-only.
    pub plot_png_base64: Option<String>,

    /// Author-safe ledger digest (see [`crate::ledger::digest`]).
    pub ledger_digest: String,

    /// The subjects this run discovers over. Rendered into the
    /// prompt with the champion's per-subject numbers.
    pub subjects: Vec<SubjectBrief>,

    /// Pack paragraph introducing the subject table.
    pub subject_table_intro: String,

    /// The problem statement (pack-provided), for orientation.
    pub problem_definition: String,

    /// Full theorist preamble: static IR contract + pack domain text.
    pub preamble: String,

    /// The pack's vocabulary (validates proposals, names the input).
    pub lexicon: Lexicon,

    /// Harness-fixed policy override: when the loop has stalled, the
    /// harness forces `explore` regardless of the theorist's preference.
    pub forced_mode: Option<Mode>,

    /// Canonical hash of the champion mechanism — a chosen proposal
    /// hashing to this is a refit, not a new idea.
    pub champion_hash: String,
}

/// Everything the referee sees for one promotion decision: both
/// mechanisms as IR and both diagnostic plots (champion and
/// candidate) simulated against the same measured training data.
/// Deliberately no fitness numbers — the referee judges the curves.
pub struct JudgeContext<'a> {
    pub generation: u32,

    /// The incumbent's idea — the model currently holding champion.
    pub champion_mechanism: &'a Mechanism,

    /// The evaluated candidate — the model proposing to replace it.
    pub candidate_mechanism: &'a Mechanism,

    /// Base64 PNG: the champion's simulated vs measured trajectories.
    pub champion_plot_png_base64: String,

    /// Base64 PNG: the candidate's simulated vs measured trajectories.
    pub candidate_plot_png_base64: String,

    /// The subjects whose rows the plots show (empty for a single
    /// anonymous subject). The referee sees their identity and the
    /// champion's per-subject numbers — not any fitness value.
    pub subjects: Vec<SubjectBrief>,

    /// Pack paragraph introducing the subject table.
    pub subject_table_intro: String,

    /// The problem statement (pack-provided).
    pub problem_definition: String,

    /// Full referee preamble: static judging rules + pack domain text.
    pub preamble: String,
}

/// One mechanism idea: the IR plus the theory wrapped around it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    /// The mechanism as typed data — what gets lowered and fitted.
    pub mechanism: Mechanism,

    /// The physical claim: why this structure should fit better.
    pub hypothesis: String,

    /// Which curve feature should improve if the hypothesis holds.
    #[serde(default)]
    pub prediction: Option<String>,

    /// The concrete outcome that would kill the hypothesis.
    #[serde(default)]
    pub falsifier: Option<String>,

    /// The theorist's complexity estimate for the idea.
    #[serde(default)]
    pub cost: Option<String>,
}

impl Proposal {
    /// Canonical content hash — the ledger's dedup key.
    pub fn hash(&self) -> String {
        mechanism_ir::mechanism_hash(&self.mechanism)
    }
}

/// The theorist's answer for one generation: the (validated)
/// proposals, which one to implement, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Theory {
    /// All structurally valid proposals, in the order proposed.
    pub proposals: Vec<Proposal>,

    /// Index into `proposals` of the idea to implement this turn.
    pub chosen: usize,

    /// Effective mode: the theorist's choice, overridden by the
    /// harness when the loop is stalled.
    pub mode: Mode,

    /// The theorist's stated reason (or the fallback's explanation).
    pub reason: String,
}

impl Theory {
    pub fn chosen_proposal(&self) -> &Proposal {
        &self.proposals[self.chosen]
    }
}

/// The referee's answer for one evaluated generation: does the
/// candidate replace the champion, and why. Produced by constrained
/// decoding (native structured output); the JSON keys are camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChampionDecision {
    pub promote_to_champion: bool,
    pub reasoning: String,
}

/// The theorist hook: propose mechanisms, select one, persist both.
///
/// Writes `gen/NNNN/mechanism.json` (the chosen idea — the file the
/// harness lowers) and `gen/NNNN/ideas.json` (all proposals plus the
/// theory metadata — the lineage the ledger summarizes).
pub fn theorize(config: &LlmConfig, context: &GenContext) -> Result<Theory> {
    if context.mechanism_path.parent().is_none() {
        bail!(
            "mechanism path {} has no parent directory",
            context.mechanism_path.display()
        );
    }

    println!(
        "[evolve] gen {:04}: theorist `{}` proposing {} mechanisms",
        context.generation,
        config.model,
        PROPOSALS_PER_GENERATION,
    );

    let agent = agent::theorist::<RawTheory>(config, &context.preamble)
        .context("building the theorist agent")?;

    let prompt = theorist_prompt(context);

    let response = match &context.plot_png_base64 {
        Some(image) => agent::run_message(&agent, &prompt, Some(image))?,
        None => agent::run(&agent, &prompt)?,
    };

    let parsed = parse_theory(&response, &context.lexicon).or_else(|error| {
        eprintln!("[evolve] theorist output unparseable ({error}) — one repair round");

        let previous = extract_json(&response);

        let repair = theorist_repair_prompt(&error, previous);

        agent::run(&agent, &repair)
            .ok()
            .and_then(|response| parse_theory(&response, &context.lexicon).ok())
            .ok_or_else(|| {
                anyhow::anyhow!("theorist output still unparseable after repair: {error}")
            })
    })?;

    let mut theory = select(parsed, context);

    // Total-parse-failure fallback: refit the champion. The loop must
    // always have something to lower; a refit is the honest minimal move.
    if theory.proposals.is_empty() {
        println!("[evolve] falling back to champion refit");

        theory = fallback_theory(context)?;
    }

    persist(&theory, context)?;

    let chosen = theory.chosen_proposal();

    println!(
        "[evolve] gen {:04}: chose proposal {} [{}] hash {} — {}",
        context.generation,
        theory.chosen,
        theory.mode.label(),
        chosen.hash(),
        theory.reason,
    );

    Ok(theory)
}

/// The doctor: one tool-using repair turn for a mechanism.json the
/// validator rejected. The doctor edits *data*, not code — it has the
/// same file tools as the old author, sandboxed to the generation
/// directory, but its only job is to make the IR validate again
/// without abandoning the scientific intent.
pub fn doctor(
    config: &LlmConfig,
    mechanism_path: &Path,
    errors: &str,
    preamble: &str,
) -> Result<()> {
    let Some(work_dir) = mechanism_path.parent() else {
        bail!(
            "mechanism path {} has no parent directory",
            mechanism_path.display()
        );
    };

    println!("[evolve] doctor repairing {}", mechanism_path.display());

    let agent = agent::coder(config, work_dir, preamble)
        .with_context(|| format!("building the doctor agent for {}", work_dir.display()))?;

    let prompt = doctor_prompt(mechanism_path, errors);

    let response = agent::run(&agent, &prompt)?;

    println!("[evolve] doctor: {response}");

    Ok(())
}

/// The referee: one turn to decide whether an evaluated candidate
/// becomes the new champion. Like the theorist, the judge has no
/// tools; unlike it, it sees no numbers — its whole input is the two
/// mechanisms (Mechanism IR JSON) and the two simulated-vs-measured
/// trajectory plots, so selection pressure comes from the curves, not
/// a scalar fitness.
pub fn judge(config: &LlmConfig, context: &JudgeContext<'_>) -> Result<ChampionDecision> {
    println!(
        "[evolve] gen {:04}: judge `{}` weighing champion against candidate",
        context.generation,
        config.model,
    );

    let agent = agent::theorist::<ChampionDecision>(config, &context.preamble)
        .context("building the judge agent")?;

    let parts = judge_parts(context);

    let response = agent::run_parts(&agent, &parts)?;

    let parsed = parse_decision(&response).or_else(|error| {
        eprintln!("[evolve] judge output unparseable ({error}) — one repair round");

        let repair = judge_repair_prompt(&error);

        agent::run(&agent, &repair)
            .ok()
            .and_then(|response| parse_decision(&response).ok())
            .ok_or_else(|| {
                anyhow::anyhow!("judge output still unparseable after repair: {error}")
            })
    })?;

    println!(
        "[evolve] gen {:04}: judge promote={} — {}",
        context.generation, parsed.promote_to_champion, parsed.reasoning,
    );

    Ok(parsed)
}

/// Apply the harness-fixed policy to a parsed theory: forced explore
/// must pick a proposal that is not the champion.
fn select(mut parsed: ParsedTheory, context: &GenContext) -> Theory {
    let mode = context.forced_mode.unwrap_or(parsed.mode);

    let mut chosen = parsed.chosen.min(parsed.proposals.len().saturating_sub(1));

    if mode == Mode::Explore {
        if let Some(distinct) = parsed
            .proposals
            .iter()
            .position(|proposal| proposal.hash() != context.champion_hash)
        {
            if parsed.proposals[chosen].hash() == context.champion_hash {
                chosen = distinct;
            }
        } else {
            parsed
                .reason
                .push_str(" (forced explore, but every proposal hashed to the champion — refitting)");
        }
    }

    Theory {
        proposals: parsed.proposals,
        chosen,
        mode,
        reason: parsed.reason,
    }
}

/// The always-available fallback theory: refit the champion at full
/// budget. Requires the champion's mechanism to still validate.
fn fallback_theory(context: &GenContext) -> Result<Theory> {
    let text = std::fs::read_to_string(&context.mechanism_path)
        .with_context(|| format!("reading {}", context.mechanism_path.display()))?;

    let mechanism: Mechanism =
        serde_json::from_str(&text).context("parsing the champion mechanism")?;

    mechanism_ir::validate(&mechanism, &context.lexicon).map_err(|errors| {
        anyhow::anyhow!(
            "champion mechanism {} no longer validates:\n{errors}",
            context.mechanism_path.display()
        )
    })?;

    Ok(Theory {
        proposals: vec![Proposal {
            mechanism,
            hypothesis: "champion refit: re-fit the incumbent structure at full budget"
                .to_string(),
            prediction: None,
            falsifier: None,
            cost: None,
        }],
        chosen: 0,
        mode: Mode::Exploit,
        reason: "theorist output unparseable after repair; champion refit fallback".to_string(),
    })
}

/// Persist the chosen mechanism (the lowering input) and the full
/// theory (the lineage record).
fn persist(theory: &Theory, context: &GenContext) -> Result<()> {
    let Some(work_dir) = context.mechanism_path.parent() else {
        bail!(
            "mechanism path {} has no parent directory",
            context.mechanism_path.display()
        );
    };

    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("creating {}", work_dir.display()))?;

    let chosen = theory.chosen_proposal();

    std::fs::write(
        &context.mechanism_path,
        serde_json::to_string_pretty(&chosen.mechanism)?,
    )
    .with_context(|| format!("writing {}", context.mechanism_path.display()))?;

    let ideas_path = work_dir.join("ideas.json");

    std::fs::write(
        &ideas_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "generation": context.generation,
            "mode": theory.mode,
            "reason": theory.reason,
            "chosen": theory.chosen,
            "champion_hash": context.champion_hash,
            "proposals": theory.proposals,
        }))?,
    )
    .with_context(|| format!("writing {}", ideas_path.display()))?;

    Ok(())
}

#[derive(Debug)]
struct ParsedTheory {
    proposals: Vec<Proposal>,
    chosen: usize,
    mode: Mode,
    reason: String,
}

/// Parse the referee's response: extract the JSON object and decode
/// the typed decision.
fn parse_decision(response: &str) -> Result<ChampionDecision, String> {
    let json = extract_json(response).ok_or("no JSON object found in the response")?;

    serde_json::from_str(json).map_err(|error| error.to_string())
}

/// Parse the theorist's response: extract the JSON object, validate
/// every proposal's mechanism, keep only the valid ones, and remap
/// the chosen index. All-invalid responses yield an empty proposal
/// list (the caller falls back); partially-valid ones keep the valid
/// ideas and log the rejected ones.
fn parse_theory(response: &str, lexicon: &Lexicon) -> Result<ParsedTheory, String> {
    let json = extract_json(response).ok_or("no JSON object found in the response")?;

    let raw: RawTheory = serde_json::from_str(json).map_err(|error| error.to_string())?;

    let mode = match raw.mode.as_str() {
        "explore" => Mode::Explore,
        _ => Mode::Exploit,
    };

    let mut proposals = Vec::new();
    let mut chosen = raw.chosen;
    let mut rejections = String::new();

    for (index, proposal) in raw.proposals.into_iter().enumerate() {
        match validate_proposal(proposal, lexicon) {
            Ok(proposal) => {
                if index == raw.chosen {
                    chosen = proposals.len();
                }

                proposals.push(proposal);
            }
            Err(errors) => {
                eprintln!("[evolve] proposal {index} rejected by the validator:\n{errors}");

                rejections.push_str(&format!(
                    "proposal {index}: {}\n",
                    crate::ledger::truncate(&errors.replace('\n', " "), 600)
                ));

                if index == raw.chosen && raw.chosen > proposals.len() {
                    chosen = 0;
                }
            }
        }
    }

    if proposals.is_empty() {
        return Err(format!(
            "every proposed mechanism failed validation:\n{rejections}"
        ));
    }

    let chosen = chosen.min(proposals.len() - 1);

    Ok(ParsedTheory {
        proposals,
        chosen,
        mode,
        reason: raw.reason,
    })
}

fn validate_proposal(raw: RawProposal, lexicon: &Lexicon) -> Result<Proposal, String> {
    let mechanism: Mechanism =
        serde_json::from_value(raw.mechanism).map_err(|error| error.to_string())?;

    mechanism_ir::validate(&mechanism, lexicon)?;

    Ok(Proposal {
        mechanism,
        hypothesis: raw.hypothesis,
        prediction: raw.prediction,
        falsifier: raw.falsifier,
        cost: raw.cost,
    })
}

/// The theorist's wire format: the schema the endpoint's native
/// structured-output mode constrains the reply to. `mechanism` stays
/// free-form JSON — the IR validator ([`crate::mechanism_ir::validate`])
/// is the semantic gate, the schema only fixes the envelope.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RawTheory {
    proposals: Vec<RawProposal>,
    chosen: usize,
    mode: String,
    reason: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RawProposal {
    mechanism: serde_json::Value,
    hypothesis: String,

    #[serde(default)]
    prediction: Option<String>,

    #[serde(default)]
    falsifier: Option<String>,

    #[serde(default)]
    cost: Option<String>,
}

/// The JSON object inside a possibly chatty response: first `{` to
/// last `}`.
fn extract_json(response: &str) -> Option<&str> {
    let start = response.find('{')?;
    let end = response.rfind('}')?;

    if end < start {
        return None;
    }

    Some(&response[start..=end])
}

/// Write the author's measurement file: the training curves of every
/// subject, evenly downsampled, as one long-format CSV the author can
/// read, grep, and reason over. Only curves the author is allowed to
/// see may be passed in — the file lands inside the agent's sandbox.
pub fn write_observation_csv(
    dir: &Path,
    subjects: &[(String, Vec<Curve>)],
    max_points_per_curve: usize,
) -> Result<PathBuf> {
    let path = dir.join(OBSERVATION_CSV_NAME);

    let mut csv = format!("{OBSERVATION_CSV_HEADER}\n");

    for (subject, curves) in subjects {
        for (index, curve) in curves.iter().enumerate() {
            let t = &curve.raw.t;
            let y = &curve.raw.y;

            for &i in &sample_indices(t.len(), max_points_per_curve) {
                csv.push_str(&format!(
                    "{subject},{index},{},{:.3},{:.5}\n",
                    curve.condition.display(), t[i], y[i],
                ));
            }
        }
    }

    std::fs::write(&path, csv)
        .with_context(|| format!("writing {}", path.display()))?;

    Ok(path)
}

/// Evenly spaced sample indices covering first and last point.
fn sample_indices(n: usize, max_points: usize) -> Vec<usize> {
    if n <= max_points || max_points < 2 {
        return (0..n).collect();
    }

    let mut indices = Vec::with_capacity(max_points);

    for k in 0..max_points {
        let index =
            (k as f64 * (n - 1) as f64 / (max_points - 1) as f64).round() as usize;

        if indices.last() != Some(&index) {
            indices.push(index);
        }
    }

    indices
}

/// The theorist's static role and IR contract. Domain text (assay
/// physics, structure families, a worked example in the pack's own
/// vocabulary) is appended by the pack; the driver concatenates the
/// two. The only vocabulary here is the lexicon's.
pub fn theorist_preamble_static(lexicon: &Lexicon) -> String {
    let describe = |channels: &[crate::lexicon::Channel]| {
        channels
            .iter()
            .map(|channel| {
                format!(
                    "- {}: {} ({})",
                    channel.name, channel.description, channel.units
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let names = |channels: &[crate::lexicon::Channel]| {
        channels
            .iter()
            .map(|channel| format!("{:?}", channel.name))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let input_table = describe(&lexicon.inputs);
    let output_table = describe(&lexicon.outputs);
    let input_names = names(&lexicon.inputs);
    let output_names = names(&lexicon.outputs);

    format!(
        r#"You are an expert mechanistic theorist operating inside an automated mechanism-discovery loop.

You propose candidate mechanisms as structured JSON (the "Mechanism IR"). You have no tools and no filesystem: every fact you need is inlined in the task prompt. You never write Modelica, Rust, or any code — a deterministic compiler lowers your IR to a simulation, so your only job is to get the *structure* right.

THE EXPERIMENT CHANNELS
The recorded world speaks in channels. Input channels are drives: recorded piecewise-constant traces (each sample held until the next) that the harness replays into the model identically everywhere — they are data, never fitted.
{input_table}
Output channels are what the mechanism must predict:
{output_table}

THE DIAGNOSTIC IMAGE: when the task message carries a PNG, it shows the champion mechanism's SIMULATED signal (solid lines) against the MEASURED training data (dashed) at each training condition, on the same time axis — one row per subject (each row its own y-scale), with a residual strip (sim − measured) under every row. Read the residual shape off it: does the sim lag the data's rise, overshoot its plateau, decay too slowly, or bend where the data stays flat? A gap the fitted numbers cannot close is a STRUCTURE error. In a multi-subject image, a structure error is per-subject: watch which rows the champion fits and which it breaks.

THE MECHANISM IR
A mechanism is a reaction graph. Top level:
- "name": Modelica identifier of the model.
- "entities": physical participants. Each has "name", "kind" ("driven" | "dynamic"), optional "compartment", and optional "role" (a descriptive label from the problem's vocabulary). Exactly ONE entity is "driven" and carries "driven_by": one of {input_names} — it is the reservoir of that channel's quantity: its state is slaved to the channel's recorded drive trace (piecewise-constant between the recorded samples; any protocol step, ramp stair, or hold is already in that data). The model itself must stay time-invariant: the same equations run under every drive.
- "states": occupiable states, each {{"entity": <entity name>, "name": ..., "initial": <number>}}. Referenced as "Entity.state". The driven entity has exactly one state (its reservoir); every other entity's occupancy is what evolves.
- "parameters": every fitted knob, {{"name", "start", "min", "max"}}. Declaration order IS the fitting order. All rates, capacities, scales, and drifts reference parameters declared here.
- "algebraic": auxiliary variables {{"name", "expr"}}, computed from other variables and parameters.
- "processes": kinetics {{"name", "from": [...], "to": [...], "rate"}}. Participants are {{"state": <"Entity.state" or an algebraic name>, "stoich": <number, default 1>}}.
- "observable" (or "observables"): how occupancy becomes a measured channel — one observable per output channel, and every output channel must be predicted: {{"channel": <one of {output_names}; default "signal">, "contributions": [{{"target": <state ref or algebraic name>, "weight": <number, default 1>}}], "scale": <optional parameter name>, "drift": <optional artifact term>}}.

Rate laws: {{"kind": "mass_action", "parameter": "k"}} — flux = k * (each "from" participant raised to its stoich; total order must be <= 2) — or {{"kind": "custom", "expr": {{...}}}} for anything mass action cannot say (transport between compartments, allosteric factors).
Expressions are typed trees, tagged by "type": "var" ({{"name": "Entity.state" or algebraic name}}), "param", "input" ({{"name": <one of {input_names}>}} — the live value of that drive channel), "const", "sum" {{"terms": [...]}}, "product" {{"factors": [...]}}, "ratio" {{"numerator", "denominator"}}, "neg" {{"operand"}}, "pow" {{"base", "exponent"}}, "exp", "ln". No other forms exist.

Drift terms (baseline artifacts on the signal): {{"kind": "offset", "parameter": p}} | {{"kind": "linear", "rate": p}} (p * time) | {{"kind": "exponential", "amplitude": a, "tau": t}} (a * (1 - exp(-time / t))).

Hard rules the validator enforces (violations are rejected):
1. Exactly one driven entity, with "driven_by" naming one of {input_names}, and exactly one state. Its state is the reservoir: it never accumulates mass, it IS the channel's drive.
2. State initials are compile-time constants. The optimizer can NEVER move an initial. A parameter-dependent total (a capacity, a pool size) is expressed with the algebraic conservation idiom — never by making an initial depend on a parameter.
3. At least one dynamic (non-driven) state and at least one parameter; every state must be referenced by a process, an algebraic definition, or an observable.
4. Names are identifiers; parameters and algebraic names are global namespaces (no collisions with each other, with the channel names {input_names} / {output_names}, "time", or Modelica keywords).
5. Algebraic definitions must be acyclic (define in terms of states, parameters, inputs, and earlier algebraics — order in the list does not matter).
6. Mass-action total stoichiometric order <= 2.
7. Only non-driven states accumulate mass (get a der). Products gain flux * stoich, reactants lose it. Algebraic and driven participants are passive.

HOW TO THEORIZE
Read the parent diagnostics: which shape errors survive the fit? Where do residuals bend? A shape error that no parameter values fix is a STRUCTURE error: change the structure, not the numbers. The ledger tells you what was already tried, what failed, and which ideas were deferred — resurrect a promising deferred idea rather than reinventing one, and never re-propose what already failed for the stated reason.
Propose exactly 3 mechanisms per turn, structurally different from each other; range from conservative to ambitious. Parsimony is a virtue: every state and parameter must earn its keep, because the fit budget is fixed and complexity is logged.
- "hypothesis": the physical claim, one or two sentences.
- "prediction": which curve feature should improve if it holds.
- "falsifier": the concrete result that would kill it.
- "cost": rough complexity, e.g. "one extra state, two parameters".
- "mode": "explore" (a structurally new mechanism) or "exploit" (a refit or refinement of the champion).
- "chosen": index (0, 1, or 2) of the proposal to implement this turn.
- "reason": one sentence justifying the choice.

OUTPUT FORMAT — reply with exactly one JSON document and nothing else:
{{"proposals": [{{"mechanism": {{...}}, "hypothesis": "...", "prediction": "...", "falsifier": "...", "cost": "..."}}, {{"..."}}, {{"..."}}], "chosen": 0, "mode": "explore", "reason": "..."}}"#
    )
}

/// The doctor's static role: repair an invalid mechanism.json without
/// changing its scientific intent.
pub fn doctor_preamble_static(lexicon: &Lexicon) -> String {
    let input_names = lexicon
        .inputs
        .iter()
        .map(|channel| format!("{:?}", channel.name))
        .collect::<Vec<_>>()
        .join(", ");

    let output_names = lexicon
        .outputs
        .iter()
        .map(|channel| format!("{:?}", channel.name))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"You are a mechanism-IR repair technician inside an automated discovery loop.

Your sandbox contains one file of interest: `mechanism.json`, a Mechanism IR document describing a mechanism as typed JSON (entities, states, parameters, algebraic definitions, processes, observables). The validator rejected it; your job is to make it validate while preserving its scientific intent — change structure only as much as the errors require.

You have file tools. Read `mechanism.json` first. The IR contract:
- exactly one entity with "kind": "driven" and "driven_by" naming one of the input channels {input_names}, carrying exactly one state;
- every other entity: "kind": "dynamic" (an optional descriptive "role" is metadata only);
- states: {{"entity", "name", "initial"}} — initials are plain numbers (compile-time constants, never parameter-dependent; express parameter-dependent totals as an algebraic variable, e.g. {{"name": "P_free", "expr": {{"type": "sum", "terms": [{{"type": "param", "name": "rmax"}}, {{"type": "neg", "operand": {{"type": "var", "name": "PL.bound"}}}}]}}}});
- parameters: {{"name", "start", "min", "max"}} with min < max and start inside; at least one;
- algebraic: {{"name", "expr"}}, acyclic; expressions are typed trees tagged by "type": var | param | input ({{"name": one of {input_names}}}) | const | sum | product | ratio | neg | pow | exp | ln;
- processes: {{"name", "from": [{{"state": "Entity.state" or an algebraic name, "stoich": n}}], "to": [...], "rate": {{"kind": "mass_action", "parameter": k}} | {{"kind": "custom", "expr": ...}}}}; mass-action total order <= 2; every referenced name must exist;
- observable / observables: {{ "channel": one of {output_names} (default "signal"), "contributions": [{{"target": ..., "weight": w}}], "scale": optional parameter, "drift": optional {{"kind": "offset"|"linear"|"exponential", ...}} }}; at least one contribution; one observable per output channel at most;
- every state must be referenced somewhere; names are identifiers, unique per namespace, and avoid the channel names {input_names} / {output_names}, "time" and Modelica keywords.

Fix the errors listed in the task, keep everything else exactly as it is, and reply with a one-paragraph summary."#
    )
}

/// The referee's static role: judge whether an evaluated candidate
/// mechanism should replace the champion, from the trajectory plots
/// alone.
pub fn referee_preamble_static(problem_definition: &str) -> String {
    format!(
        r#"You are the referee of an automated mechanism-discovery loop.

The loop's problem definition: "{problem_definition}" A theorist proposes candidate mechanisms as structured JSON (the "Mechanism IR"); a deterministic compiler lowers each one to a simulation, an optimizer fits its parameters to the training input levels, and the simulated trajectories are plotted against the measured data.

Your one job: decide whether the CURRENT proposed model should replace the PREVIOUS model as champion. You are the loop's only judge. There is no numeric fitness in your prompt and you must not invent one: you decide like a scientist comparing two overlay plots.

WHAT YOU SEE
- The champion's mechanism (Mechanism IR JSON) and, below it, an image of its SIMULATED signal (solid lines) against the MEASURED training data (dashed) at each training input level.
- The current candidate's mechanism and the same kind of image for it.
- Both images share the same measured data and the same time axis; only the simulated curves differ.
- Each image has ONE ROW PER SUBJECT — a different measured instance of the same kind of thing (the subject table in the task names them). Rows have independent y-scales, and every row's parameters were refitted separately: rows differing in amplitude or speed is parameter business, not structure business. Under each row, a residual strip (sim − measured) shows the gap directly.

HOW TO JUDGE
Compare the residuals — the gap between each solid line and its dashed partner:
- does the simulated rise track the measured rise's timing and steepness?
- does the simulated level settle where the data settles, or overshoot / undershoot it?
- after any protocol step, does the decay follow the measured tail's shape and timescale?
- artifacts: baseline drift, or bends the data does not have.
A gap that fitted parameters cannot close is a STRUCTURE error — it weighs heavier than a small difference in residual size. A model that fits some subjects but breaks another (a row whose residual strip shows a bend no refit removes) has the wrong structure. Prefer parsimony: when two models explain the data equally well, the simpler mechanism (fewer states and parameters) wins.

DECISION RULE
Promote the candidate only if it explains every subject's measured trajectories at least as well as the champion, and better where the two visibly differ. When the evidence is ambiguous — the fits are hard to tell apart, or each wins on different rows — keep the champion: incumbency is the conservative default, and the loop can revisit a discarded idea later.

Reply with the structured decision only: "promoteToChampion" (boolean) and "reasoning" (a short paragraph grounded in what the two plots show — name the curve features that decided it)."#
    )
}

/// One theorist turn: diagnostics + ledger + measurement data + the
/// champion idea + the task.
fn theorist_prompt(context: &GenContext) -> String {
    let champion = std::fs::read_to_string(&context.mechanism_path)
        .unwrap_or_else(|_| "(champion mechanism unreadable)".to_string());

    let measurements = context
        .observation_csv
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|csv| {
            format!(
                "MEASUREMENT DATA (training curves you are allowed to see; CSV with columns \
                 {OBSERVATION_CSV_HEADER} — one response series per training condition and \
                 subject, evenly downsampled from full-resolution records; the `input` column \
                 is the conditioning label):\n\
                 {csv}\n\
                 The validation condition is NOT in this data: the mechanism must \
                 extrapolate to it from the conditions you can see, and the harness \
                 measures that prediction on the full-resolution curve.\n\n",
            )
        })
        .unwrap_or_default();

    let mode_instruction = match context.forced_mode {
        Some(Mode::Explore) => {
            "POLICY: the harness has forced EXPLORE this turn (the loop is \
             stalled). Your chosen proposal must be structurally new — \
             its mechanism hash must differ from the champion's.\n\n"
        }
        Some(Mode::Exploit) => {
            "POLICY: the harness has forced EXPLOIT this turn. Refine or \
             refit the champion structure.\n\n"
        }
        None => "",
    };

    let subjects = format_subjects(&context.subjects, &context.subject_table_intro);

    format!(
        r#"PROBLEM DEFINITION
{problem}

GENERATION {generation:04}: propose {proposals} mechanism ideas and choose one to implement.

{mode_instruction}{subjects}PARENT DIAGNOSTICS (author-safe; held-out scores are hidden from you):
{diagnostics}

RESEARCH LEDGER (what was tried, what failed, what was deferred):
{ledger}

{measurements}CURRENT CHAMPION MECHANISM (its idea, as Mechanism IR JSON; its hash is {hash}):
{champion}

Propose {proposals} structurally different mechanisms as specified in your instructions, and reply with the JSON document only."#,
        problem = context.problem_definition,
        generation = context.generation,
        proposals = PROPOSALS_PER_GENERATION,
        mode_instruction = mode_instruction,
        subjects = subjects,
        diagnostics = format_diagnostics(&context.parent),
        ledger = context.ledger_digest,
        measurements = measurements,
        hash = context.champion_hash,
        champion = champion,
    )
}

/// Repair round for an unparseable theorist response. When the failed
/// document is recoverable, the model is shown EXACTLY what it sent
/// and asked to fix only the reported errors — without the failed
/// text a stateless re-prompt silently becomes a fresh proposal round
/// with invented names, discarding the idea the validator was about
/// to accept.
fn theorist_repair_prompt(error: &str, previous: Option<&str>) -> String {
    match previous {
        Some(json) => format!(
            r#"Your previous reply failed: {error}

Here is the document you sent:

{json}

Fix ONLY the reported errors in this document, minimally. Keep every mechanism's name, hypothesis, entities, states, parameters, and structure exactly as written unless an error forces a change — do NOT propose new mechanisms and do NOT rename anything. Reply again with exactly one complete JSON document and nothing else — no prose, no code fences."#,
        ),
        None => format!(
            r#"Your previous reply could not be parsed: {error}

Reply again with exactly one JSON document and nothing else — no prose, no code fences:
{{"proposals": [{{"mechanism": {{...}}, "hypothesis": "...", "prediction": "...", "falsifier": "...", "cost": "..."}}, ...], "chosen": 0, "mode": "explore"|"exploit", "reason": "..."}}

Every "mechanism" must be a complete, valid Mechanism IR document as specified in your instructions."#,
        ),
    }
}

/// One doctor turn: the invalid mechanism + the validator's errors.
fn doctor_prompt(mechanism_path: &Path, errors: &str) -> String {
    let path = mechanism_path.display();

    format!(
        r#"The file `{path}` in your working directory failed IR validation:

{errors}

        Read it, fix it minimally so it validates (preserving the scientific intent), and reply with a one-paragraph summary of what you changed."#,
    )
}

/// The referee's prompt as interleaved text/image parts (see
/// [`crate::agent::run_parts`]): problem definition, champion model
/// with its trajectories below, candidate model with its trajectories
/// below, then the decision task.
fn judge_parts(context: &JudgeContext<'_>) -> Vec<(String, Option<String>)> {
    let champion = serde_json::to_string_pretty(context.champion_mechanism)
        .unwrap_or_else(|_| "(champion mechanism unreadable)".to_string());

    let candidate = serde_json::to_string_pretty(context.candidate_mechanism)
        .unwrap_or_else(|_| "(candidate mechanism unreadable)".to_string());

    let subjects = format_subjects(&context.subjects, &context.subject_table_intro);

    vec![
        (
            format!(
                "PROBLEM DEFINITION\n   \"{problem}\"\n\n\
                 {subjects}\
                 PREVIOUS PROPOSED MATHEMATICAL MODEL (champion)\n{champion}\n\n\
                 The simulated trajectories for the champion are available below — one row \
                 per subject, each row's parameters refitted separately.",
                problem = context.problem_definition,
                subjects = subjects,
                champion = champion,
            ),
            Some(context.champion_plot_png_base64.clone()),
        ),
        (
            format!(
                "CURRENT PROPOSED MATHEMATICAL MODEL\n{candidate}\n\n\
                 Your proposed model was executed against the observed experiments.\n\
                 The simulated trajectories for the new iteration are available below — one \
                 row per subject, each row's parameters refitted separately."
            ),
            Some(context.candidate_plot_png_base64.clone()),
        ),
        (
            "Decide if you want to promote the current model as a champion or discard it."
                .to_string(),
            None,
        ),
    ]
}

/// Repair round for an unparseable judge response.
fn judge_repair_prompt(error: &str) -> String {
    format!(
        r#"Your previous reply could not be parsed: {error}

Reply again with exactly one JSON document and nothing else — no prose, no code fences:
{{"promoteToChampion": true|false, "reasoning": "..."}}"#,
    )
}

/// The parent's author-safe outcome, rendered as compact prompt lines.
fn format_diagnostics(parent: &AuthorDiagnostics) -> String {
    let mut text = format!(
        "generation: {:04}\nstatus: {}",
        parent.generation, parent.status,
    );

    if let Some(error) = &parent.error {
        text.push_str(&format!("\nerror: {error}"));
    }

    if let Some(complexity) = parent.complexity {
        text.push_str(&format!(
            "\ncomplexity: {} states, {} parameters, {} equations",
            complexity.states, complexity.parameters, complexity.equations,
        ));
    }

    if let Some(fitness) = parent.fitness {
        text.push_str(&format!(
            "\nfitness (mean validation RMSE, lower is better): {fitness:.6}"
        ));
    }

    for subject in &parent.per_subject {
        text.push_str(&format!(
            "\nsubject {}: status {} train_loss {:.4} validation_rmse {:.6}",
            subject.subject, subject.status, subject.train_loss, subject.validation_rmse,
        ));

        let theta: Vec<String> = subject
            .theta
            .iter()
            .map(|(name, value)| format!("{name}={value:.3e}"))
            .collect();

        text.push_str(&format!("\n  fitted theta: {}", theta.join(", ")));

        // Sentinels: numeric self-checks with an explicit ask — the
        // bounds and the form are yours to change; the data is not.
        for pinned in &subject.pinned {
            text.push_str(&format!(
                "\n  SENTINEL {pinned} — the fit wants to go past this bound; \
                 widen it or change the form so the parameter is not pinned."
            ));
        }

        if let Some(modulation) = subject.modulation {
            if modulation < 0.5 {
                text.push_str(&format!(
                    "\n  SENTINEL the mechanism's response varies only {:.0}% of how much \
                     the measured data varies across the validation conditions — it largely \
                     ignores the condition axis. A parameter stuck at a bound is usually \
                     why: a sensitivity constant pinned far outside the condition range \
                     makes every condition behave the same.",
                    modulation * 100.0,
                ));
            }
        }
    }

    if let Some(note) = &parent.note {
        text.push_str(&format!("\nparent note: {note}"));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

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

    fn curve(input: f64, n: usize) -> Curve {
        Curve {
            condition: crate::experiment::Condition::Level(input),
            raw: crate::data::RawCurve {
                t: (0..n).map(|i| i as f64 * 0.2).collect(),
                y: (0..n).map(|i| (i as f64 * 0.01).sin()).collect(),
            },
            session: 0,
            t_step: None,
            input_after: 0.0,
            drives: vec![],
            output_channel: None,
        }
    }

    fn baseline_mechanism() -> Mechanism {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("packs/binding/mechanism.json");

        serde_json::from_str(
            &std::fs::read_to_string(&path).expect("baseline mechanism readable"),
        )
        .expect("baseline mechanism parses")
    }

    fn proposal(mechanism: Mechanism, hypothesis: &str) -> Proposal {
        Proposal {
            mechanism,
            hypothesis: hypothesis.to_string(),
            prediction: None,
            falsifier: None,
            cost: None,
        }
    }

    #[test]
    fn observation_csv_downsamples_and_labels() {
        let dir = std::env::temp_dir().join("skill-evolve-test");

        let _ = std::fs::remove_dir_all(&dir);

        std::fs::create_dir_all(&dir).unwrap();

        let subjects =
            vec![("steady-zebra-lava".to_string(), vec![curve(0.0, 50), curve(31.6, 7)])];

        let path = write_observation_csv(&dir, &subjects, 10).expect("csv written");

        let text = std::fs::read_to_string(&path).unwrap();

        let rows: Vec<&str> = text.lines().collect();

        assert_eq!(rows[0], OBSERVATION_CSV_HEADER);

        // 10 sampled points + 7 kept points.
        assert_eq!(rows.len(), 1 + 10 + 7);

        assert!(rows[1].starts_with("steady-zebra-lava,0,0,0.000,"));

        // First and last time of the downsampled curve survive.
        assert!(rows[10].starts_with("steady-zebra-lava,0,0,"));
        assert!(rows[10].contains(",9.800,"));

        assert!(rows[11].starts_with("steady-zebra-lava,1,31.6,"));
    }

    #[test]
    fn sample_indices_cover_the_ends() {
        assert_eq!(sample_indices(10, 4), vec![0, 3, 6, 9]);
        assert_eq!(sample_indices(4, 10), vec![0, 1, 2, 3]);
        assert_eq!(sample_indices(100, 2), vec![0, 99]);
    }

    #[test]
    fn json_is_extracted_from_prose() {
        assert_eq!(
            extract_json("Here you go:\n{\"a\": 1}\nDone."),
            Some("{\"a\": 1}"),
        );

        assert_eq!(extract_json("no json here"), None);

        // A code fence with braces inside prose still extracts.
        assert_eq!(
            extract_json("```json\n{\"b\": {\"c\": 2}}\n```"),
            Some("{\"b\": {\"c\": 2}}"),
        );
    }

    #[test]
    fn theory_parsing_keeps_valid_proposals_and_remaps_chosen() {
        let baseline = baseline_mechanism();

        let mut broken = baseline_mechanism();

        broken.parameters.clear();

        let response = serde_json::to_string(&json!({
            "proposals": [
                {"mechanism": broken, "hypothesis": "broken"},
                {"mechanism": baseline, "hypothesis": "good"},
            ],
            "chosen": 1,
            "mode": "explore",
            "reason": "the good one",
        }))
        .unwrap();

        let parsed = parse_theory(&response, &lexicon()).expect("parses");

        // The invalid proposal is filtered; chosen remaps to its
        // position among the survivors.
        assert_eq!(parsed.proposals.len(), 1);
        assert_eq!(parsed.chosen, 0);
        assert_eq!(parsed.mode, Mode::Explore);
        assert_eq!(parsed.reason, "the good one");

        assert_eq!(
            parsed.proposals[0].hash(),
            mechanism_ir::mechanism_hash(&baseline_mechanism()),
        );
    }

    #[test]
    fn theory_parsing_rejects_garbage() {
        assert!(parse_theory("not json at all", &lexicon()).is_err());

        let response = serde_json::to_string(&json!({
            "proposals": [],
            "chosen": 0,
            "mode": "explore",
            "reason": "empty",
        }))
        .unwrap();

        assert!(parse_theory(&response, &lexicon()).is_err());
    }

    #[test]
    fn champion_decision_round_trips_camel_case() {
        let decision = ChampionDecision {
            promote_to_champion: true,
            reasoning: "the dissociation tail now matches the data".to_string(),
        };

        let json = serde_json::to_string(&decision).unwrap();

        assert!(json.contains("promoteToChampion"));

        let parsed: ChampionDecision = serde_json::from_str(&json).unwrap();

        assert!(parsed.promote_to_champion);
        assert_eq!(parsed.reasoning, decision.reasoning);
    }

    #[test]
    fn decision_parsing_extracts_json_from_prose() {
        let parsed = parse_decision(
            "Sure:\n{\"promoteToChampion\": false, \
             \"reasoning\": \"plateau overshoot persists\"}\nDone.",
        )
        .expect("parses");

        assert!(!parsed.promote_to_champion);
        assert!(parsed.reasoning.contains("plateau"));

        assert!(parse_decision("no json").is_err());
    }

    #[test]
    fn judge_parts_interleave_models_and_images() {
        let mechanism = baseline_mechanism();

        let context = JudgeContext {
            generation: 4,
            champion_mechanism: &mechanism,
            candidate_mechanism: &mechanism,
            champion_plot_png_base64: "champion-png".to_string(),
            candidate_plot_png_base64: "candidate-png".to_string(),
            subjects: vec![SubjectBrief {
                id: "steady-zebra-lava".to_string(),
                title: "AdaptyV-01".to_string(),
                detail: vec![
                    ("sequence".to_string(), "PSTV".to_string()),
                    ("binding strength".to_string(), "Strong".to_string()),
                ],
                fitted: vec![("kon".to_string(), 2.2e4)],
            }],
            subject_table_intro: "SUBJECTS (one row per designed peptide)".to_string(),
            problem_definition: "Infer a mathematical model of binding kinetics.".to_string(),
            preamble: String::new(),
        };

        let parts = judge_parts(&context);

        assert_eq!(parts.len(), 3);

        assert!(parts[0].0.contains("PROBLEM DEFINITION"));
        assert!(parts[0].0.contains("binding kinetics"));
        assert!(parts[0].0.contains("\"entities\""));
        assert!(parts[0].0.contains("steady-zebra-lava"));
        assert!(parts[0].0.contains("kon=2.20e4"));
        assert_eq!(parts[0].1.as_deref(), Some("champion-png"));

        assert!(parts[1].0.contains("CURRENT PROPOSED MATHEMATICAL MODEL"));
        assert_eq!(parts[1].1.as_deref(), Some("candidate-png"));

        assert!(parts[2].0.contains("promote"));
        assert!(parts[2].1.is_none());
    }

    #[test]
    fn format_subjects_renders_identity_metadata_and_fit() {
        let subjects = vec![SubjectBrief {
            id: "steady-zebra-lava".to_string(),
            title: "AdaptyV-01".to_string(),
            detail: vec![("binding strength".to_string(), "Strong".to_string())],
            fitted: vec![
                ("kon".to_string(), 2.2e4),
                ("koff".to_string(), 2.0e-2),
            ],
        }];

        let text = format_subjects(&subjects, "SUBJECTS (each row is a peptide):");

        assert!(text.contains("steady-zebra-lava | AdaptyV-01 | binding strength Strong"));
        assert!(text.contains("kon=2.20e4, koff=2.00e-2"));

        let empty = SubjectBrief {
            id: "x".to_string(),
            title: "X".to_string(),
            detail: vec![],
            fitted: vec![],
        };

        assert!(format_subjects(&[empty], "").contains("(none yet)"));

        assert_eq!(format_subjects(&[], "intro"), "");
    }

    #[test]
    fn forced_explore_picks_a_structurally_new_proposal() {
        let champion = baseline_mechanism();

        let champion_hash = mechanism_ir::mechanism_hash(&champion);

        let mut different = baseline_mechanism();

        different.parameters.push(mechanism_ir::ParameterDecl {
            name: "k_extra".to_string(),
            start: 1.0,
            min: 0.0,
            max: 10.0,
        });

        let context = GenContext {
            generation: 2,
            mechanism_path: PathBuf::from("gen/0002/mechanism.json"),
            parent: AuthorDiagnostics {
                generation: 1,
                status: "Success".to_string(),
                error: None,
                complexity: None,
                fitness: None,
                per_subject: vec![],
                note: None,
            },
            observation_csv: None,
            plot_png_base64: None,
            ledger_digest: String::new(),
            subjects: vec![],
            subject_table_intro: String::new(),
            problem_definition: String::new(),
            preamble: String::new(),
            lexicon: lexicon(),
            forced_mode: Some(Mode::Explore),
            champion_hash: champion_hash.clone(),
        };

        let parsed = ParsedTheory {
            proposals: vec![
                proposal(different.clone(), "new structure"),
                proposal(champion.clone(), "refit"),
            ],
            chosen: 1,
            mode: Mode::Exploit,
            reason: "refit".to_string(),
        };

        let theory = select(parsed, &context);

        assert_eq!(theory.mode, Mode::Explore);
        assert_ne!(theory.chosen, 1);
        assert_eq!(theory.chosen_proposal().hash(), mechanism_ir::mechanism_hash(&different));
    }

    #[test]
    fn preambles_name_the_input_and_the_problem() {
        let theorist = theorist_preamble_static(&lexicon());

        assert!(theorist.contains("\"driven_by\": one of \"concentration\""));
        assert!(theorist.contains("OUTPUT FORMAT"));

        let doctor = doctor_preamble_static(&lexicon());

        assert!(doctor.contains("repair technician"));
        assert!(doctor.contains("naming one of the input channels \"concentration\""));

        let referee = referee_preamble_static("Infer a mathematical model of binding kinetics.");

        assert!(referee.contains("Infer a mathematical model of binding kinetics."));
        assert!(referee.contains("promoteToChampion"));
    }
}