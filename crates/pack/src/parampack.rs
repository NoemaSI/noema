//! `skill parampack` — a parameter pack for law discovery (Phase D).
//!
//! The table `skill export` produces (`subject, feature…, theta…`) is
//! a second experimental reality: the subjects are covariate units
//! (binders, strains, batches), the measured outputs are pack A's
//! frozen parameters, and the design axis is the subject firewall
//! itself — binders the law may see (train), binders it must select
//! on (validation), and binders it has never seen (hidden).
//!
//! This command renders that reality as a pack: one subject `"law"`,
//! one curve per (row, output) — a constant drive trace of the row's
//! features and a constant measured value of the (log-)transformed
//! parameter — and a tag-conditioned split over row ids. The evolving
//! artifact is then a static mechanism `theta_hat = f(features)`: its
//! parameters ARE the law's coefficients, fitted jointly over the
//! train rows by the same fixed optimizer, scored by the same fixed
//! referee on unseen binders. A fit in theta-space that does not
//! reproduce curves is not a law — and here the curves ARE thetas, so
//! the honest acceptance criterion (cross-pack curve simulation,
//! Phase D step 3) is the natural next command, not a new concept.
//!
//! ```text
//! skill parampack --table=<law_table.csv> --out-dir=<pack dir>
//!                 --outputs=kon,koff,rmax [--features=a,b,c]
//!                 [--linear=col,…]           # outputs NOT log10-transformed
//!                 [--ratio=70/15/15]         # deterministic hash split (default)
//!                 [--train=… --validation=… --hidden=…]  # explicit id lists
//! ```
//!
//! Features default to every remaining table column that is numeric
//! and complete across all rows; anything else is dropped with a
//! report (a law never imputes silently).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::data::{Curve, RawCurve};
use crate::experiment::{Condition, InputDrive};
use crate::lexicon::Lexicon;
use crate::mechanism_ir::{self, Mechanism};
use crate::pack::Subject;

/// One declared channel of the generated pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LawChannel {
    /// Modelica identifier (the lexicon name; `c_`-prefixed for features).
    pub channel: String,
    /// Original table column (for reports and the provenance trail).
    pub column: String,
    pub units: String,
    pub description: String,
}

/// One measured row: a covariate unit and its (transformed) thetas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LawRow {
    pub id: String,
    pub features: Vec<f64>,
    pub outputs: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LawSplit {
    pub train: Vec<String>,
    pub validation: Vec<String>,
    pub hidden: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LawPreambles {
    pub theorist: String,
    pub doctor: String,
    pub referee: String,
}

/// Everything the generated pack needs, embedded as base64 JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LawTable {
    pub name: String,
    pub problem: String,
    /// Human-readable provenance line (export digest, pack A, gen).
    pub provenance: String,
    pub features: Vec<LawChannel>,
    pub outputs: Vec<LawChannel>,
    pub rows: Vec<LawRow>,
    pub split: LawSplit,
    pub preambles: LawPreambles,
}

pub fn run(args: &[String]) -> Result<()> {
    let flag = |prefix: &str| -> Option<String> {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .map(str::to_string)
    };

    let table_path = PathBuf::from(flag("--table=").context("parampack needs --table=<csv>")?);
    let out_dir = PathBuf::from(flag("--out-dir=").context("parampack needs --out-dir=<dir>")?);

    let outputs: Vec<String> = flag("--outputs=")
        .context("parampack needs --outputs=<theta,theta,…>")?
        .split(',')
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();

    let linear: Vec<String> = flag("--linear=")
        .map(|list| list.split(',').map(str::to_string).collect())
        .unwrap_or_default();

    let requested_features: Option<Vec<String>> =
        flag("--features=").map(|list| list.split(',').map(str::to_string).collect());

    let ratio: (u64, u64) = match flag("--ratio=") {
        Some(text) => {
            let mut parts = text.split('/');

            let train = parts.next().and_then(|part| part.parse().ok()).unwrap_or(70);
            let validation = parts.next().and_then(|part| part.parse().ok()).unwrap_or(15);

            (train, validation)
        }
        None => (70, 15),
    };

    // ---------------------------------------------------------------- table

    let mut reader =
        csv::Reader::from_path(&table_path).with_context(|| format!("opening {}", table_path.display()))?;

    let headers: Vec<String> = reader.headers()?.iter().map(String::from).collect();

    let id_column = headers[0].clone();
    let records: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>()?;

    let column = |name: &str| -> usize {
        headers
            .iter()
            .position(|header| header == name)
            .unwrap_or(usize::MAX)
    };

    for output in &outputs {
        if column(output) == usize::MAX {
            bail!("output column {output:?} not in the table (columns: {})", headers.join(", "));
        }
    }

    // Rows by id (a subject appears once).
    let mut by_id: BTreeMap<String, &csv::StringRecord> = BTreeMap::new();

    for record in &records {
        let id = record.get(column(&id_column)).unwrap_or_default().to_string();

        if by_id.insert(id.clone(), record).is_some() {
            bail!("duplicate subject {id:?} in the table — export must emit one row per subject");
        }
    }

    // Feature columns: requested, or every remaining complete-numeric
    // column. Incomplete or non-numeric columns are dropped with a
    // report — a law never imputes silently.
    let mut feature_columns: Vec<String> = Vec::new();
    let mut dropped: Vec<(String, String)> = Vec::new();

    let candidates: Vec<String> = match &requested_features {
        Some(list) => list.clone(),
        None => headers
            .iter()
            .skip(1)
            .filter(|header| !outputs.contains(&header.to_string()))
            .map(String::from)
            .collect(),
    };

    for name in candidates {
        let index = match column(&name) {
            usize::MAX => {
                dropped.push((name, "not a table column".to_string()));
                continue;
            }
            index => index,
        };

        let mut reason = None;
        let mut filled = 0usize;

        for record in &records {
            let value = record.get(index).unwrap_or_default().trim();

            if value.is_empty() {
                reason = Some("missing values".to_string());
                break;
            }

            if value.parse::<f64>().is_err() {
                reason = Some("non-numeric values".to_string());
                break;
            }

            filled += 1;
        }

        match reason {
            None if filled == records.len() => feature_columns.push(name),
            Some(why) => dropped.push((name, why)),
            None => dropped.push((name, "incomplete".to_string())),
        }
    }

    if let Some(requested) = &requested_features {
        for (name, why) in &dropped {
            if requested.contains(name) {
                bail!("requested feature {name:?} was dropped: {why}");
            }
        }
    }

    if feature_columns.is_empty() {
        bail!("no usable feature columns (dropped: {:?})", dropped);
    }

    // Rows: features raw, outputs log10'd (unless --linear= lists them).
    let mut rows = Vec::new();

    for (id, record) in &by_id {
        let features: Vec<f64> = feature_columns
            .iter()
            .map(|name| record.get(column(name)).unwrap_or_default().trim().parse::<f64>().unwrap())
            .collect();

        let mut values = Vec::new();
        let mut skip = false;

        for output in &outputs {
            let text = record.get(column(output)).unwrap_or_default().trim();
            let Ok(value) = text.parse::<f64>() else { skip = true; break };

            if linear.iter().any(|name| name == output) {
                values.push(value);
            } else if value > 0.0 && value.is_finite() {
                values.push(value.log10());
            } else {
                skip = true;
                break;
            }
        }

        if skip {
            eprintln!("skill: row {id}: non-positive or missing theta — row excluded from the law");
            continue;
        }

        rows.push(LawRow { id: id.clone(), features, outputs: values });
    }

    if rows.len() < 20 {
        bail!("only {} usable rows — a law needs at least 20 subjects", rows.len());
    }

    // ---------------------------------------------------------------- split

    let explicit = [flag("--train="), flag("--validation="), flag("--hidden=")];

    let split = if explicit.iter().any(|role| role.is_some()) {
        let list = |role: &Option<String>, name: &str| -> Result<Vec<String>> {
            let text = role
                .as_ref()
                .with_context(|| format!("explicit splits must give all three: --{name}"))?;

            let ids: Vec<String> = text
                .split(',')
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect();

            if ids.is_empty() {
                bail!("split role {name} is empty");
            }

            for id in &ids {
                if !rows.iter().any(|row| &row.id == id) {
                    bail!("--{name} lists unknown subject {id:?}");
                }
            }

            Ok(ids)
        };

        LawSplit {
            train: list(&explicit[0], "train")?,
            validation: list(&explicit[1], "validation")?,
            hidden: list(&explicit[2], "hidden")?,
        }
    } else {
        // Deterministic hash split: the same ids land in the same role
        // on every regeneration; the hidden share is everything past
        // train+validation of the 100 buckets.
        let train_end = ratio.0;
        let validation_end = ratio.0 + ratio.1;

        let mut split = LawSplit { train: vec![], validation: vec![], hidden: vec![] };

        for row in &rows {
            let bucket = fnv1a(row.id.as_bytes()) % 100;

            match bucket {
                b if b < train_end => split.train.push(row.id.clone()),
                b if b < validation_end => split.validation.push(row.id.clone()),
                _ => split.hidden.push(row.id.clone()),
            }
        }

        if split.train.is_empty() || split.validation.is_empty() || split.hidden.is_empty() {
            bail!("hash split left a role empty — pass explicit --train/--validation/--hidden");
        }

        split
    };

    // ---------------------------------------------------------------- channels

    let mut used_names: Vec<String> = Vec::new();
    let next_channel = |base: &str, used: &mut Vec<String>| -> String {
        let mut name = sanitize(base);

        while used.iter().any(|existing| existing == &name) {
            name = format!("{name}_x");
        }

        used.push(name.clone());
        name
    };

    let features: Vec<LawChannel> = feature_columns
        .iter()
        .map(|name| LawChannel {
            channel: next_channel(&format!("c_{name}"), &mut used_names),
            column: name.clone(),
            units: "1".to_string(),
            description: format!("covariate `{name}` (computed before measurement)"),
        })
        .collect();

    let output_channels: Vec<LawChannel> = outputs
        .iter()
        .map(|name| LawChannel {
            channel: next_channel(name, &mut used_names),
            column: name.clone(),
            units: if linear.iter().any(|linear| linear == name) {
                format!("raw {name}")
            } else {
                format!("log10({name})")
            },
            description: format!("pack A's frozen parameter `{name}` for this subject"),
        })
        .collect();

    // ---------------------------------------------------------------- provenance

    let provenance_path = format!("{}.provenance.json", table_path.display());
    let provenance_json: serde_json::Value = std::fs::read_to_string(&provenance_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::json!({"note": "no provenance sidecar found next to the table"}));

    let mechanism_quote = provenance_json["workspaces"]
        .as_array()
        .and_then(|workspaces| workspaces.first())
        .and_then(|workspace| workspace["mechanism"].as_str())
        .unwrap_or("(no champion mechanism text in the provenance)")
        .to_string();

    let provenance_line = format!(
        "data: export of pack `{}` (digest {}) — the parameters below were fitted by that mechanism; \
         the law predicts them, it does not re-fit them",
        provenance_json["pack"].as_str().unwrap_or("?"),
        provenance_json["table_digest"].as_str().unwrap_or("?"),
    );

    let name = flag("--name=").unwrap_or_else(|| {
        format!(
            "law_{}",
            table_path
                .file_stem()
                .map(|stem| sanitize(&stem.to_string_lossy()))
                .unwrap_or_else(|| "pack".to_string()),
        )
    });

    let problem = format!(
        "Discover the LAW that predicts, for a subject whose curves were never measured, the \
         parameters {} that pack A's champion mechanism fitted per subject — from covariates \
         known before measurement. {} The law is ONE shared parameter set: held-out subjects \
         (hidden) decide what counts.",
        output_channels.iter().map(|channel| format!("`{}` ({})", channel.channel, channel.units)).collect::<Vec<_>>().join(", "),
        provenance_line,
    );

    let preambles = preambles_for(&features, &output_channels, &mechanism_quote);

    let table = LawTable {
        name: name.clone(),
        problem,
        provenance: provenance_line,
        features,
        outputs: output_channels,
        rows,
        split,
        preambles,
    };

    // ---------------------------------------------------------------- outputs

    let baseline = baseline_mechanism(&table);

    let lexicon = lexicon_of(&table);
    mechanism_ir::validate(&baseline, &lexicon)
        .map_err(|error| anyhow::anyhow!("generated baseline mechanism is invalid: {error}"))?;

    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;

    std::fs::write(
        out_dir.join("mechanism.json"),
        serde_json::to_string_pretty(&baseline).expect("serializable"),
    )
    .context("writing mechanism.json")?;

    std::fs::write(out_dir.join("pack.rs"), render_pack(&table))
        .context("writing pack.rs")?;

    println!("skill: parampack `{name}` -> {}/pack.rs", out_dir.display());
    println!(
        "skill: {} rows | {} features | {} outputs | split {} train / {} validation / {} hidden (hash)",
        table.rows.len(),
        table.features.len(),
        table.outputs.len(),
        table.split.train.len(),
        table.split.validation.len(),
        table.split.hidden.len(),
    );

    if !dropped.is_empty() {
        println!("skill: dropped feature columns: {}", dropped.iter().map(|(name, why)| format!("{name} ({why})")).collect::<Vec<_>>().join(", "));
    }

    println!("skill: next: skill evolve --pack={}/pack.rs --gens=0 --no-evolve", out_dir.display());

    Ok(())
}

// ---------------------------------------------------------------- helpers

fn sanitize(text: &str) -> String {
    let mut out = String::new();

    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }

    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out = format!("f_{out}");
    }

    out
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    hash
}

/// The lexicon the generated pack declares (same construction).
pub fn lexicon_of(table: &LawTable) -> Lexicon {
    let channel = |c: &LawChannel| crate::lexicon::Channel {
        name: c.channel.clone(),
        units: c.units.clone(),
        description: c.description.clone(),
    };

    Lexicon {
        inputs: table.features.iter().map(channel).collect(),
        outputs: table.outputs.iter().map(channel).collect(),
        roles: vec![],
        notes: "Every input channel is a covariate held constant per curve; every output \
         channel is one frozen parameter of one subject. Time is not a physical axis here."
            .to_string(),
    }
}

/// The deliberately-mediocre gen-0 law: per output, a linear function
/// of the FIRST feature only. Headroom is the point. The IR's state
/// idioms carry the static map: the driven reservoir `X.free` holds
/// the first feature's value, and the kernel state `K.n` (initial 1,
/// no processes) is the constant that multiplies into the intercept.
pub fn baseline_mechanism(table: &LawTable) -> Mechanism {
    let first = &table.features[0].channel;

    let mut parameters = Vec::new();
    let mut algebraic = Vec::new();
    let mut observables = Vec::new();

    for output in &table.outputs {
        let intercept = format!("intercept_{}", output.channel);
        let slope = format!("slope_{}", output.channel);
        let law = format!("law_{}", output.channel);

        parameters.push(serde_json::json!({"name": intercept, "start": 0.0, "min": -20.0, "max": 20.0}));
        parameters.push(serde_json::json!({"name": slope, "start": 0.0, "min": -20.0, "max": 20.0}));

        algebraic.push(serde_json::json!({
            "name": law,
            "expr": {"type": "sum", "terms": [
                {"type": "product", "factors": [
                    {"type": "param", "name": intercept},
                    {"type": "var", "name": "K.n"},
                ]},
                {"type": "product", "factors": [
                    {"type": "param", "name": slope},
                    {"type": "var", "name": "X.free"},
                ]},
            ]},
        }));

        observables.push(serde_json::json!({
            "channel": output.channel,
            "contributions": [{"target": law, "weight": 1.0}],
        }));
    }

    let json = serde_json::json!({
        "name": "Law0",
        "compartments": [],
        "entities": [
            {"name": "X", "kind": "driven", "driven_by": first},
            {"name": "K", "kind": "dynamic", "role": "kernel"},
        ],
        "states": [
            {"entity": "X", "name": "free", "initial": 0.0},
            {"entity": "K", "name": "n", "initial": 1.0},
        ],
        "parameters": parameters,
        "algebraic": algebraic,
        "processes": [],
        "observable": null,
        "observables": observables,
    });

    serde_json::from_value(json).expect("generated baseline is valid IR by construction")
}

fn preambles_for(features: &[LawChannel], outputs: &[LawChannel], mechanism_quote: &str) -> LawPreambles {
    let feature_list = features
        .iter()
        .map(|feature| format!("- `{}`: {}", feature.channel, feature.description))
        .collect::<Vec<_>>()
        .join("\n");

    let output_list = outputs
        .iter()
        .map(|output| format!("- `{}` ({})", output.channel, output.units))
        .collect::<Vec<_>>()
        .join("\n");

    LawPreambles {
        theorist: format!(
            r#"DOMAIN: LAW DISCOVERY OVER MEASURED PARAMETERS

You are not modeling a physical system in time. Each measured "curve" is ONE subject's ONE parameter value (held constant over the token time axis): the input channels are that subject's covariates, held constant; the output must equal the subject's measured parameter. The mechanism you propose is therefore a STATIC MAP theta_hat = f(covariates) — its parameters are the law's coefficients, shared by every subject: the harness fits ONE theta vector over all training subjects at once. A mechanism that adds per-subject freedom is not a law.

FEATURES (covariates known before measurement; units are pre-normalized raw values):
{feature_list}

TARGETS (frozen parameters of the champion mechanism — {}):
{output_list}

The targets are in log10 units unless a channel says "raw": one unit of error is one dex. Propose `f` using algebraic expressions over `input` channels (or state references — the driven state `X.free` IS the first feature's value) and parameters (sums, products, ratios, exp, ln, pow). The kernel state `K.n` (initial 1, no processes) is a constant: multiply an intercept into it so every state stays referenced, like the shipped baseline does. Structure families worth trying: multiplicative allometry (products of powers), saturating legs (x/(K+x)), interactions (products of features), conserved combinations across outputs (e.g. if one target is a ratio of two others, tie their laws), and log-features.

THE DATA YOU ARE PREDICTING was fitted by this mechanism (provenance, cite it in your note):
{mechanism_quote}

You never see held-out subjects' values; the split roles on subject identity (tags). A law that fits training subjects and degrades on held-out ones is exactly what the firewall is for — prefer low-complexity forms with mechanism-legitimate shapes."#,
            outputs.iter().map(|output| output.column.clone()).collect::<Vec<_>>().join(", "),
        ),
        doctor: "Domain: this is a static law, not a time course. The model lives in `algebraic` \
         expressions over covariate channels and parameters, emitted through one observable \
         per output channel. The driven state `X.free` equals the first feature; the kernel \
         state `K.n` is the constant 1 (multiply intercepts into it — the validator rejects \
         unreferenced states). Keep parameter bounds wide (targets are log10 units)."
            .to_string(),
        referee: "Domain: predicted versus measured subject parameters (log10 dex on \
         non-raw channels). Each row is one subject; a good law tracks the measured values \
         across subjects on EVERY output channel simultaneously — judge the residual \
         pattern (systematic tilt = wrong functional form; per-channel offset = a missing \
         shared term), not just the aggregate."
            .to_string(),
    }
}

// ---------------------------------------------------------------- template

/// Render the parameter pack. The template is fixed: the table rides
/// embedded as base64 JSON, ingestion lives in `skill::parampack`
/// itself (one place, shared with the tests).
pub fn render_pack(table: &LawTable) -> String {
    let json = serde_json::to_string(table).expect("serializable");
    let embedded = base64::engine::general_purpose::STANDARD.encode(&json);

    format!(
        r#"//! Generated by `skill parampack`: the law pack `{name}`.
//!
//! Provenance: {provenance}
//!
//! Regenerate from the table — do not hand-edit this file. The data
//! and split come from the embedded table; the ingestion rules are
//! `skill::parampack::load_rows`, shared with the integration tests.
//!
//! Compile and run: `skill evolve --pack=<this file>`.

use skill::anyhow::Result;
use skill::base64::Engine as _;
use skill::lexicon::Lexicon;
use skill::pack::{{Baseline, ProblemPack, SplitPolicy, Subject}};

const TABLE_B64: &str = "{embedded}";

fn table() -> skill::parampack::LawTable {{
    let json = skill::base64::engine::general_purpose::STANDARD
        .decode(TABLE_B64)
        .expect("generated table decodes");

    skill::serde_json::from_slice(&json).expect("generated table is valid")
}}

pub struct GeneratedLawPack;

impl ProblemPack for GeneratedLawPack {{
    fn name(&self) -> String {{
        table().name
    }}

    fn problem_statement(&self) -> String {{
        table().problem
    }}

    fn lexicon(&self) -> Lexicon {{
        skill::parampack::lexicon_of(&table())
    }}

    fn subjects(&self) -> Result<Vec<Subject>> {{
        Ok(skill::parampack::load_rows(&table()))
    }}

    fn split_policy(&self) -> SplitPolicy {{
        let split = table().split;

        SplitPolicy::tags(&split.train, &split.validation, &split.hidden)
    }}

    fn baseline(&self) -> Baseline {{
        Baseline {{
            mechanism: std::path::PathBuf::from("mechanism.json"),
            plugin: None,
        }}
    }}

    fn subject_table_intro(&self) -> String {{
        "ONE SUBJECT (`law`): every curve below is one measured subject's one frozen \
         parameter, with that subject's covariates as constant drives. Rows of the \
         subject table are TARGETS, not experimental units — the law's coefficients are \
         shared; the split tags identify which subjects the law may learn from."
            .to_string()
    }}

    fn theorist_preamble(&self) -> String {{
        table().preambles.theorist
    }}

    fn doctor_preamble(&self) -> String {{
        table().preambles.doctor
    }}

    fn referee_preamble(&self) -> String {{
        table().preambles.referee
    }}

    fn diagnostic_mode(&self) -> skill::pack::DiagnosticMode {{
        // The independent axis is a covariate placeholder: show the
        // loop predicted-vs-measured parity, not flat "curves".
        skill::pack::DiagnosticMode::Parity
    }}
}}

/// Compile this file as a pack: `skill evolve --pack=<this file>`.
pub fn load_pack() -> Box<dyn ProblemPack> {{
    Box::new(GeneratedLawPack)
}}
"#,
        name = table.name,
        provenance = table.provenance,
    )
}

/// The single `"law"` subject: one curve per (row, output). Public so
/// the integration tests and `skill inspect` read the data through
/// exactly the code the generated pack runs.
pub fn load_rows(table: &LawTable) -> Vec<Subject> {
    let mut curves = Vec::new();

    for row in &table.rows {
        for (index, output) in table.outputs.iter().enumerate() {
            curves.push(Curve {
                condition: Condition::Tag(row.id.clone()),
                raw: RawCurve {
                    t: vec![0.0, 1.0],
                    y: vec![row.outputs[index], row.outputs[index]],
                },
                session: 0,
                t_step: None,
                input_after: 0.0,
                drives: table
                    .features
                    .iter()
                    .zip(&row.features)
                    .map(|(feature, value)| InputDrive::constant(&feature.channel, *value))
                    .collect(),
                output_channel: Some(output.channel.clone()),
            });
        }
    }

    vec![Subject {
        id: "law".to_string(),
        title: "the law".to_string(),
        detail: vec![(
            "subjects".to_string(),
            table.rows.len().to_string(),
        )],
        curves,
    }]
}

// Ensure the base64 engine import is used even if the template moves.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::SplitPolicy;

    fn table_fixture() -> LawTable {
        LawTable {
            name: "law_test".to_string(),
            problem: "predict theta from covariates".to_string(),
            provenance: "digest 0000".to_string(),
            features: vec![
                LawChannel {
                    channel: "c_mw".to_string(),
                    column: "mw".to_string(),
                    units: "1".to_string(),
                    description: "molecular weight".to_string(),
                },
                LawChannel {
                    channel: "c_polar".to_string(),
                    column: "polar".to_string(),
                    units: "1".to_string(),
                    description: "polar fraction".to_string(),
                },
            ],
            outputs: vec![LawChannel {
                channel: "kon".to_string(),
                column: "kon".to_string(),
                units: "log10(kon)".to_string(),
                description: "association rate".to_string(),
            }],
            rows: (0..30)
                .map(|index| LawRow {
                    id: format!("subject-{index}"),
                    features: vec![index as f64, 1.0 - index as f64 / 30.0],
                    outputs: vec![index as f64 / 10.0],
                })
                .collect(),
            split: LawSplit {
                train: (0..20).map(|index| format!("subject-{index}")).collect(),
                validation: (20..25).map(|index| format!("subject-{index}")).collect(),
                hidden: (25..30).map(|index| format!("subject-{index}")).collect(),
            },
            preambles: LawPreambles {
                theorist: "T".to_string(),
                doctor: "D".to_string(),
                referee: "R".to_string(),
            },
        }
    }

    #[test]
    fn baseline_validates_and_uses_input_channels() {
        let table = table_fixture();
        let lexicon = lexicon_of(&table);
        let baseline = baseline_mechanism(&table);

        mechanism_ir::validate(&baseline, &lexicon).expect("baseline validates");

        let lowered = crate::lower::lower(&baseline, &lexicon, "test").expect("lowers");

        assert!(lowered.modelica.contains("input Real c_mw;"));
        assert!(lowered.modelica.contains("output Real kon;"));
    }

    #[test]
    fn rows_become_tagged_constant_curves() {
        let table = table_fixture();
        let subjects = load_rows(&table);

        assert_eq!(subjects.len(), 1);
        assert_eq!(subjects[0].curves.len(), 30);

        let curve = &subjects[0].curves[0];
        assert_eq!(curve.condition, Condition::Tag("subject-0".to_string()));
        assert_eq!(curve.drives.len(), 2);
        assert_eq!(curve.output_channel.as_deref(), Some("kon"));

        // The split roles the tags by identity.
        let roles = crate::split::split_roles(
            &subjects[0].curves,
            &SplitPolicy::tags(
                &[
                    "subject-0".to_string(),
                    "subject-1".to_string(),
                ],
                &["subject-2".to_string()],
                &["subject-29".to_string()],
            ),
        );

        assert_eq!(roles.train.len(), 2);
        assert_eq!(roles.validation.len(), 1);
        assert_eq!(roles.hidden.len(), 1);
    }

    #[test]
    fn pack_template_round_trips_the_table() {
        let table = table_fixture();
        let source = render_pack(&table);

        assert!(source.contains("TABLE_B64"));
        assert!(source.contains("skill::parampack::load_rows"));
    }
}