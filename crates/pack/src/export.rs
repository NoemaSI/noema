//! `skill export` — a workspace's frozen parameters become DATA.
//!
//! Phase D meta-discovery (TEST_PLAN.md): pack A's champion report
//! freezes one theta per subject; those thetas are the measured
//! outputs of a second pack B that searches for `theta = f(covariate)`.
//! This command is the only sanctioned bridge: it reads a generation's
//! report (the champion by default, `--gen=N` to override), joins an
//! optional covariate table, and writes the canonical
//! `subject, feature…, theta…` CSV plus a provenance sidecar.
//!
//! The provenance link (pack, workspace, gen, plugin source hash, the
//! champion mechanism text) is what keeps B honest later: B's data is
//! A's verdict, and if A's champion improves, the digest changes and
//! B must be re-exported and re-fitted. A's hidden curves never cross
//! the bridge — only A's frozen numbers and its mechanism text.
//!
//! ```text
//! skill export --pack=<pack.rs> --workspace=<dir> [--workspace=<dir>]…
//!              [--gen=N] [--covariates=<csv>] [--id-column=<col>] --out=<csv>
//! ```
//!
//! Several `--workspace` dirs merge (one row per subject, first
//! workspace wins; subject sets are expected to be disjoint).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::pack_loader::PackHandle;
use crate::workspace::Workspace;

/// Parsed CLI shape of `skill export`.
pub struct ExportRequest {
    pub workspaces: Vec<PathBuf>,
    pub generation: Option<u32>,
    pub covariates: Option<PathBuf>,
    pub id_column: String,
    pub out: PathBuf,
}

pub fn export(handle: &PackHandle, args: &[String]) -> Result<()> {
    let request = parse(args)?;

    let pack = handle.pack.as_ref();
    let name = pack.name();

    // One row per subject, ordered by id; theta order from the first
    // successful subject's report (parameter declaration order).
    let mut rows: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    let mut theta_names: Vec<String> = Vec::new();
    let mut provenance_workspaces = Vec::new();

    for workspace_dir in &request.workspaces {
        let workspace = Workspace::new(workspace_dir.clone());

        let generation = match request.generation {
            Some(generation) => generation,
            None => workspace
                .champion()
                .with_context(|| {
                    format!(
                        "{} has no champion.json and no --gen= was given",
                        workspace_dir.display(),
                    )
                })?
                .generation,
        };

        let report = workspace
            .read_report(generation)
            .with_context(|| format!("no report.json for gen {generation:04} in {}", workspace_dir.display()))?;

        let mechanism_path = workspace.mechanism_path(generation);
        let mechanism_text = std::fs::read_to_string(&mechanism_path)
            .with_context(|| format!("reading {}", mechanism_path.display()))?;

        let mut fitted = 0usize;

        for subject in &report.per_subject {
            if !subject.status.starts_with("Success") {
                continue;
            }

            if theta_names.is_empty() {
                theta_names = subject.theta.iter().map(|(name, _)| name.clone()).collect();
            }

            let entry = rows.entry(subject.subject.clone()).or_default();

            if !entry.is_empty() {
                eprintln!(
                    "skill: subject {} already exported from an earlier workspace — keeping the first",
                    subject.subject,
                );
                continue;
            }

            *entry = subject.theta.iter().cloned().collect();
            fitted += 1;
        }

        provenance_workspaces.push(serde_json::json!({
            "workspace": workspace_dir.display().to_string(),
            "generation": generation,
            "source_hash": report.source_hash,
            "fitted_subjects": fitted,
            "mechanism": mechanism_text,
        }));
    }

    if rows.is_empty() {
        bail!("no successfully fitted subjects found in any workspace");
    }

    // Covariates: header order preserved, joined on the id column.
    let mut covariate_names: Vec<String> = Vec::new();
    let mut covariates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    if let Some(path) = &request.covariates {
        let mut reader = csv::Reader::from_path(path)
            .with_context(|| format!("opening covariates {}", path.display()))?;

        let headers = reader.headers().context("reading covariate headers")?.clone();

        let id_index = headers
            .iter()
            .position(|header| header == request.id_column)
            .with_context(|| {
                format!(
                    "covariate file has no id column {:?} (headers: {})",
                    request.id_column,
                    headers.iter().collect::<Vec<_>>().join(", "),
                )
            })?;

        covariate_names = headers
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != id_index)
            .map(|(_, header)| header.to_string())
            .collect();

        for record in reader.records() {
            let record = record.context("parsing covariate row")?;

            covariates.insert(
                record.get(id_index).unwrap_or_default().to_string(),
                covariate_names
                    .iter()
                    .map(|header| {
                        record
                            .get(headers.iter().position(|name| name == header).unwrap())
                            .unwrap_or_default()
                            .to_string()
                    })
                    .collect(),
            );
        }
    }

    // The canonical table: subject, feature…, theta…
    let mut table = String::new();
    table.push_str("subject");

    for header in covariate_names.iter().chain(theta_names.iter()) {
        table.push(',');
        table.push_str(header);
    }

    table.push('\n');

    let mut joined = 0usize;

    for (subject, theta) in &rows {
        table.push_str(subject);

        if let Some(values) = covariates.get(subject) {
            joined += 1;

            for value in values {
                table.push(',');
                table.push_str(&csv_cell(value));
            }
        } else {
            for _ in &covariate_names {
                table.push(',');
            }
        }

        for parameter in &theta_names {
            table.push(',');

            match theta.get(parameter) {
                Some(value) => table.push_str(&value.to_string()),
                None => table.push_str(""),
            }
        }

        table.push('\n');
    }

    std::fs::write(&request.out, &table)
        .with_context(|| format!("writing {}", request.out.display()))?;

    let provenance_path = format!("{}.provenance.json", request.out.display());

    let provenance = serde_json::json!({
        "pack": name,
        "workspaces": provenance_workspaces,
        "parameters": theta_names,
        "covariates": covariate_names,
        "subjects": rows.len(),
        "covariates_joined": joined,
        "table_digest": crate::workspace::source_hash(&table),
    });

    std::fs::write(
        Path::new(&provenance_path),
        serde_json::to_string_pretty(&provenance).expect("serializable"),
    )
    .with_context(|| format!("writing {provenance_path}"))?;

    println!(
        "skill: exported {} subjects × {} parameters (+{} covariates, {joined} joined) -> {}",
        rows.len(),
        theta_names.len(),
        covariate_names.len(),
        request.out.display(),
    );
    println!("skill: provenance -> {provenance_path}");

    Ok(())
}

fn parse(args: &[String]) -> Result<ExportRequest> {
    let flag = |prefix: &str| -> Option<String> {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .map(str::to_string)
    };

    let workspaces: Vec<PathBuf> = args
        .iter()
        .filter_map(|arg| arg.strip_prefix("--workspace="))
        .map(PathBuf::from)
        .collect();

    if workspaces.is_empty() {
        bail!("export needs at least one --workspace=<dir>");
    }

    let out = flag("--out=")
        .map(PathBuf::from)
        .context("export needs --out=<csv>")?;

    Ok(ExportRequest {
        workspaces,
        generation: flag("--gen=").map(|value| value.parse()).transpose().context("--gen= expects a number")?,
        covariates: flag("--covariates=").map(PathBuf::from),
        id_column: flag("--id-column=").unwrap_or_else(|| "id".to_string()),
        out,
    })
}

/// Quote a covariate cell if it could otherwise break the CSV.
fn csv_cell(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}