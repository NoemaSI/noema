//! Pack authoring: the `skill new` interview and its deterministic
//! tools (data survey, mapping-driven loading, curve preview).
//!
//! A scientist answers questions in their own vocabulary; nothing
//! here mentions packs, plugins, or Modelica. The interview records
//! its outcome as `packs/<name>/session.json` — pure data (paths,
//! column roles, channel declarations, split draft) that the later
//! stages consume to draft the baseline mechanism and generate the
//! pack. The loading implemented here ([`load_subjects`]) is exactly
//! the algorithm the generated pack's `subjects()` will carry, so a
//! preview and the eventual pack read the data the same way.

use std::collections::{BTreeSet, HashMap};
use std::io::IsTerminal;

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::data::RawCurve;
use crate::experiment::{Condition, InputDrive};
use crate::pack::Subject;

/// Everything the interview learns: the whole authoring state up to
/// the point code generation starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorSession {
    pub name: String,
    pub problem_statement: String,

    /// The scientist's answer to "is there a textbook model for
    /// this?" — words or equations, fed verbatim to the drafting
    /// stage. `None`/empty means "let the system propose".
    #[serde(default)]
    pub baseline_hint: Option<String>,

    /// The scientist's own words about a data layout the interview
    /// could not map to columns — the job description for
    /// [`crate::prepare`]. Set when the interview takes the
    /// needs-preparation branch; unused by the fast lane.
    #[serde(default)]
    pub prepare_brief: Option<String>,

    pub data: DataSpec,
    pub mapping: ColumnMap,
    pub split: SplitDraft,
}

/// Where the measured data lives. The interview accepts tabular
/// files only (CSV/TSV): column mapping is the declared ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSpec {
    pub path: String,
    pub delimiter: char,
}

/// The column-role mapping: which raw column plays which part.
/// Every field is optional: a session may exist before
/// [`crate::prepare`] has discovered where those parts live
/// ([`mapping_gaps`] is the single completeness check).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMap {
    /// Column naming the experimental unit (one subject).
    #[serde(default)]
    pub subject: Option<String>,

    /// Columns identifying one recorded time series within a
    /// subject (may be empty: one curve per subject).
    #[serde(default)]
    pub curve: Vec<String>,

    #[serde(default)]
    pub time: Option<String>,

    /// Controlled signals (what the experimenter set).
    #[serde(default)]
    pub drives: Vec<ChannelColumn>,

    /// Measured signals (what the instrument reported).
    #[serde(default)]
    pub outputs: Vec<ChannelColumn>,

    /// Column whose value labels each curve's condition. `None`
    /// means the condition is the first drive channel's value.
    pub condition_column: Option<String>,
}

/// One drive or output channel mapped from a raw column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelColumn {
    /// The raw column, once known — `None` until the interview's
    /// fast lane or [`crate::prepare`] supplies it.
    #[serde(default)]
    pub column: Option<String>,

    pub name: String,
    pub units: String,
    pub description: String,

    /// `true`: the column is recorded per row (a full trace).
    /// `false`: constant within one curve (a set level).
    pub recorded: bool,
}

/// What the mapping still needs before any loading can happen:
/// empty means the session is complete.
pub fn mapping_gaps(session: &AuthorSession) -> Vec<String> {
    let mut gaps = Vec::new();

    if session.mapping.subject.is_none() {
        gaps.push("subject column".to_string());
    }

    if session.mapping.time.is_none() {
        gaps.push("time column".to_string());
    }

    for channel in session.mapping.drives.iter().chain(&session.mapping.outputs) {
        if channel.column.is_none() {
            gaps.push(format!("column for channel `{}`", channel.name));
        }
    }

    if session.mapping.outputs.is_empty() {
        gaps.push("at least one output channel".to_string());
    }

    gaps
}

/// The proposed train/validation/hidden conditions, as condition
/// display strings (numeric levels or tags).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SplitDraft {
    pub train: Vec<String>,
    pub validation: Vec<String>,
    pub hidden: Vec<String>,
}

// ---------------------------------------------------------------- survey

/// Column-level picture of a tabular file: enough for a human to
/// point at the right columns without opening the file.
#[derive(Debug, Serialize)]
pub struct Survey {
    pub path: String,
    pub rows: usize,
    pub columns: Vec<ColumnSurvey>,
}

#[derive(Debug, Serialize)]
pub struct ColumnSurvey {
    pub name: String,
    pub numeric: bool,
    pub empty: usize,
    pub distinct: usize,
    pub distinct_truncated: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub samples: Vec<String>,
}

/// Cap on rows a survey reads, so huge files stay cheap.
pub const SURVEY_ROWS: usize = 50_000;

fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim();

    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Some(value),
        _ => None,
    }
}

pub fn survey(path: &str, delimiter: char, head: usize) -> Result<Survey> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter as u8)
        .from_path(path)
        .with_context(|| format!("opening {path}"))?;

    let headers = reader.headers().context("reading headers")?.clone();

    struct Acc {
        numeric: bool,
        seen: bool,
        empty: usize,
        values: BTreeSet<String>,
        min: Option<f64>,
        max: Option<f64>,
        samples: Vec<String>,
    }

    let mut columns: Vec<Acc> = headers
        .iter()
        .map(|_| Acc {
            numeric: true,
            seen: false,
            empty: 0,
            values: BTreeSet::new(),
            min: None,
            max: None,
            samples: Vec::new(),
        })
        .collect();

    let mut rows = 0;

    for record in reader.records().take(head) {
        let record = record.context("parsing row")?;

        rows += 1;

        for (index, field) in record.iter().enumerate() {
            let Some(column) = columns.get_mut(index) else {
                continue;
            };

            let field = field.trim();

            if field.is_empty() {
                column.empty += 1;
                continue;
            }

            column.seen = true;

            if column.values.len() < 1001 {
                column.values.insert(field.to_string());
            }

            if column.samples.len() < 3 && !column.samples.iter().any(|s| s == field) {
                column.samples.push(field.to_string());
            }

            match parse_number(field) {
                Some(value) => {
                    column.min = Some(column.min.map_or(value, |m| m.min(value)));
                    column.max = Some(column.max.map_or(value, |m| m.max(value)));
                }
                None => column.numeric = false,
            }
        }
    }

    Ok(Survey {
        path: path.to_string(),
        rows,
        columns: headers
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let column = &columns[index];

                ColumnSurvey {
                    name: name.to_string(),
                    numeric: column.numeric && column.seen,
                    empty: column.empty,
                    distinct: column.values.len().min(1000),
                    distinct_truncated: column.values.len() > 1000,
                    min: column.min,
                    max: column.max,
                    samples: column.samples.clone(),
                }
            })
            .collect(),
    })
}

/// Human-readable survey for the interview terminal.
pub fn survey_text(survey: &Survey) -> String {
    let mut text = format!("{} rows\n", survey.rows);

    for column in &survey.columns {
        let range = match (column.min, column.max) {
            (Some(min), Some(max)) => format!("  range [{min}, {max}]"),
            _ => String::new(),
        };

        text.push_str(&format!(
            "  {:<28} {:>7}  distinct {:>5}{}  empty {:>5}  e.g. {}{range}\n",
            column.name,
            if column.numeric { "number" } else { "text" },
            column.distinct,
            if column.distinct_truncated { "+" } else { "" },
            column.empty,
            column.samples.join(" | "),
        ));
    }

    text
}

// ---------------------------------------------------------------- loading

/// Load the mapped file into [`Subject`] records — the fixed
/// ingestion algorithm the generated pack will carry.
pub fn load_subjects(session: &AuthorSession) -> Result<Vec<Subject>> {
    let gaps = mapping_gaps(session);

    if !gaps.is_empty() {
        bail!(
            "mapping incomplete ({}): run `skill new prepare` first",
            gaps.join(", "),
        );
    }

    fn ready(name: &Option<String>) -> &str {
        name.as_deref().expect("gaps checked")
    }

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(session.data.delimiter as u8)
        .from_path(&session.data.path)
        .with_context(|| format!("opening {}", session.data.path))?;

    let headers = reader.headers().context("reading headers")?.clone();

    let column = |name: &str| -> Result<usize> {
        headers
            .iter()
            .position(|header| header == name)
            .with_context(|| format!("no column `{name}` in {}", session.data.path))
    };

    let subject_c = column(ready(&session.mapping.subject))?;
    let time_c = column(ready(&session.mapping.time))?;
    let curve_cs: Vec<usize> = session
        .mapping
        .curve
        .iter()
        .map(|name| column(name))
        .collect::<Result<_>>()?;
    let condition_c = session
        .mapping
        .condition_column
        .as_deref()
        .map(column)
        .transpose()?;

    let drive_cs: Vec<usize> = session
        .mapping
        .drives
        .iter()
        .map(|channel| column(ready(&channel.column)))
        .collect::<Result<_>>()?;

    let output_cs: Vec<usize> = session
        .mapping
        .outputs
        .iter()
        .map(|channel| column(ready(&channel.column)))
        .collect::<Result<_>>()?;

    struct Row {
        t: f64,
        drives: Vec<Option<f64>>,
        outputs: Vec<Option<f64>>,
        condition: Option<String>,
    }

    let mut order: Vec<(String, String)> = Vec::new();
    let mut groups: HashMap<(String, String), Vec<Row>> = HashMap::new();

    for record in reader.records() {
        let record = record.context("parsing row")?;

        let field = |index: usize| record.get(index).unwrap_or("").trim().to_string();

        let Some(t) = parse_number(&field(time_c)) else {
            continue;
        };

        let key = (
            field(subject_c),
            curve_cs
                .iter()
                .map(|&index| field(index))
                .collect::<Vec<_>>()
                .join("/"),
        );

        let row = Row {
            t,
            drives: drive_cs.iter().map(|&index| parse_number(&field(index))).collect(),
            outputs: output_cs.iter().map(|&index| parse_number(&field(index))).collect(),
            condition: condition_c.map(|index| field(index)),
        };

        if !groups.contains_key(&key) {
            order.push(key.clone());
        }

        groups.entry(key).or_default().push(row);
    }

    let mut subjects: Vec<Subject> = Vec::new();
    let mut sessions = 0;

    for key in order {
        let (subject_id, curve_id) = (&key.0, &key.1);

        let mut rows = groups.remove(&key).expect("group present");
        rows.sort_by(|a, b| a.t.total_cmp(&b.t));

        let condition = match &session.mapping.condition_column {
            Some(_) => rows
                .iter()
                .find_map(|row| row.condition.clone())
                .map(|value| match parse_number(&value) {
                    Some(level) => Condition::Level(level),
                    None => Condition::Tag(value),
                })
                .unwrap_or(Condition::Tag("unset".to_string())),
            None => Condition::Level(
                rows.iter()
                    .find_map(|row| row.drives.first().copied().flatten())
                    .unwrap_or(0.0),
            ),
        };

        let drives: Vec<InputDrive> = session
            .mapping
            .drives
            .iter()
            .enumerate()
            .map(|(index, channel)| {
                if channel.recorded {
                    let mut last = rows
                        .iter()
                        .find_map(|row| row.drives[index])
                        .unwrap_or(0.0);

                    let values = rows
                        .iter()
                        .map(|row| {
                            if let Some(value) = row.drives[index] {
                                last = value;
                            }
                            last
                        })
                        .collect();

                    InputDrive {
                        channel: channel.name.clone(),
                        t: rows.iter().map(|row| row.t).collect(),
                        values,
                    }
                } else {
                    let values: Vec<f64> =
                        rows.iter().filter_map(|row| row.drives[index]).collect();

                    let first = values.first().copied().unwrap_or(0.0);

                    if values
                        .iter()
                        .any(|value| (value - first).abs() > 1e-9 * first.abs().max(1.0))
                    {
                        eprintln!(
                            "skill: warning: `{}` varies within curve {subject_id}/{curve_id} \
                             but is declared constant — first value used; \
                             is it recorded per row?",
                            channel.column.as_deref().unwrap_or(&channel.name),
                        );
                    }

                    InputDrive::constant(&channel.name, first)
                }
            })
            .collect();

        let curves = session
            .mapping
            .outputs
            .iter()
            .enumerate()
            .map(|(index, channel)| {
                let (t, y): (Vec<f64>, Vec<f64>) = rows
                    .iter()
                    .filter_map(|row| row.outputs[index].map(|value| (row.t, value)))
                    .unzip();

                crate::data::Curve {
                    condition: condition.clone(),
                    raw: RawCurve { t, y },
                    session: sessions,
                    t_step: None,
                    input_after: 0.0,
                    drives: drives.clone(),
                    output_channel: Some(channel.name.clone()),
                }
            })
            .collect();

        sessions += 1;

        match subjects.iter_mut().find(|subject| subject.id == *subject_id) {
            Some(subject) => subject.curves.extend(curves),
            None => subjects.push(Subject {
                id: subject_id.clone(),
                title: subject_id.clone(),
                detail: vec![],
                curves,
            }),
        }
    }

    Ok(subjects)
}

/// Distinct conditions across all curves, in first-seen order,
/// canonicalized through [`Condition::display`].
pub fn conditions_of(subjects: &[Subject]) -> Vec<Condition> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut conditions = Vec::new();

    for subject in subjects {
        for curve in &subject.curves {
            if seen.insert(curve.condition.display()) {
                conditions.push(curve.condition.clone());
            }
        }
    }

    conditions.sort_by(|a, b| match (a, b) {
        (Condition::Level(a), Condition::Level(b)) => a.total_cmp(b),
        (Condition::Tag(a), Condition::Tag(b)) => a.cmp(b),
        (Condition::Level(_), Condition::Tag(_)) => std::cmp::Ordering::Less,
        (Condition::Tag(_), Condition::Level(_)) => std::cmp::Ordering::Greater,
    });

    conditions
}

/// Default firewall proposal: levels sorted, lowest third to train,
/// next to validation, the rest held out.
pub fn propose_split(conditions: &[Condition]) -> SplitDraft {
    let k = conditions.len();

    let per = (k / 3).max(1);
    let train = per.min(k.saturating_sub(2).max(1));
    let validation = per.min(k.saturating_sub(train).max(1));

    SplitDraft {
        train: conditions[..train].iter().map(Condition::display).collect(),
        validation: conditions[train..train + validation]
            .iter()
            .map(Condition::display)
            .collect(),
        hidden: conditions[train + validation..]
            .iter()
            .map(Condition::display)
            .collect(),
    }
}

// ---------------------------------------------------------------- preview

/// Data-only figure: measured curves, one panel per subject, no
/// model — the interview's "is this what you measured?" check.
pub fn preview_png(
    session: &AuthorSession,
    subjects: &[Subject],
    panels: usize,
) -> Result<Vec<u8>> {
    let output = session
        .mapping
        .outputs
        .first()
        .context("no output channels")?;

    let step = (subjects.len() / panels).max(1);

    let chosen: Vec<&Subject> = subjects.iter().step_by(step).take(panels).collect();

    let panels: Vec<(String, Vec<crate::plot::DataCurve>)> = chosen
        .iter()
        .map(|subject| {
            let curves = subject
                .curves
                .iter()
                .map(|curve| crate::plot::DataCurve {
                    x: curve.raw.t.clone(),
                    y: curve.raw.y.clone(),
                    label: format!(
                        "{} #{}",
                        curve.condition.display(),
                        curve.session + 1,
                    ),
                })
                .collect();

            (subject.id.clone(), curves)
        })
        .collect();

    crate::plot::data_preview_png(
        &format!("{} — measured data", session.name),
        &format!("{} ({})", output.name, output.units),
        &panels,
    )
}

// ---------------------------------------------------------------- interview

/// Read a `session.json` (the single source of truth every stage
/// re-reads, so stages can run in any order after prepare).
pub fn read_session(path: &str) -> Result<AuthorSession> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {path}"))?;

    serde_json::from_str(&text).with_context(|| format!("parsing {path}"))
}

/// Write a `session.json`.
pub fn write_session(path: &std::path::Path, session: &AuthorSession) -> Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(session)?)
        .with_context(|| format!("writing {}", path.display()))
}

pub(crate) fn ask_line(prompt: &str, default: Option<&str>, allow_empty: bool) -> Result<String> {
    let stdin = std::io::stdin();

    loop {
        print!("skill: {prompt}");

        if let Some(default) = default {
            print!(" [{default}]");
        }

        println!(":");

        std::io::Write::flush(&mut std::io::stdout())?;

        let mut line = String::new();

        if stdin.read_line(&mut line)? == 0 {
            bail!("interview ended (no more input)");
        }

        let answer = line.trim();

        if answer.is_empty() {
            if let Some(default) = default {
                return Ok(default.to_string());
            }
            if allow_empty {
                return Ok(String::new());
            }
            continue;
        }

        return Ok(answer.to_string());
    }
}

pub(crate) fn ask(prompt: &str, default: Option<&str>) -> Result<String> {
    ask_line(prompt, default, false)
}

pub(crate) fn ask_optional(prompt: &str, default: Option<&str>) -> Result<String> {
    ask_line(prompt, default, true)
}

fn ask_column(columns: &[String], prompt: &str, default: Option<&str>) -> Result<String> {
    loop {
        let answer = ask(prompt, default)?;

        if let Some(index) = answer.parse::<usize>().ok().filter(|i| *i > 0) {
            if let Some(name) = columns.get(index - 1) {
                return Ok(name.clone());
            }
        } else if columns.iter().any(|name| name == &answer) {
            return Ok(answer);
        }

        println!("skill: pick one of: {}", columns.join(", "));
    }
}

fn ask_channels(
    columns: &[String],
    kind: &str,
    columns_known: bool,
) -> Result<Vec<ChannelColumn>> {
    let mut channels = Vec::new();

    println!("skill: which {kind} channels exist?");

    loop {
        let prompt = if columns_known {
            format!("  {kind} column (blank = done)")
        } else {
            format!("  {kind} channel name (blank = done)")
        };

        let answer = ask_optional(&prompt, None)?;

        if answer.is_empty() {
            break;
        }

        let column = if columns_known {
            if !columns.iter().any(|name| name == &answer) {
                println!("skill: no such column; pick one of: {}", columns.join(", "));
                continue;
            }

            Some(answer.clone())
        } else {
            let answer_column = ask_optional(
                "  column in the file? (blank = the preparation agent will find it)",
                None,
            )?;

            if answer_column.is_empty() {
                None
            } else if columns.iter().any(|name| name == &answer_column) {
                Some(answer_column)
            } else {
                println!("skill: no such column; leaving it for the preparation agent");
                None
            }
        };

        let name = if columns_known {
            ask("  channel name (one word, used in the model)", Some(&answer))?
        } else {
            answer.clone()
        };

        let units = ask("  units", Some("1"))?;
        let description = ask("  what does this number physically mean?", Some(&name))?;
        let recorded = ask("  recorded per row (a full trace)? [y/n]", Some("y"))?;

        channels.push(ChannelColumn {
            column,
            name,
            units,
            description,
            recorded: recorded.starts_with('y'),
        });
    }

    Ok(channels)
}

/// Parse an edited split line: `train=a,b | validation=c | hidden=d,e`.
pub(crate) fn parse_split(text: &str) -> Result<SplitDraft> {
    let mut split = SplitDraft::default();

    for part in text.split('|') {
        let (role, values) = part
            .split_once('=')
            .with_context(|| format!("expected role=values, got `{part}`"))?;

        let values: Vec<String> = values
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        match role.trim() {
            "train" => split.train = values,
            "validation" => split.validation = values,
            "hidden" => split.hidden = values,
            role => bail!("unknown role `{role}` (train/validation/hidden)"),
        }
    }

    Ok(split)
}

// ---------------------------------------------------------------- generation

/// A split-draft display string back into a condition: numeric
/// strings are levels, anything else a tag.
pub fn parse_condition(text: &str) -> Condition {
    match text.trim().parse::<f64>() {
        Ok(value) if value.is_finite() => Condition::Level(value),
        _ => Condition::Tag(text.trim().to_string()),
    }
}

/// Render the `pack.rs` the session describes. The template is
/// fixed: the generated pack embeds the session JSON and calls back
/// into this module, so the ingestion algorithm lives in exactly one
/// place and a regenerated pack reads its data identically to the
/// interview preview.
pub fn generate_pack(session: &AuthorSession) -> String {
    let json = serde_json::to_string_pretty(session).expect("serializable");

    // Base64 keeps the session out of any string-delimiter trouble.
    let embedded = base64::engine::general_purpose::STANDARD.encode(&json);

    format!(
        r#"//! Generated by `skill new` for the problem `{name}`.
//!
//! Edit the interview (or `session.json` in this directory) and
//! regenerate — do not hand-edit this file. Data ingestion and the
//! split come from the embedded session; the loading rules are
//! `skill::author::load_subjects`, shared with `skill new preview`.
//!
//! Compile and run: `skill evolve --pack=<this file>`.

use skill::anyhow::Result;
use skill::base64::Engine as _;
use skill::lexicon::{{Channel, Lexicon}};
use skill::pack::{{Baseline, ProblemPack, SplitPolicy, Subject}};

const SESSION_B64: &str = "{embedded}";

fn session() -> skill::author::AuthorSession {{
    let json = skill::base64::engine::general_purpose::STANDARD
        .decode(SESSION_B64)
        .expect("generated session decodes");

    skill::serde_json::from_slice(&json).expect("generated session is valid")
}}

pub struct GeneratedPack;

impl ProblemPack for GeneratedPack {{
    fn name(&self) -> String {{
        session().name
    }}

    fn problem_statement(&self) -> String {{
        session().problem_statement
    }}

    fn lexicon(&self) -> Lexicon {{
        let session = session();

        let channel = |c: &skill::author::ChannelColumn| Channel {{
            name: c.name.clone(),
            units: c.units.clone(),
            description: c.description.clone(),
        }};

        Lexicon {{
            inputs: session.mapping.drives.iter().map(channel).collect(),
            outputs: session.mapping.outputs.iter().map(channel).collect(),
            roles: vec![],
            notes: "Drives are replayed exactly as recorded (zero-order \
             hold); they are data, never fitted."
                .to_string(),
        }}
    }}

    fn subjects(&self) -> Result<Vec<Subject>> {{
        skill::author::load_subjects(&session())
    }}

    fn split_policy(&self) -> SplitPolicy {{
        let session = session();

        let conditions = |values: &[String]| -> Vec<_> {{
            values.iter().map(|value| skill::author::parse_condition(value)).collect()
        }};

        SplitPolicy {{
            train: conditions(&session.split.train),
            validation: conditions(&session.split.validation),
            hidden: conditions(&session.split.hidden),
        }}
    }}

    fn baseline(&self) -> Baseline {{
        Baseline {{
            mechanism: std::path::PathBuf::from("mechanism.json"),
            plugin: None,
        }}
    }}
}}

/// Compile this file as a pack: `skill evolve --pack=<this file>`.
pub fn load_pack() -> Box<dyn ProblemPack> {{
    Box::new(GeneratedPack)
}}
"#,
        name = session.name,
    )
}

// ---------------------------------------------------------------- generation

/// The lexicon the session declares — the same construction the
/// generated pack performs.
pub fn lexicon_of(session: &AuthorSession) -> crate::lexicon::Lexicon {
    let channel = |c: &ChannelColumn| crate::lexicon::Channel {
        name: c.name.clone(),
        units: c.units.clone(),
        description: c.description.clone(),
    };

    crate::lexicon::Lexicon {
        inputs: session.mapping.drives.iter().map(channel).collect(),
        outputs: session.mapping.outputs.iter().map(channel).collect(),
        roles: vec![],
        notes: "Drives are replayed exactly as recorded (zero-order hold); \
         they are data, never fitted."
            .to_string(),
    }
}

/// A token-cheap digest of the measured reality: what a drafting
/// LLM gets in place of tools. Up to five subjects, each curve
/// summarized by span, range, and eight sampled values.
pub fn data_digest(session: &AuthorSession, subjects: &[Subject]) -> String {
    let mut text = String::new();

    text.push_str(&format!("PROBLEM: {}\n\n", session.problem_statement));

    for channel in &session.mapping.drives {
        text.push_str(&format!(
            "DRIVE `{}` ({}, {}): {}\n",
            channel.name,
            channel.units,
            if channel.recorded { "recorded trace" } else { "constant per curve" },
            channel.description,
        ));
    }

    for channel in &session.mapping.outputs {
        text.push_str(&format!(
            "OUTPUT `{}` ({}, recorded): {}\n",
            channel.name, channel.units, channel.description,
        ));
    }

    text.push_str("\nCONDITIONS:");

    for condition in conditions_of(subjects) {
        let count = subjects
            .iter()
            .flat_map(|subject| &subject.curves)
            .filter(|curve| curve.condition.matches(&condition))
            .count();

        text.push_str(&format!(" {} ({curves} curves)", condition.display(), curves = count));
    }

    // Bound guidance, derived from the data rather than assumed: the
    // most common silent failure is a sensitivity constant whose
    // declared range cannot reach the condition range the data
    // explores, which pins the fit and flattens the response.
    let levels: Vec<f64> = conditions_of(subjects)
        .iter()
        .filter_map(|condition| condition.level())
        .collect();

    text.push_str("\n\nPARAMETER BOUNDS:");

    if levels.len() >= 2 {
        let lo = levels.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        text.push_str(&format!(
            " the condition axis spans {lo}..{hi}. Any parameter that sets where or how \
             sharply the mechanism responds along that axis (thresholds, half-saturation \
             or half-inhibition constants, switch points, time constants of the response) \
             MUST have min/max bracketing that span generously — roughly lo/10 .. hi*10 — \
             so the fit can place it anywhere the data points. A parameter whose range \
             cannot reach the condition range gets stuck at its bound and the mechanism \
             stops responding to the conditions; the harness reports such pins as \
             sentinels, but do not plan on repair: propose honest ranges now."
        ));
    } else {
        text.push_str(
            " the condition axis is categorical or single-valued; set every parameter's \
             min/max wide enough that the fit is never limited by the declared range.",
        );
    }

    text.push_str("\n\nMEASURED (subjects up to 5, curves up to 4 each):\n");

    for subject in subjects.iter().take(5) {
        text.push_str(&format!("{}:\n", subject.id));

        for curve in subject.curves.iter().take(4) {
            let points = curve.raw.t.len();

            let sampled: Vec<String> = (0..8)
                .map(|slot| slot * points / 8)
                .map(|index| curve.raw.y.get(index).copied().unwrap_or(f64::NAN))
                .map(|value| format!("{value:.3}"))
                .collect();

            text.push_str(&format!(
                "  condition {} | t {:.1}..{:.1} ({points} samples) | y {:.3}..{:.3} | y ~ [{}]\n",
                curve.condition.display(),
                curve.raw.t.first().copied().unwrap_or(0.0),
                curve.raw.t.last().copied().unwrap_or(0.0),
                curve.raw.y.iter().copied().fold(f64::INFINITY, f64::min),
                curve.raw.y.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                sampled.join(", "),
            ));
        }
    }

    text
}

/// Pull the first JSON object out of a reply: a fenced block if
/// present, else the first balanced `{...}` (string-aware).
fn extract_json(reply: &str) -> Option<&str> {
    if let Some(start) = reply.find("```") {
        let body = &reply[start + 3..];
        let body = body.strip_prefix("json").unwrap_or(body);
        let body = body.trim_start_matches(['\n', '\r', ' ']);

        if let Some(end) = body.find("```") {
            let candidate = body[..end].trim();

            if candidate.starts_with('{') {
                return Some(candidate);
            }
        }
    }

    let start = reply.find('{')?;
    let bytes = reply.as_bytes();

    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, byte) in bytes.iter().enumerate().skip(start) {
        match byte {
            b'"' if !escaped => in_string = !in_string,
            b'\\' if in_string => escaped = !escaped,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;

                if depth == 0 {
                    return Some(&reply[start..=offset]);
                }
            }
            _ => escaped = false,
        }
    }

    None
}

/// Draft the pack's baseline `mechanism.json`: one reasoning agent,
/// context-injected (digest + preview image + the scientist's
/// textbook-model hint), repaired against `validate`/`lower` errors
/// up to `tries` times. No tools, no file access.
pub fn draft(session: &AuthorSession, tries: usize) -> Result<()> {
    use crate::evolve::theorist_preamble_static;
    use crate::mechanism_ir::{self, Mechanism};

    let config = crate::agent::LlmConfig::from_env()?;

    #[cfg(feature = "viewer")]
    crate::viewer::serve_local();

    let lexicon = lexicon_of(session);
    let subjects = load_subjects(session)?;

    let preamble = format!(
        "{}\n\nYou are drafting the generation-0 BASELINE mechanism for a \
         brand-new experimental problem.\n\
         \n\
         A baseline is a STARTING POINT, not a solution. It will be \
         handed to a discovery loop that exists precisely to fix and \
         improve it. Your job is only:\n\
         1. a valid IR object that compiles and lowers,\n\
         2. the standard model (as hinted, else the simplest one the \
            digest supports), with honest parameter ranges,\n\
         3. NOTHING else. Do not try to explain every feature of the \
            data, do not add mechanisms to fix residual patterns you \
            can predict, do not optimize. A slightly-wrong simple \
            baseline beats a polished complex one — the loop \
            iterates, you do not get a second look at the data.\n\
         \n\
         Where you are unsure about an IR syntax or semantic detail, \
         pick the simplest legal reading and emit it: structural \
         mistakes come back to you as precise validator errors on the \
         next attempt, so deliberating about them now is wasted. \
         Known answers: a process with an empty `from` list is legal \
         (production from a source); a driven entity's state always \
         holds the channel's recorded value. Aim \
         for at most ~6 parameters and one dynamic state per obvious \
         timescale.\n\
         \n\
         Reply with ONLY the mechanism JSON object (the full object, \
         every field) — no prose, no commentary.",
        theorist_preamble_static(&lexicon),
    );

    let agent = crate::agent::theorist::<Mechanism>(&config, &preamble)?;

    let image = preview_png(session, &subjects, 2)
        .ok()
        .map(|png| crate::plot::encode_base64(&png));

    let mut prompt = data_digest(session, &subjects);

    match &session.baseline_hint {
        Some(hint) if !hint.trim().is_empty() => {
            prompt.push_str(&format!(
                "\nThe scientist reports a standard model for this process:\n{hint}\n\
                 Use it as the baseline's structure unless the data contradicts it."
            ));
        }
        _ => prompt.push_str(
            "\nNo standard model was offered: propose the simplest mechanism \
             the digest supports.",
        ),
    }

    let mut last = String::from("no reply parsed");

    for attempt in 1..=tries {
        println!("skill: draft attempt {attempt}/{tries} — asking the model...");

        let reply = crate::agent::run_message(&agent, &prompt, image.as_deref())?;

        let Some(json) = extract_json(&reply) else {
            last = "no JSON object in the reply".to_string();
            prompt = "Your reply contained no JSON object. Reply with ONLY the \
                      complete mechanism JSON object."
                .to_string();
            continue;
        };

        let mechanism: Mechanism = match serde_json::from_str(json) {
            Ok(mechanism) => mechanism,
            Err(error) => {
                last = format!("invalid mechanism JSON: {error}");
                prompt = format!(
                    "Your mechanism JSON below does not parse: {error}\n\n{json}\n\n\
                     Fix ONLY the syntax error at the reported position. Keep every \
                     name, state, parameter, and equation exactly as written — do NOT \
                     propose a new mechanism. Reply with ONLY the corrected JSON object."
                );
                continue;
            }
        };

        let mut errors = String::new();

        if let Err(validation) = mechanism_ir::validate(&mechanism, &lexicon) {
            errors.push_str(&validation);
        }

        if errors.is_empty() {
            if let Err(lowering) = crate::lower::lower(&mechanism, &lexicon, "draft") {
                errors.push_str(&lowering);
            }
        }

        if !errors.is_empty() {
            last = errors.clone();
            prompt = format!(
                "The mechanism below failed the harness checks:\n{errors}\n\n{json}\n\n\
                 Fix ONLY the reported errors in this mechanism, minimally. Keep its \
                 name, its entities, states, parameters, and structure unless an error \
                 forces a change — do NOT propose a new mechanism. Reply with ONLY the \
                 complete, corrected mechanism JSON object."
            );
            continue;
        }

        let dir = std::path::Path::new("packs").join(&session.name);

        std::fs::create_dir_all(&dir)?;

        let file = dir.join("mechanism.json");

        std::fs::write(&file, serde_json::to_string_pretty(&mechanism)?)?;

        println!("skill: baseline mechanism `{}` -> {}", mechanism.name, file.display());
        println!(
            "skill: next:  skill new generate --session={} && \
             skill new validate --pack={} && skill new smoke --pack={}",
            file.with_file_name("session.json").display(),
            file.with_file_name("pack.rs").display(),
            file.with_file_name("pack.rs").display(),
        );

        return Ok(());
    }

    bail!("drafting failed after {tries} attempts — last errors:\n{last}")
}

// ---------------------------------------------------------------- gates

/// T3: compile the pack and report the reality it declares — the
/// machine's acceptance check after `generate`.
fn validate(pack_path: &str) -> Result<()> {
    let handle = crate::pack_loader::load_pack(std::path::Path::new(pack_path))?;

    let pack = handle.pack.as_ref();

    println!("skill: pack `{}` compiled", pack.name());

    let lexicon = pack.lexicon();

    for channel in &lexicon.inputs {
        println!("skill:   input `{}` ({})", channel.name, channel.units);
    }

    for channel in &lexicon.outputs {
        println!("skill:   output `{}` ({})", channel.name, channel.units);
    }

    let policy = pack.split_policy();

    let role_count = |conditions: &[Condition], subjects: &[Subject]| -> usize {
        subjects
            .iter()
            .flat_map(|subject| &subject.curves)
            .filter(|curve| {
                conditions
                    .iter()
                    .any(|condition| curve.condition.matches(condition))
            })
            .count()
    };

    let subjects = pack.subjects()?;

    println!(
        "skill:   {} subjects, {} curves",
        subjects.len(),
        subjects.iter().map(|subject| subject.curves.len()).sum::<usize>(),
    );

    for (role, conditions) in [
        ("train", &policy.train),
        ("validation", &policy.validation),
        ("hidden", &policy.hidden),
    ] {
        let count = role_count(conditions, &subjects);

        if count == 0 {
            println!("skill:   WARNING: no curves match the {role} conditions");
        } else {
            println!("skill:   {role}: {} curves", count);
        }
    }

    let baseline = pack.baseline();
    let mechanism = handle.resolve(&baseline.mechanism);

    if mechanism.exists() {
        println!("skill:   baseline mechanism {}", mechanism.display());
    } else {
        println!(
            "skill:   baseline mechanism MISSING ({}) — the next stage drafts it",
            mechanism.display(),
        );
    }

    Ok(())
}

// ---------------------------------------------------------------- commands

/// The `skill new` subcommand tree.
pub fn run(args: &[String]) -> Result<(), String> {
    let flag = |name: &str| {
        args.iter()
            .find_map(|arg| arg.strip_prefix(&format!("--{name}=")))
            .map(str::to_string)
    };

    match args.first().map(String::as_str).unwrap_or("") {
        // NOTE: this is the first step in the skill creation workflow
        "" => crate::prepare::new_flow().map_err(|error| format!("{error:#}")),
        "survey" => {
            let path = flag("data").ok_or("usage: skill new survey --data=<file> [--delim=,] [--head=N] [--json]")?;
            let delimiter = flag("delim")
                .and_then(|text| text.chars().next())
                .unwrap_or(',');
            let head = flag("head")
                .map(|text| text.parse().unwrap_or(SURVEY_ROWS))
                .unwrap_or(SURVEY_ROWS);

            let survey = survey(&path, delimiter, head).map_err(|error| format!("{error:#}"))?;

            if args.iter().any(|arg| arg == "--json") {
                println!("{}", serde_json::to_string_pretty(&survey).expect("serializable"));
            } else {
                print!("{}", survey_text(&survey));
            }

            Ok(())
        }

        "preview" => {
            let path = flag("session").ok_or("usage: skill new preview --session=<session.json> [--out=<png>] [--panels=N]")?;

            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("reading {path}: {error}"))?;

            let session: AuthorSession = serde_json::from_str(&text)
                .map_err(|error| format!("parsing {path}: {error}"))?;

            let subjects = load_subjects(&session).map_err(|error| format!("{error:#}"))?;
            let panels = flag("panels").map(|text| text.parse().unwrap_or(3)).unwrap_or(3);

            let png = preview_png(&session, &subjects, panels).map_err(|error| format!("{error:#}"))?;

            let out = flag("out").unwrap_or_else(|| format!("{}-preview.png", session.name));

            std::fs::write(&out, png).map_err(|error| format!("writing {out}: {error}"))?;

            println!("skill: preview of {} subjects -> {out}", subjects.len());

            Ok(())
        }

        "draft" => {
            let path = flag("session").ok_or("usage: skill new draft --session=<session.json> [--tries=N]")?;

            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("reading {path}: {error}"))?;

            let session: AuthorSession = serde_json::from_str(&text)
                .map_err(|error| format!("parsing {path}: {error}"))?;

            let tries = flag("tries").map(|text| text.parse().unwrap_or(3)).unwrap_or(3);

            draft(&session, tries).map_err(|error| format!("{error:#}"))
        }

        "generate" => {
            let path = flag("session").ok_or("usage: skill new generate --session=<session.json> [--force]")?;

            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("reading {path}: {error}"))?;

            let session: AuthorSession = serde_json::from_str(&text)
                .map_err(|error| format!("parsing {path}: {error}"))?;

            let gaps = mapping_gaps(&session);

            if !gaps.is_empty() {
                return Err(format!(
                    "mapping incomplete ({}): run `skill new prepare --session={path}` first",
                    gaps.join(", "),
                ));
            }

            if session.split.train.is_empty() {
                return Err(format!(
                    "no split recorded: run `skill new split --session={path}` first"
                ));
            }

            let dir = std::path::Path::new("packs").join(&session.name);

            std::fs::create_dir_all(&dir)
                .map_err(|error| format!("creating {}: {error}", dir.display()))?;

            let out = dir.join("pack.rs");

            if out.exists() && !args.iter().any(|arg| arg == "--force") {
                return Err(format!("{} exists — pass --force to overwrite", out.display()));
            }

            std::fs::write(&out, generate_pack(&session))
                .map_err(|error| format!("writing {}: {error}", out.display()))?;

            println!("skill: generated {}", out.display());
            println!("skill: check it:  skill new validate --pack={}", out.display());
            println!("skill: then fit it: skill new smoke --pack={}", out.display());

            Ok(())
        }

        "validate" => {
            let path = flag("pack").ok_or("usage: skill new validate --pack=<pack.rs>")?;

            validate(&path).map_err(|error| format!("{error:#}"))
        }

        "smoke" => {
            let path = flag("pack").ok_or("usage: skill new smoke --pack=<pack.rs> [evolve flags]")?;

            let handle = crate::pack_loader::load_pack(std::path::Path::new(&path))
                .map_err(|error| format!("{error:#}"))?;

            // Pass the loop only harness flags; the pack is already
            // loaded. Defaults: one seed generation, no LLM, and a
            // throwaway workspace beside the real one.
            let mut loop_args: Vec<String> = args
                .iter()
                .filter(|arg| {
                    ["--subject=", "--subjects=", "--holdout-subject=", "--nfev=", "--subjects-limit="]
                        .iter()
                        .any(|prefix| arg.starts_with(prefix))
                        || *arg == "--no-evolve"
                })
                .cloned()
                .collect();

            loop_args.extend(
                ["--gens=1".to_string(), "--no-evolve".to_string()],
            );

            if !loop_args.iter().any(|arg| arg.starts_with("--workspace=")) {
                loop_args.push(format!(
                    "--workspace={}",
                    crate::workspace_root()
                        .join("workspace")
                        .join(format!("{}-smoke", handle.pack.name()))
                        .display(),
                ));
            }

            crate::driver::run(&handle, &loop_args)
        }

        "prepare" => {
            let path = flag("session").ok_or("usage: skill new prepare --session=<session.json>")?;

            crate::prepare::prepare(&path).map_err(|error| format!("{error:#}"))
        }

        "split" => {
            let path = flag("session").ok_or("usage: skill new split --session=<session.json>")?;

            let session = read_session(&path).map_err(|error| format!("{error:#}"))?;

            let split = split_stage(&session).map_err(|error| format!("{error:#}"))?;

            let session = AuthorSession { split, ..session };

            write_session(std::path::Path::new(&path), &session)
                .map_err(|error| format!("{error:#}"))?;

            println!("skill: split recorded in {path}");

            Ok(())
        }


        "manual" => interview().map_err(|error| format!("{error:#}")),

        other => Err(format!(
            "unknown `skill new` subcommand `{other}` — try: (blank = guided) | manual | \
             survey | preview | draft | generate | validate | smoke | prepare | split"
        )),
    }
}

fn interview() -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("`skill new` is interactive — run it in a terminal");
    }

    println!("skill: new problem — a few questions, then we look at your data together.");

    let name = loop {
        let candidate = ask("what should this problem be called? (one lowercase word)", None)?;

        if candidate
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            && !candidate.is_empty()
        {
            break candidate;
        }

        println!("skill: lowercase letters, digits, `-`, `_` only");
    };

    let problem_statement = ask("in one sentence: what should the discovered model explain?", None)?;

    let baseline_hint = ask_optional(
        "does a textbook / standard model already describe this process? \
         (words or equations; blank = let the system propose)",
        None,
    )?;
    let baseline_hint = (!baseline_hint.is_empty()).then_some(baseline_hint);

    let path = ask("where are your measurements? (a CSV/TSV file, or a folder of raw files)", None)?;

    let tabular = ask(
        "is it already ONE table with one row per measurement, whose columns directly name \
         the subject, the recorded series, time, the inputs you set, and the reported outputs? [y/n]",
        Some("y"),
    )?;

    let (mapping, prepare_brief, delimiter) = if tabular.starts_with('y') {
        let delimiter = ask("column delimiter? [,/tab]", Some(","))
            .map(|text| match text.as_str() {
                "tab" | "\\t" => '\t',
                other => other.chars().next().unwrap_or(','),
            })?;

        let survey = survey(&path, delimiter, SURVEY_ROWS)?;

        print!("{}", survey_text(&survey));

        let columns: Vec<String> = survey.columns.iter().map(|c| c.name.clone()).collect();

        let subject = ask_column(&columns, "which column is one experimental unit (one subject)?", None)?;

        let curve_raw = ask_optional(
            "which column(s) mark one recorded time series within a subject? (comma-separated, blank = one curve per subject)",
            None,
        )?;
        let curve: Vec<String> = curve_raw
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty() && columns.iter().any(|c| c == name))
            .map(str::to_string)
            .collect();

        let time = ask_column(
            &columns,
            "which column is time?",
            columns.iter().find(|name| name.to_lowercase().contains("time")).map(String::as_str),
        )?;

        let drives = ask_channels(&columns, "controlled input (what YOU set on the instrument)", true)?;
        let outputs = ask_channels(&columns, "measured output (what the instrument REPORTED)", true)?;

        if outputs.is_empty() {
            bail!("at least one measured output column is needed");
        }

        let condition_column = if drives.iter().any(|drive| !drive.recorded) {
            ask_optional(
                "across what did you vary the experiment, so the model must be TESTED on values it never saw? (column, blank = the set-level input column)",
                None,
            )?
        } else {
            ask_optional(
                "which column labels the experimental condition to hold out? (blank = first recorded input)",
                None,
            )?
        };

        let condition_column = (!condition_column.is_empty()).then_some(condition_column);

        let mapping = ColumnMap {
            subject: Some(subject),
            curve,
            time: Some(time),
            drives,
            outputs,
            condition_column,
        };

        (mapping, None, delimiter)
    } else {
        println!(
            "skill: then a preparation agent will turn your files into the one table the \
             loop reads. It works autonomously; you approve the picture at the end."
        );

        let brief = ask(
            "describe your data in your own words: files and formats, and where the time \
             series, the inputs you set, and the reported outputs live",
            None,
        )?;

        let drives = ask_channels(&[], "controlled input (what YOU set on the instrument)", false)?;
        let outputs = ask_channels(&[], "measured output (what the instrument REPORTED)", false)?;

        if outputs.is_empty() {
            bail!("at least one measured output is needed");
        }

        let mapping = ColumnMap {
            subject: None,
            curve: vec![],
            time: None,
            drives,
            outputs,
            condition_column: None,
        };

        (mapping, Some(brief), ',')
    };

    let session = AuthorSession {
        name: name.clone(),
        problem_statement,
        baseline_hint,
        prepare_brief,
        data: DataSpec { path, delimiter },
        mapping,
        split: SplitDraft::default(),
    };

    let dir = std::path::Path::new("packs").join(&name);

    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let file = dir.join("session.json");

    if session.prepare_brief.is_some() {
        std::fs::write(&file, serde_json::to_string_pretty(&session)?)
            .with_context(|| format!("writing {}", file.display()))?;

        println!("skill: recorded in {} — the preparation agent takes it from here:", file.display());
        println!("skill:   skill new prepare --session={}", file.display());
        println!("skill:   skill new split   --session={}   (after prepare commits)", file.display());

        return Ok(());
    }

    let split = split_stage(&session)?;

    let session = AuthorSession { split, ..session };

    std::fs::write(&file, serde_json::to_string_pretty(&session)?)
        .with_context(|| format!("writing {}", file.display()))?;

    println!("skill: interview recorded in {}", file.display());
    println!("skill: check the curves visually:  skill new preview --session={}", file.display());
    println!("skill: next stages (baseline mechanism, domain text, pack generation) build on this file.");

    Ok(())
}

/// The firewall stage: load the mapped data, list the conditions,
/// propose roles, and let the human confirm or edit. Shared by the
/// interview (fast lane) and `skill new split` (after prepare).
fn split_stage(session: &AuthorSession) -> Result<SplitDraft> {
    let subjects = load_subjects(session)?;

    if subjects.is_empty() {
        bail!("the mapping produced no subjects — check the columns");
    }

    let curves: usize = subjects.iter().map(|subject| subject.curves.len()).sum();
    println!(
        "skill: {} subjects, {} curves loaded",
        subjects.len(),
        curves,
    );

    let conditions = conditions_of(&subjects);

    println!(
        "skill: conditions found: {}",
        conditions
            .iter()
            .map(Condition::display)
            .collect::<Vec<_>>()
            .join(", ")
    );

    if conditions.len() < 3 {
        println!(
            "skill: WARNING — fewer than 3 conditions: the discovery loop needs at \
             least one train, one validation, and one hidden condition."
        );
    }

    let proposed = propose_split(&conditions);

    let split_text = format!(
        "train={} | validation={} | hidden={}",
        proposed.train.join(","),
        proposed.validation.join(","),
        proposed.hidden.join(","),
    );

    println!("skill: proposed split: {split_text}");

    loop {
        let answer = ask("accept this split? [y] or edit as `train=.. | validation=.. | hidden=..`", Some("y"))?;

        let candidate = if answer.eq_ignore_ascii_case("y") {
            proposed.clone()
        } else {
            parse_split(&answer)?
        };

        let known: BTreeSet<String> = conditions.iter().map(Condition::display).collect();

        let unknown: Vec<&String> = candidate
            .train
            .iter()
            .chain(&candidate.validation)
            .chain(&candidate.hidden)
            .filter(|value| !known.contains(*value))
            .collect();

        if !unknown.is_empty() {
            let unknown: Vec<&str> = unknown.iter().map(|value| value.as_str()).collect();
            println!("skill: unknown conditions: {}", unknown.join(", "));
            continue;
        }

        if candidate.train.is_empty() || candidate.validation.is_empty() || candidate.hidden.is_empty() {
            println!("skill: every role (train/validation/hidden) needs at least one condition");
            continue;
        }

        return Ok(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str, text: &str) -> String {
        let path = std::env::temp_dir().join(format!("skill-author-{}-{name}", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    }

    fn toy_csv(who: &str) -> String {
        fixture(
            &format!("toy-{who}.csv"),
            "binder,run,minutes,dose,level,report\n\
             A,1,0,10,low,0.1\n\
             A,1,1,10,low,0.4\n\
             A,2,0,100,mid,0.2\n\
             A,2,1,100,mid,0.9\n\
             B,1,0,10,low,0.15\n\
             B,1,1,10,low,0.5\n\
             B,2,0,1000,high,0.3\n\
             B,2,1,1000,high,1.2\n",
        )
    }

    fn toy_session(path: String) -> AuthorSession {
        AuthorSession {
            name: "toy".to_string(),
            problem_statement: "explain the toy assay".to_string(),
            baseline_hint: None,
            prepare_brief: None,
            data: DataSpec { path, delimiter: ',' },
            mapping: ColumnMap {
                subject: Some("binder".to_string()),
                curve: vec!["run".to_string()],
                time: Some("minutes".to_string()),
                drives: vec![ChannelColumn {
                    column: Some("dose".to_string()),
                    name: "dose".to_string(),
                    units: "nM".to_string(),
                    description: "set dose".to_string(),
                    recorded: false,
                }],
                outputs: vec![ChannelColumn {
                    column: Some("report".to_string()),
                    name: "report".to_string(),
                    units: "mV".to_string(),
                    description: "instrument report".to_string(),
                    recorded: true,
                }],
                condition_column: None,
            },
            split: SplitDraft::default(),
        }
    }

    #[test]
    fn survey_reports_kinds_ranges_and_samples() {
        let survey = survey(&toy_csv("survey"), ',', SURVEY_ROWS).unwrap();

        assert_eq!(survey.rows, 8);

        let binder = &survey.columns[0];
        assert!(!binder.numeric);
        assert_eq!(binder.distinct, 2);

        let dose = survey.columns.iter().find(|c| c.name == "dose").unwrap();
        assert!(dose.numeric);
        assert_eq!((dose.min, dose.max), (Some(10.0), Some(1000.0)));
    }

    #[test]
    fn loading_groups_curves_and_holds_constant_drives() {
        let session = toy_session(toy_csv("load"));

        let subjects = load_subjects(&session).unwrap();

        assert_eq!(subjects.len(), 2);
        assert_eq!(subjects[0].curves.len(), 2);

        let curve = &subjects[1].curves[1];
        assert_eq!(curve.condition, Condition::Level(1000.0));
        assert_eq!(curve.output_channel.as_deref(), Some("report"));
        assert_eq!(curve.raw.y, vec![0.3, 1.2]);

        assert_eq!(curve.drives.len(), 1);
        assert_eq!(curve.drives[0].values, vec![1000.0]);
    }

    #[test]
    fn missing_condition_column_falls_back_to_the_drive_level() {
        let session = toy_session(toy_csv("fallback"));

        let conditions = conditions_of(&load_subjects(&session).unwrap());

        assert_eq!(
            conditions,
            vec![
                Condition::Level(10.0),
                Condition::Level(100.0),
                Condition::Level(1000.0),
            ],
        );
    }

#[test]
fn extract_json_prefers_fences_and_survives_braces_in_strings() {
    let fenced = "Sure!\n```json\n{\"name\": \"m\"}\n```\ndone";
    assert_eq!(extract_json(fenced), Some("{\"name\": \"m\"}"));

    let bare = "Here: {\"name\": \"a { b }\", \"x\": [1, 2]} tail";
    assert_eq!(
        extract_json(bare),
        Some("{\"name\": \"a { b }\", \"x\": [1, 2]}"),
    );

    assert_eq!(extract_json("no json here"), None);
}

#[test]
fn digest_survives_a_real_load_and_stays_compact() {
    let session = toy_session(toy_csv("digest"));

    let digest = data_digest(&session, &load_subjects(&session).unwrap());

    assert!(digest.contains("DRIVE `dose` (nM, constant per curve)"));
    assert!(digest.contains("OUTPUT `report` (mV, recorded)"));
    assert!(digest.contains("condition 1000"));
    assert!(digest.lines().count() < 30);
}

#[test]
fn proposed_split_keeps_every_role_nonempty() {
        for k in 3..=9 {
            let conditions: Vec<Condition> =
                (0..k).map(|index| Condition::Level(index as f64)).collect();

            let split = propose_split(&conditions);

            assert!(!split.train.is_empty() && !split.validation.is_empty());
            assert!(!split.hidden.is_empty(), "k={k} hidden empty");

            let total = split.train.len() + split.validation.len() + split.hidden.len();
            assert_eq!(total, k as usize);
        }
    }

    #[test]
    fn split_edits_round_trip_and_reject_unknown_roles() {
        let split = parse_split("train=0,1 | validation=2 | hidden=3,4").unwrap();

        assert_eq!(split.train, vec!["0", "1"]);
        assert_eq!(split.hidden, vec!["3", "4"]);

        assert!(parse_split("test=1").is_err());
    }
}
