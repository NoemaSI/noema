//! `skill new prepare`: turn the scientist's raw files into the one
//! canonical curve table the loop reads.
//!
//! The interview records what the human knows in their own words
//! (`session.json`, possibly with an incomplete mapping and a
//! `prepare_brief`); this stage hands that to a tool-using agent
//! that explores the raw data, writes a converter, and commits a
//! complete mapping. The canonical table is the contract: everything
//! downstream ([`crate::author::load_subjects`], preview, generate,
//! validate, smoke, the generated pack) never learns whether the
//! scientist's file was already tabular or was built by `prepare.py`.
//!
//! Provenance rule: the agent must leave behind a reproducible
//! recipe. Either the mapping points straight at the original file
//! (already tabular), or it points at `data/canonical.csv` that
//! `prepare.py` regenerates from the untouched originals. The agent
//! never modifies the raw data (read sandbox includes it, write
//! sandbox does not).
//!
//! # The run tool
//!
//! The agent's central capability — executing converter code — runs
//! through [`crate::jail`]: every command executes under
//! [`crate::jail::JailPolicy::prepare`] (pack directory `--rw-map`ed,
//! raw data `--map`ped read-only, network and agent state explicitly
//! off), with `DATA_DIR` exported into the command itself. The tool
//! returns combined stdout+stderr (truncated) and the exit code; a
//! timeout kills the command and says so.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rig_agent::tool::DynamicTool;
use serde::Deserialize;
use serde_json::{Value, json};
use serdes_ai::tools::{SchemaBuilder, SyncFunctionTool, ToolReturn};

use crate::agent::{self, LlmConfig, rig_tool};
use crate::author::{self, AuthorSession, ChannelColumn, ColumnMap};
use crate::jail::{JailPolicy, JailedCommand};

/// Cap on survey output rows returned to the agent.
const TOOL_SURVEY_ROWS: usize = 20_000;

// ---------------------------------------------------------------- tools

fn schema(builder: SchemaBuilder) -> Value {
    builder.build().expect("tool schema builds")
}

/// `survey {path, delimiter?}` — the deterministic column picture of
/// any tabular file.
fn survey_tool() -> DynamicTool {
    rig_tool::<_, ()>(SyncFunctionTool::new(
        "survey",
        "Report columns of a tabular file: type, distinct count, range, samples. \
         Use it before proposing a mapping; it never fails on messy numbers.",
        schema(
            SchemaBuilder::new()
                .string("path", "file path (inside the visible directories)", true)
                .string("delimiter", "column delimiter, default ','", false),
        ),
        |_ctx, args| {
            let path = args["path"].as_str().unwrap_or_default().to_string();
            let delimiter = args["delimiter"].as_str().unwrap_or(",").chars().next().unwrap_or(',');

            match author::survey(&path, delimiter, TOOL_SURVEY_ROWS) {
                Ok(survey) => Ok(ToolReturn::text(author::survey_text(&survey))),
                Err(error) => Ok(ToolReturn::error(format!("{error:#}"))),
            }
        },
    )
)
}

#[derive(Deserialize)]
struct ChannelArg {
    column: String,
    name: String,
    #[serde(default)]
    recorded: bool,
    /// The agent's proposal for what the channel means — the
    /// scientist can fix it at the gate without an LLM.
    #[serde(default)]
    units: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
struct SplitArg {
    #[serde(default)]
    train: Vec<String>,
    #[serde(default)]
    validation: Vec<String>,
    #[serde(default)]
    hidden: Vec<String>,
}

#[derive(Deserialize)]
struct ValidateArgs {
    csv: String,
    #[serde(default)]
    delimiter: Option<String>,
    subject: String,
    #[serde(default)]
    curve: Vec<String>,
    time: String,
    #[serde(default)]
    drives: Vec<ChannelArg>,
    outputs: Vec<ChannelArg>,
    #[serde(default)]
    condition_column: Option<String>,
    /// The proposed firewall over the conditions the earlier
    /// validate_table call reported.
    #[serde(default)]
    split: Option<SplitArg>,
    #[serde(default)]
    commit: bool,
}

/// Keep the scientist's channel vocabulary: units and descriptions
/// already in the session survive a re-mapping by channel name;
/// otherwise the agent's proposal fills them in.
fn merged_channel(
    previous: &[ChannelColumn],
    arg: &ChannelArg,
    recorded: bool,
) -> ChannelColumn {
    let known = previous.iter().find(|channel| channel.name == arg.name);

    ChannelColumn {
        column: Some(arg.column.clone()),
        name: arg.name.clone(),
        units: known
            .map(|c| c.units.clone())
            .or_else(|| arg.units.clone())
            .unwrap_or_else(|| arg.name.clone()),
        description: known
            .map(|c| c.description.clone())
            .or_else(|| arg.description.clone())
            .unwrap_or_else(|| arg.name.clone()),
        recorded,
    }
}

/// `validate_table {...mapping..., commit?}` — try a candidate
/// mapping against the real loader; with `commit: true` a successful
/// mapping is written into `session.json` (the agent's artifact).
fn validate_table_tool(session_path: PathBuf) -> DynamicTool {
    rig_tool::<_, ()>(SyncFunctionTool::new(
        "validate_table",
        "Try a column mapping by actually loading the data. Returns subject/curve/condition \
         counts or a precise error — the conditions it reports are exactly the strings the \
         split must use. Set commit=true once satisfied, including your split proposal: the \
         mapping and split are then written into session.json and become the pack's \
         ingestion. Call it as many times as needed; only the final commit matters.",
        schema(
            SchemaBuilder::new()
                .string("csv", "path of the tabular file the mapping reads", true)
                .string("delimiter", "column delimiter, default ','", false)
                .string("subject", "column naming one experimental unit", true)
                .string_array("curve", "columns identifying one recorded series within a subject", false)
                .string("time", "time column", true)
                .raw(
                    "drives",
                    json!({
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "column": { "type": "string" },
                                "name": { "type": "string" },
                                "recorded": { "type": "boolean" },
                                "units": { "type": "string" },
                                "description": { "type": "string" }
                            },
                            "required": ["column", "name"]
                        }
                    }),
                    false,
                )
                .raw(
                    "outputs",
                    json!({
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "column": { "type": "string" },
                                "name": { "type": "string" },
                                "units": { "type": "string" },
                                "description": { "type": "string" }
                            },
                            "required": ["column", "name"]
                        }
                    }),
                    true,
                )
                .string("condition_column", "column labeling the held-out condition (omit to use the set-level drive)", false)
                .raw(
                    "split",
                    json!({
                        "type": "object",
                        "properties": {
                            "train": { "type": "array", "items": { "type": "string" } },
                            "validation": { "type": "array", "items": { "type": "string" } },
                            "hidden": { "type": "array", "items": { "type": "string" } }
                        }
                    }),
                    false,
                )
                .boolean("commit", "write the successful mapping and split into session.json", false),
        ),
        move |_ctx, args| {
            let parsed: ValidateArgs = match serde_json::from_value(args) {
                Ok(parsed) => parsed,
                Err(error) => return Ok(ToolReturn::error(format!("bad arguments: {error}"))),
            };

            match try_validate(&session_path, &parsed) {
                Ok(summary) => Ok(ToolReturn::json(summary)),
                Err(message) => Ok(ToolReturn::error(message)),
            }
        },
    )
)
}

fn try_validate(session_path: &Path, args: &ValidateArgs) -> Result<Value, String> {
    let mut session = author::read_session(&session_path.display().to_string())
        .map_err(|error| format!("{error:#}"))?;

    // The committed path must outlive the agent's cwd: the pack
    // loads later from wherever `evolve` runs, so a relative csv
    // would silently point at a different file.
    let csv = std::fs::canonicalize(&args.csv)
        .map(|path| path.display().to_string())
        .map_err(|error| format!("cannot read `{}`: {error}", args.csv))?;

    session.data = author::DataSpec {
        path: csv,
        delimiter: args
            .delimiter
            .as_deref()
            .and_then(|text| text.chars().next())
            .unwrap_or(','),
    };

    session.mapping = ColumnMap {
        subject: Some(args.subject.clone()),
        curve: args.curve.clone(),
        time: Some(args.time.clone()),
        drives: args
            .drives
            .iter()
            .map(|drive| merged_channel(&session.mapping.drives, drive, drive.recorded))
            .collect(),
        outputs: args
            .outputs
            .iter()
            .map(|output| merged_channel(&session.mapping.outputs, output, true))
            .collect(),
        condition_column: args.condition_column.clone(),
    };

    let subjects = author::load_subjects(&session).map_err(|error| format!("{error:#}"))?;

    if subjects.is_empty() {
        return Err("the mapping loaded zero subjects — the grouping columns are wrong".to_string());
    }

    let conditions: Vec<Value> = author::conditions_of(&subjects)
        .iter()
        .map(|condition| {
            let curves = subjects
                .iter()
                .flat_map(|subject| &subject.curves)
                .filter(|curve| curve.condition.matches(condition))
                .count();

            json!({ "condition": condition.display(), "curves": curves })
        })
        .collect();

    let mut summary = json!({
        "ok": true,
        "subjects": subjects.len(),
        "curves": subjects.iter().map(|subject| subject.curves.len()).sum::<usize>(),
        "conditions": conditions,
    });

    // A proposed split is checked against the loaded conditions with
    // the harness's own matching — the same oracle the gate and the
    // pack will use, so an agent cannot commit labels that would
    // silently match nothing.
    if let Some(split) = &args.split {
        let known = author::conditions_of(&subjects);

        let unknown: Vec<&String> = split
            .train
            .iter()
            .chain(&split.validation)
            .chain(&split.hidden)
            .filter(|value| {
                !known
                    .iter()
                    .any(|condition| &condition.display() == value.as_str())
            })
            .collect();

        if !unknown.is_empty() {
            return Err(format!(
                "split names conditions that do not exist: {} — use exactly the \
                 condition strings reported by this tool",
                unknown
                    .iter()
                    .map(|value| value.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }

        if split.train.is_empty() || split.validation.is_empty() || split.hidden.is_empty() {
            return Err("every role (train/validation/hidden) needs at least one condition".to_string());
        }

        session.split = author::SplitDraft {
            train: split.train.clone(),
            validation: split.validation.clone(),
            hidden: split.hidden.clone(),
        };
    }

    if args.commit {
        if session.split.train.is_empty() {
            return Err(
                "commit needs a split proposal (train/validation/hidden over the reported \
                 conditions)"
                    .to_string(),
            );
        }

        author::write_session(session_path, &session).map_err(|error| format!("{error:#}"))?;

        summary["committed"] = json!(true);
    }

    Ok(summary)
}

/// `preview {panels?}` — render the loaded data to `preview.png` in
/// the pack directory for the human's approval gate.
fn preview_tool(session_path: PathBuf) -> DynamicTool {
    rig_tool::<_, ()>(SyncFunctionTool::new(
        "preview",
        "Render the currently committed mapping's measured curves to preview.png (one panel \
         per subject). Call it after a successful commit and report the path to the human; \
         the run only ends with a preview the human can check.",
        schema(SchemaBuilder::new().integer("panels", "subjects per figure, default 3", false)),
        move |_ctx, args| {
            let panels = args["panels"].as_u64().unwrap_or(3) as usize;

            let outcome = (|| -> Result<String> {
                let session = author::read_session(&session_path.display().to_string())?;
                let subjects = author::load_subjects(&session)?;
                let png = author::preview_png(&session, &subjects, panels)?;

                let out = session_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("preview.png");

                std::fs::write(&out, png)?;

                Ok(out.display().to_string())
            })();

            match outcome {
                Ok(path) => Ok(ToolReturn::text(format!(
                    "wrote {path} ({panels} panels) — show it to the scientist for approval"
                ))),
                Err(error) => Ok(ToolReturn::error(format!("{error:#}"))),
            }
        },
    )
)
}

/// The execution tool: jailed shell commands for building and
/// running the converter, under [`JailPolicy::prepare`] — pack
/// directory writable, raw data readable, nothing else.
pub fn run_tool(pack_dir: PathBuf, data_dir: PathBuf) -> DynamicTool {
    rig_tool::<_, ()>(SyncFunctionTool::new(
        "run",
        "Execute a shell command in a jail (no network, minimal environment, writes \
         confined to the pack directory). Default cwd is the pack directory; DATA_DIR \
         is exported and must be treated read-only. `pixi run python` is available. \
         Returns combined stdout+stderr (truncated) and the exit code.",
        schema(
            SchemaBuilder::new()
                .string("command", "shell command to execute", true)
                .string("cwd", "working directory inside the pack directory, default the pack directory", false)
                .integer("timeout_secs", "kill the command after this many seconds (default 120, max 600)", false),
        ),
        move |_ctx, args| {
            let command = args["command"].as_str().unwrap_or_default();

            if command.is_empty() {
                return Ok(ToolReturn::error("`command` must not be empty"));
            }

            // Policy enforcement, not suggestion: a cwd outside the
            // pack directory would be a directory the jail does not
            // mount writable, so refuse it by name instead of
            // letting the jail fail opaquely.
            let cwd = match args["cwd"].as_str() {
                Some(text) => {
                    let cwd = PathBuf::from(text);

                    if !cwd.starts_with(&pack_dir) {
                        return Ok(ToolReturn::error(format!(
                            "cwd `{}` is outside the pack directory ({})",
                            cwd.display(),
                            pack_dir.display(),
                        )));
                    }

                    cwd
                }
                None => pack_dir.clone(),
            };

            let timeout = std::time::Duration::from_secs(
                args["timeout_secs"].as_u64().unwrap_or(120).clamp(1, 600),
            );

            let outcome = JailedCommand::new("sh", &cwd, JailPolicy::prepare(&pack_dir, &data_dir))
                .arg("-c")
                .env("DATA_DIR", &data_dir.display().to_string())
                .run_shell(command, timeout);

            match outcome {
                Ok(out) => Ok(ToolReturn::text(format!(
                    "exit {:?}{}{}",
                    out.exit,
                    if out.timed_out { " (timed out — killed)" } else { "" },
                    if out.text.is_empty() { "" } else { "\n" },
                ) + &out.text)),
                Err(error) => Ok(ToolReturn::error(format!("{error:#}"))),
            }
        },
    ))
}

// ---------------------------------------------------------------- stage

/// The prepare agent's job contract: goal, artifacts, honesty rules.
/// Two modes share one preamble: a session with channels already
/// declared (the interview's needs-preparation branch) must cover
/// them; an empty session (the `skill new` proposal flow) must
/// invent the whole model of the data — channels, units, meanings,
/// split — from what it finds.
fn preamble(session: &AuthorSession, raw: &Path) -> String {
    let declared = !session.mapping.outputs.is_empty();

    let channels = if declared {
        let mut text = String::from(
            "CHANNELS THE SCIENTIST DECLARED (your mapping must cover every one):",
        );

        for channel in &session.mapping.drives {
            text.push_str(&format!(
                "\n- DRIVE `{}` ({}, {}): {}",
                channel.name,
                channel.units,
                if channel.recorded { "recorded per row" } else { "constant per curve" },
                channel.description,
            ));
        }

        for channel in &session.mapping.outputs {
            text.push_str(&format!(
                "\n- OUTPUT `{}` ({}, recorded per row): {}",
                channel.name, channel.units, channel.description,
            ));
        }

        text
    } else {
        "CHANNELS: none declared — propose them. Infer which recorded quantity is \
         the thing the experimenter SET (the drive: constant per curve or recorded \
         per row) and which is what the instrument REPORTED (the output). Give every \
         channel a short lowercase name, plausible units inferred from the values' \
         range and shape, and a one-sentence description of what the number \
         physically means."
            .to_string()
    };

    let goal = if declared {
        "GOAL: make `session.json` describe a mapping that loads the scientist's \
         reality: one subject id per experimental unit, one curve per recorded time \
         series, one time column, every declared drive and output channel mapped to \
         a column."
    } else {
        "GOAL: propose the COMPLETE reading of the data — mapping (subject, series, \
         time, channels, condition axis), channel names/units/meanings, and the \
         train/validation/hidden split over the condition levels — and commit it. \
         The split is the generalization firewall: lowest levels to train, middle \
         to validation, highest held out; every role non-empty; use exactly the \
         condition strings validate_table reports."
    };

    format!(
        r#"You are the data-preparation agent of a scientific mechanism-discovery harness.

THE PROBLEM: {problem}

THE SCIENTIST DESCRIBED THEIR DATA AS:
{brief}

{channels}
RAW DATA: {raw} — read it freely, NEVER modify it. Your working directory is the
pack directory; session.json there records the current mapping.

{goal}

You have: file tools; `survey` for tabular files; `validate_table` (call it
often — it loads the real data and returns exact counts, conditions, and
errors; commit=true with your split records mapping + split); `preview`
(renders the committed data for the human); `run` (execute converter code).

ACCEPTABLE OUTCOMES, in order of preference:
1. The data is already a suitable table — commit a direct mapping. No converter.
2. It needs shaping — write `prepare.py` in the pack directory, run it to
   produce `data/canonical.csv` (a flat table: one row per measurement with
   columns for subject, series id, time, each channel, and condition), then
   commit the mapping pointing at it. `prepare.py` must regenerate the output
   from the untouched originals by itself (`pixi run python prepare.py`); the
   harness will re-run it later without you.

HONESTY RULES: never commit a mapping you have not seen validate_table accept;
never invent columns; if the data cannot express what the problem needs (e.g. no
time axis, no controlled input), stop and say exactly what is missing — a wrong
table is worse than none. Finish by calling preview and stating the preview.png
path."#,
        problem = session.problem_statement,
        brief = session.prepare_brief.as_deref().unwrap_or("(none given — explore and ask nothing)"),
        channels = channels,
        goal = goal,
        raw = raw.display(),
    )
}

/// Run the preparation agent for one session until it commits (or
/// reports honestly why it cannot). `note` is appended to the
/// scientist's description when the human corrects the proposal at
/// the gate and asks for a redo.
pub fn run_proposal(
    session_arg: &str,
    note: Option<&str>,
    maybe_llm_config: impl Into<Option<LlmConfig>>, 
) -> Result<()> {
    let session_path = PathBuf::from(session_arg);

    let mut session = author::read_session(session_arg)?;

    if let Some(note) = note {
        session.prepare_brief = Some(match &session.prepare_brief {
            Some(brief) => format!("{brief}\n\nTHE HUMAN'S CORRECTION (previous attempt was wrong — this takes precedence):\n{note}"),
            None => note.to_string(),
        });

        author::write_session(&session_path, &session)?;
    }

    let pack_dir = session_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    std::fs::create_dir_all(&pack_dir)?;

    // Absolute, canonical mounts: the jail maps and the cwd
    // containment check both compare paths the agent may also write
    // as absolute, so relative spellings must not create two names
    // for one directory.
    let pack_dir = std::fs::canonicalize(&pack_dir).unwrap_or(pack_dir);

    std::fs::create_dir_all(pack_dir.join("data"))?;

    let raw = PathBuf::from(&session.data.path);

    let raw_dir = if raw.is_dir() {
        raw.clone()
    } else {
        raw.parent().map(Path::to_path_buf).unwrap_or_default()
    };

    let raw_dir = std::fs::canonicalize(&raw_dir).unwrap_or(raw_dir);

    let config = maybe_llm_config.into().map(Ok).unwrap_or_else(LlmConfig::from_env)?;

    #[cfg(feature = "viewer")]
    crate::viewer::serve_local();

    let (mut tools, prompt_tail) = agent::file_tools(&pack_dir, &[raw_dir.clone()])?;

    tools.push(survey_tool());
    tools.push(validate_table_tool(session_path.clone()));
    tools.push(preview_tool(session_path.clone()));
    tools.push(run_tool(pack_dir.clone(), raw_dir.clone()));

    let system_prompt = format!("{}\n\n{prompt_tail}", preamble(&session, &raw_dir));

    let agent = agent::reasoning_builder(&system_prompt, &config)?
        .dynamic_tools(tools)
        .default_max_turns(60)
        .build();

    agent::run(
        &agent,
        "Begin. Explore the raw data, build the canonical table if needed, and finish \
         by committing a validated mapping and split with validate_table, then render \
         preview.png and stop.",
    )?;

    let session = author::read_session(session_arg)?;

    let gaps = author::mapping_gaps(&session);

    if gaps.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "the agent did not commit a complete mapping (missing: {}); review its \
             report and retry",
            gaps.join(", "),
        )
    }
}

/// Legacy entry: `skill new prepare --session=…` for sessions the
/// interview already filled with declared channels.
pub fn prepare(session_arg: &str) -> Result<()> {
    run_proposal(session_arg, None, None)?;

    println!("skill: mapping committed; check packs/…/preview.png");

    Ok(())
}

// ---------------------------------------------------------------- v2 flow

/// Derive the pack slug from the data location: `bactgrowth.txt` →
/// `bactgrowth`, a folder → its name.
fn slug_for(path: &str) -> String {
    let name = Path::new(path.trim_end_matches('/'))
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "pack".to_string());

    let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&name);

    let slug: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();

    let slug = slug.trim_matches('_').to_string();

    if slug.is_empty() {
        "pack".to_string()
    } else {
        slug
    }
}

/// The gate's reality report: everything the human needs to approve
/// or correct, rendered from the session by actually loading the
/// data (a broken mapping fails here, not silently).
fn summary_text(session_path: &Path) -> Result<String> {
    use crate::experiment::Condition;

    let session = author::read_session(&session_path.display().to_string())?;

    let subjects = author::load_subjects(&session)?;

    let curves: usize = subjects.iter().map(|s| s.curves.len()).sum();

    let mut text = format!(
        "  subjects: {} ({}) | curves: {}\n",
        subjects.len(),
        subjects
            .iter()
            .take(5)
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        curves,
    );

    for channel in &session.mapping.drives {
        text.push_str(&format!(
            "  input  {} [{}] {} — {}\n",
            channel.name,
            channel.column.as_deref().unwrap_or("?"),
            channel.units,
            if channel.recorded { "recorded per row" } else { "set per curve" },
        ));
    }

    for channel in &session.mapping.outputs {
        text.push_str(&format!(
            "  output {} [{}] {}\n",
            channel.name,
            channel.column.as_deref().unwrap_or("?"),
            channel.units,
        ));
    }

    let conditions = author::conditions_of(&subjects);

    text.push_str(&format!(
        "  conditions: {}\n",
        conditions.iter().map(Condition::display).collect::<Vec<_>>().join(", ")
    ));

    for (role, values) in [
        ("train", &session.split.train),
        ("validation", &session.split.validation),
        ("hidden", &session.split.hidden),
    ] {
        let count = subjects
            .iter()
            .flat_map(|s| &s.curves)
            .filter(|curve| {
                values.iter().any(|value| {
                    conditions
                        .iter()
                        .find(|condition| &condition.display() == value)
                        .is_some_and(|condition| curve.condition.matches(condition))
                })
            })
            .count();

        text.push_str(&format!("  {role:>10}: [{}] ({count} curves)\n", values.join(", ")));
    }

    Ok(text)
}

/// One fixed-menu correction. Applied to a clone, validated by
/// actually loading the data, and only then written — a bad edit
/// reports its error and changes nothing.
fn apply_edit(session_path: &Path, edit: &str) -> Result<String> {
    let mut session = author::read_session(&session_path.display().to_string())?;

    let mut words = edit.split_whitespace();

    let command = words.next().unwrap_or("");
    let rest: Vec<&str> = words.collect();

    let names: Vec<String> = session
        .mapping
        .drives
        .iter()
        .chain(session.mapping.outputs.iter())
        .map(|c| c.name.clone())
        .collect();

    let ensure = |name: &str| -> Result<()> {
        if names.iter().any(|n| n == name) {
            Ok(())
        } else {
            anyhow::bail!(
                "no channel `{name}` — inputs/outputs are listed in the summary"
            )
        }
    };

    match (command, rest.as_slice()) {
        ("units", [name, values @ ..]) => {
            ensure(name)?;

            let units = values.join(" ");

            for c in session.mapping.drives.iter_mut().chain(session.mapping.outputs.iter_mut()) {
                if c.name == *name {
                    c.units = units.clone();
                }
            }
        }
        ("meaning", [name, values @ ..]) => {
            ensure(name)?;

            let description = values.join(" ");

            for c in session.mapping.drives.iter_mut().chain(session.mapping.outputs.iter_mut()) {
                if c.name == *name {
                    c.description = description.clone();
                }
            }
        }
        ("map", [name, column]) => {
            ensure(name)?;

            for c in session.mapping.drives.iter_mut().chain(session.mapping.outputs.iter_mut()) {
                if c.name == *name {
                    c.column = Some(column.to_string());
                }
            }
        }
        ("subject", [column]) => session.mapping.subject = Some(column.to_string()),
        ("time", [column]) => session.mapping.time = Some(column.to_string()),
        ("problem", values) => session.problem_statement = values.join(" "),
        ("split", values) => {
            session.split = author::parse_split(&values.join(" "))?;
        }
        _ => anyhow::bail!(
            "unknown edit `{command}` — menu: units | meaning | map | subject | time | \
             problem | split"
        ),
    }

    // Try-commit: the same oracle the agent faced.
author::load_subjects(&session).map_err(|error| anyhow::anyhow!("{error:#}"))?;

    author::write_session(session_path, &session)?;

    Ok(format!("ok — {command} applied"))
}

/// `skill new` v2: two questions, one autonomous proposal, one
/// human gate with fixed-menu corrections.
pub fn new_flow() -> Result<()> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("`skill new` is interactive — run it in a terminal");
    }

    println!("skill: new problem — two questions, then the system proposes and you approve.");

    let path = author::ask("where are your measurements? (a file or folder)", None)?;
    let problem = author::ask(
        "in one sentence: what should the discovered model explain?",
        None,
    )?;
    let baseline_hint = author::ask_optional(
        "optional: does a textbook model already describe this process? (blank = not needed)",
        None,
    )?;

    let name = author::ask(
        "what should this pack be called? (one lowercase word)",
        Some(&slug_for(&path)),
    )?;

    let name = name.trim().to_ascii_lowercase().replace(' ', "_");

    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        anyhow::bail!("pack name `{name}` — use letters, digits, _ or -");
    }

    let dir = std::path::Path::new("packs").join(&name);

    std::fs::create_dir_all(&dir)?;

    let session_path = dir.join("session.json");

    if session_path.exists() {
        let answer = author::ask(
            &format!("pack `{name}` already exists — start over? [y/N]"),
            Some("n"),
        )?;

        if !answer.starts_with('y') {
            anyhow::bail!("keeping packs/{name}; remove it or choose a different data name");
        }
    }

    let session = AuthorSession {
        name: name.clone(),
        problem_statement: problem,
        baseline_hint: (!baseline_hint.is_empty()).then_some(baseline_hint),
        prepare_brief: Some(
            "The scientist supplied only the path and the problem. Explore the data \
             and propose the complete reading yourself.".to_string(),
        ),
        data: author::DataSpec { path, delimiter: ',' },
        mapping: ColumnMap {
            subject: None,
            curve: vec![],
            time: None,
            drives: vec![],
            outputs: vec![],
            condition_column: None,
        },
        split: author::SplitDraft::default(),
    };

    author::write_session(&session_path, &session)?;

    println!("skill: pack `{name}` started — the preparation agent takes it from here");
    println!("skill: (its work streams to the terminal; Ctrl-C is always safe —");
    println!("skill:  nothing is approved until you say so at the review below)");

    run_proposal(&session_path.display().to_string(), None, None)?;

    gate(&session_path)
}

/// The one human gate: reality report + preview, fixed-menu
/// corrections, `redo` to send the agent away with a note.
fn gate(session_path: &Path) -> Result<()> {
    loop {
        let session = author::read_session(&session_path.display().to_string())?;

        println!("skill: here is what I found — check it:");
        print!("{}", summary_text(session_path).unwrap_or_else(|error| format!("  (mapping does not load: {error})\n")));

        let preview = (|| -> Result<PathBuf> {
            let session = author::read_session(&session_path.display().to_string())?;
            let subjects = author::load_subjects(&session)?;
            let png = author::preview_png(&session, &subjects, 3)?;

            let out = session_path.parent().unwrap_or(Path::new(".")).join("preview.png");

            std::fs::write(&out, png)?;

            Ok(out)
        })();

        match &preview {
            Ok(path) => println!("skill: curves:  {}", path.display()),
            Err(_) => println!("skill: curves:  (preview unavailable — is pixi on PATH?)"),
        }

        println!(
            "skill:\n\
             skill:   [y] accept        [q] save & quit\n\
             skill:   units <channel> <..>        meaning <channel> <..>\n\
             skill:   map <channel> <column>      subject <column>   time <column>\n\
             skill:   problem <sentence>          split train=.. | validation=.. | hidden=..\n\
             skill:   redo <what was wrong>       (send the agent away with your note)"
        );

        let answer = author::ask("your call:", None)?;
        let trimmed = answer.trim();

        if trimmed.eq_ignore_ascii_case("y") {
            println!("skill: approved. next:");
            println!("skill:   skill new draft    --session={}", session_path.display());
            println!("skill:   skill new generate --session={}", session_path.display());
            println!("skill:   skill new validate --pack=packs/{}/pack.rs", session.name);
            println!("skill:   skill new smoke    --pack=packs/{}/pack.rs", session.name);

            return Ok(());
        }

        if trimmed.eq_ignore_ascii_case("q") {
            println!("skill: saved to {} — resume with the same command later", session_path.display());

            return Ok(());
        }

        if let Some(note) = trimmed.strip_prefix("redo").map(str::trim) {
            if let Err(error) = run_proposal(&session_path.display().to_string(), Some(note), None) {
                println!("skill: redo failed: {error:#}");
            }

            continue;
        }

        match apply_edit(session_path, trimmed) {
            Ok(message) => println!("skill: {message}"),
            Err(error) => println!("skill: not applied: {error:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


#[test]
fn validate_table_loads_and_commits_a_mapping() {
    let dir = std::env::temp_dir().join(format!("skill-prepare-{}", std::process::id()));

        std::fs::create_dir_all(&dir).unwrap();

        let csv = dir.join("toy.csv");

        std::fs::write(
            &csv,
            "unit,series,t,level,reading\n\
             U1,1,0,10,0.1\nU1,1,1,10,0.4\n\
             U2,1,0,1000,0.3\nU2,1,1,1000,1.2\n",
        )
        .unwrap();

        let session_path = dir.join("session.json");

        let incomplete = AuthorSession {
            name: "preptest".to_string(),
            problem_statement: "x".to_string(),
            baseline_hint: None,
            prepare_brief: Some("toy".to_string()),
            data: author::DataSpec { path: String::new(), delimiter: ',' },
            mapping: ColumnMap {
                subject: None,
                curve: vec![],
                time: None,
                drives: vec![ChannelColumn {
                    column: None,
                    name: "level".to_string(),
                    units: "nM".to_string(),
                    description: "the dose we set".to_string(),
                    recorded: false,
                }],
                outputs: vec![ChannelColumn {
                    column: None,
                    name: "reading".to_string(),
                    units: "mV".to_string(),
                    description: "reported".to_string(),
                    recorded: true,
                }],
                condition_column: None,
            },
            split: author::SplitDraft::default(),
        };

        author::write_session(&session_path, &incomplete).unwrap();

        let args = json!({
            "csv": csv.display().to_string(),
            "subject": "unit",
            "curve": ["series"],
            "time": "t",
            "drives": [{ "column": "level", "name": "level", "recorded": false,
                         "units": "µM", "description": "agent's guess" }],
            "outputs": [{ "column": "reading", "name": "reading" }],
            "split": { "train": ["10"], "validation": ["1000"], "hidden": ["10"] },
            "commit": true,
        });

        let summary = try_validate(&session_path, &serde_json::from_value(args).unwrap()).unwrap();

        assert_eq!(summary["subjects"], 2);
        assert_eq!(summary["committed"], true);

        let committed = author::read_session(&session_path.display().to_string()).unwrap();

        assert!(author::mapping_gaps(&committed).is_empty());

        // The scientist's channel vocabulary survived the re-mapping,
        // and the committed split is the agent's proposal.
        assert_eq!(committed.mapping.drives[0].units, "nM");
        assert_eq!(committed.mapping.drives[0].description, "the dose we set");
        assert_eq!(committed.split.train, ["10".to_string()]);
        assert_eq!(committed.split.hidden, ["10".to_string()]);

        // A split naming conditions that do not exist is refused —
        // the same oracle the gate and the pack will use.
        let bad = json!({
            "csv": csv.display().to_string(),
            "subject": "unit",
            "curve": ["series"],
            "time": "t",
            "drives": [{ "column": "level", "name": "level", "recorded": false }],
            "outputs": [{ "column": "reading", "name": "reading" }],
            "split": { "train": ["10"], "validation": ["999"], "hidden": ["10"] },
        });

        let error = try_validate(&session_path, &serde_json::from_value(bad).unwrap())
            .unwrap_err();

        assert!(error.contains("999"), "{error}");

        // The gate's fixed-menu parser: a units edit lands, a bogus
        // channel is refused without touching the file.
        apply_edit(&session_path, "units level nM free").unwrap();

        let after = author::read_session(&session_path.display().to_string()).unwrap();

        assert_eq!(after.mapping.drives[0].units, "nM free");

        assert!(apply_edit(&session_path, "units nope units").is_err());

        assert_eq!(
            author::read_session(&session_path.display().to_string())
                .unwrap()
                .mapping
                .drives[0]
                .units,
            "nM free",
        );
    }

    #[test]
    fn slugs_come_from_the_data_name() {
        assert_eq!(slug_for("/x/y/bactgrowth.txt"), "bactgrowth");
        assert_eq!(slug_for("Ecoli Data/"), "ecoli_data");
        assert_eq!(slug_for("///"), "pack");
    }
}
