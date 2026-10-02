//! The evolution driver: the generation loop wired to one pack.
//!
//! This is the generalized mirror of the binding harness's
//! `binding_evolve` command. Everything it does — seeding,
//! theorizing, lowering, evaluating, judging, promoting, ledgering —
//! is harness-fixed; the only problem-specific inputs are the pack's
//! data, split policy, baseline, lexicon, and prompt text.
//!
//! ```text
//! skill evolve --pack=packs/binding/pack.rs --subjects=a,b --gens=10
//! ```

use std::path::PathBuf;

use crate::bench;
use crate::candidate::CandidateModel;
use crate::data::Curve;
use crate::evolve::{
    self, AuthorDiagnostics, GenContext, JudgeContext, SubjectBrief,
};
use crate::ledger::{self, LedgerEntry, Mode, Role, Verdict};
use crate::lexicon::Lexicon;
use crate::lower::{self, Lowered};
use crate::mechanism::MechanismModel;
use crate::mechanism_ir::{self, Mechanism};
use crate::pack::DiagnosticMode;
use crate::pack_loader::PackHandle;
use crate::plot::{self, SubjectPanel};
use crate::plugin::load_candidate;
use crate::split::{self, BenchmarkDataset, CurveRoles};
use crate::workspace::{
    self, Champion, EvaluatorOnly, GenReport, HoldoutScore, SubjectReport, Workspace,
};
use crate::agent::LlmConfig;

/// Consecutive non-improving generations before the harness forces
/// the theorist into explore mode.
const STALL_LIMIT: u32 = 3;

/// How many recent ledger entries the theorist sees each turn.
const LEDGER_DIGEST_LIMIT: usize = 12;

/// Forward one loop-lifecycle note to the SSE debug viewer
/// ([`crate::viewer`]); compiles to nothing without the `viewer`
/// feature.
#[cfg(feature = "viewer")]
fn observe(message: impl Into<String>) {
    crate::viewer::publish("info", message);
}

#[cfg(not(feature = "viewer"))]
fn observe(_: impl Into<String>) {}

/// Run the discovery loop for one pack. Returns a process exit
/// message on failure.
pub fn run(handle: &PackHandle, args: &[String]) -> Result<(), String> {
    let pack = handle.pack.as_ref();

    // SSE debug viewer: every agent stream and lifecycle note is
    // published live while the loop runs (feature `viewer`).
    #[cfg(feature = "viewer")]
    {
        let addr = crate::viewer::default_addr();

        crate::viewer::publish("info", format!("skill viewer listening on {addr}"));
        crate::viewer::serve(&addr);

        println!("skill: viewer on http://{addr}/");
    }

    let flag = |prefix: &str| -> Option<String> {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .map(str::to_string)
    };

    let gens: u32 = flag("--gens=")
        .unwrap_or_else(|| "1".to_string())
        .parse()
        .map_err(|_| "--gens= expects a number")?;

    let subject_count: usize = flag("--subjects-limit=")
        .unwrap_or_else(|| "5".to_string())
        .parse()
        .map_err(|_| "--subjects-limit= expects a number")?;

    let max_nfev: usize = flag("--nfev=")
        .unwrap_or_else(|| "200".to_string())
        .parse()
        .map_err(|_| "--nfev= expects a number")?;

    let skip_evolve = args.iter().any(|arg| arg == "--no-evolve");

    // Named subjects (candidate-restricted discovery). `--subject=`
    // entries accumulate; `--subjects=a,b,c` adds a whole list.
    let mut subject_ids: Vec<String> = args
        .iter()
        .filter_map(|arg| arg.strip_prefix("--subject="))
        .map(str::to_string)
        .collect();

    if let Some(list) = flag("--subjects=") {
        subject_ids.extend(
            list.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        );
    }

    let holdout_id = flag("--holdout-subject=").or_else(|| flag("--holdout="));

    // Per-subject lineages keep fitness numbers comparable: a
    // subject-restricted normalized RMSE means something different
    // from a multi-subject mean. The holdout joins the name so
    // transfer experiments never share a lineage with runs without it.
    let default_workspace = match subject_ids.is_empty() {
        false => {
            let mut name = subject_ids.join("+");

            if let Some(id) = &holdout_id {
                name.push('+');
                name.push_str(id);
            }

            crate::workspace_root()
                .join("workspace")
                .join(pack.name())
                .join(name)
        }
        true => crate::workspace_root().join("workspace").join(pack.name()),
    };

    let workspace = Workspace::new(
        flag("--workspace=")
            .map(PathBuf::from)
            .unwrap_or(default_workspace),
    );

    let lexicon = pack.lexicon();

    // The seed idea: the baseline mechanism the loop starts from.
    let baseline = match flag("--mechanism=") {
        Some(path) => PathBuf::from(path),
        None => handle.resolve(&pack.baseline().mechanism),
    };

    // Load the pack's subjects; the pack owns ingestion, the harness
    // owns the split.
    let subjects = pack.subjects().map_err(|error| format!("{error:#}"))?;

    // Resolve the named subjects first, so an unknown id fails with
    // context instead of an empty benchmark.
    for id in &subject_ids {
        if !subjects.iter().any(|subject| &subject.id == id) {
            return Err(format!(
                "subject `{id}` is not in the pack's dataset; ids are: {}",
                subjects
                    .iter()
                    .map(|subject| subject.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
    }

    let benchmark = split::split_subjects(subjects, &pack.split_policy());

    // `train_subjects` is the author-facing view: every subject's id
    // with its train-role curves. Subject-restricted runs take it
    // from the named subjects; unrestricted runs build it from every
    // discovery subject selected — the theorist always sees its
    // measurement file and the diagnostic plot.
    let (indices, train_subjects) = if subject_ids.is_empty() {
        let indices = bench::strongest(&benchmark, subject_count);

        let train_subjects: Vec<(String, Vec<Curve>)> = indices
            .iter()
            .map(|&index| {
                let visible = &benchmark.discovery[index];
                let held_out = &benchmark.evaluation[index];

                let mut curves = visible.visible_curves.clone();
                curves.extend(held_out.held_out_curves.iter().cloned());

                let roles = split::split_roles(&curves, &pack.split_policy());

                (visible.id.clone(), roles.train)
            })
            .collect();

        (indices, train_subjects)
    } else {
        let mut indices = Vec::new();
        let mut train_subjects: Vec<(String, Vec<Curve>)> = Vec::new();

        for id in &subject_ids {
            let index = benchmark
                .discovery
                .iter()
                .position(|subject| &subject.id == id)
                .ok_or_else(|| {
                    format!(
                        "subject `{id}` is in the dataset but has no \
                         held-out curves, so it cannot take part in the \
                         benchmark split",
                    )
                })?;

            // Every session of the subject is its data: replicate
            // runs are repeated measurements of one mechanism, and
            // the optimizer balances them. (A previous "newest
            // session only" restriction was a binding-era hack
            // inferred from data shape; it silently discarded whole
            // replicates in generated packs. If per-run drift ever
            // needs modeling, that belongs in the mechanism —
            // session-indexed parameters — not in a hidden filter.)
            // The author-facing measurement file gets exactly the
            // train-role curves — the validation level stays withheld
            // from the discovery loop.
            let visible = &benchmark.discovery[index];
            let held_out = &benchmark.evaluation[index];

            let mut curves = visible.visible_curves.clone();
            curves.extend(held_out.held_out_curves.iter().cloned());

            let roles = split::split_roles(&curves, &pack.split_policy());

            indices.push(index);
            train_subjects.push((id.clone(), roles.train));
        }

        (indices, train_subjects)
    };

    let policy = pack.split_policy();

    match subject_ids.is_empty() {
        false => {
            let line = format!(
                "skill: subject-restricted discovery on {} subjects: {} \
                 | train curves: {}",
                indices.len(),
                subject_ids.join(", "),
                train_subjects
                    .iter()
                    .map(|(id, curves)| format!("{id}:{}", curves.len()))
                    .collect::<Vec<_>>()
                    .join(" "),
            );

            observe(line.clone());
            println!("{line}");
        }
        true => {
            let line = format!(
                "skill: evolving on {} subjects: {}",
                indices.len(),
                indices
                    .iter()
                    .map(|index| benchmark.discovery[*index].id.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            );

            observe(line.clone());
            println!("{line}");
        }
    }

    // The sealed holdout subject: never added to `indices`,
    // `train_subjects`, the briefs, or the ledger — only
    // `evaluate_gen` touches it, and only to fill the report's
    // evaluator-only section.
    let holdout = match &holdout_id {
        None => None,

        Some(id) => {
            if subject_ids.iter().any(|subject| subject == id) {
                return Err(format!(
                    "holdout subject `{id}` must not also be a discovery subject",
                ));
            }

            let index = benchmark
                .discovery
                .iter()
                .position(|subject| &subject.id == id)
                .ok_or_else(|| {
                    format!(
                        "holdout subject `{id}` is not in the benchmark split \
                         (unknown id, or it has no held-out curves)",
                    )
                })?;

            // Same rule as the discovery subjects: every session counts.
            let mut curves = benchmark.discovery[index].visible_curves.clone();

            curves.extend(
                benchmark.evaluation[index]
                    .held_out_curves
                    .iter()
                    .cloned(),
            );

            let roles = split::split_roles(&curves, &policy);

            if roles.train.is_empty() || roles.validation.is_empty() {
                return Err(format!(
                    "holdout subject `{id}` has no train or validation curves",
                ));
            }

            println!(
                "skill: sealed holdout `{id}` (evaluator-only transfer test; \
                 the discovery loop never sees it)",
            );

            Some(HoldoutSetup {
                id: id.to_string(),
                span: signal_span(&roles.train),
                roles,
            })
        }
    };

    // The theorist/doctor config, resolved up front so a
    // misconfigured loop fails before it touches the workspace.
    let llm = if skip_evolve {
        None
    } else {
        Some(LlmConfig::from_env().map_err(|error| format!("LLM config: {error:#}"))?)
    };

    // Prompt texts: static contract + pack domain text.
    let problem_definition = pack.problem_statement();

    let theorist_preamble = {
        let mut text = evolve::theorist_preamble_static(&lexicon);

        let domain = pack.theorist_preamble();

        if !domain.trim().is_empty() {
            text.push_str("\n\n");
            text.push_str(domain.trim());
        }

        text
    };

    let doctor_preamble = {
        let mut text = evolve::doctor_preamble_static(&lexicon);

        let domain = pack.doctor_preamble();

        if !domain.trim().is_empty() {
            text.push_str("\n\n");
            text.push_str(domain.trim());
        }

        text
    };

    let referee_preamble = {
        let mut text = evolve::referee_preamble_static(&problem_definition);

        let domain = pack.referee_preamble();

        if !domain.trim().is_empty() {
            text.push_str("\n\n");
            text.push_str(domain.trim());
        }

        text
    };

    // Seed the workspace with the baseline mechanism; resumed
    // workspaces keep their existing generations.
    workspace
        .seed_mechanism(&baseline)
        .map_err(|error| format!("{error:#}"))?;

    // Seed generation: give it a compilable plugin by lowering its
    // mechanism (fresh workspaces), then evaluate the untouched
    // baseline unless it was evaluated already.
    let seed = workspace
        .latest_gen()
        .ok_or("workspace has no generations")?;

    if !workspace.plugin_path(seed).exists() {
        let mechanism = load_mechanism(&workspace.mechanism_path(seed), &lexicon)
            .map_err(|error| format!("seed mechanism: {error}"))?;

        let lowered = lower::lower(&mechanism, &lexicon, "baseline seed")
            .map_err(|errors| format!("seed lowering failed:\n{errors}"))?;

        write_generation_artifacts(&workspace, seed, &lowered)?;
    }

    let seed_report = match workspace.read_report(seed) {
        Some(report) => report,
        None => {
            let report = evaluate_gen(
                seed,
                &workspace,
                &benchmark,
                &indices,
                holdout.as_ref(),
                max_nfev,
                &lexicon,
                &policy,
            );

            workspace
                .write_report(&report)
                .map_err(|error| format!("{error:#}"))?;

            report
        }
    };

    print_report(&seed_report);

    let mut champion = match workspace.champion() {
        Some(champion) => champion,
        None => {
            let fitness = seed_report
                .fitness
                .ok_or("seed generation did not produce a fitness")?;

            let champion = Champion { generation: seed, fitness };

            workspace
                .set_champion(&champion)
                .map_err(|error| format!("{error:#}"))?;

            champion
        }
    };

    println!(
        "skill: champion gen {:04} at fitness {:.4}",
        champion.generation, champion.fitness,
    );

    // The research ledger shared by every generation.
    let ledger_path = workspace.ledger_path();

    // Consecutive evaluated generations without a promotion; after
    // STALL_LIMIT the harness forces the theorist into explore mode.
    let mut flat_gens = 0u32;

    // The champion's diagnostic figure, cached by champion
    // generation: the plot depends only on the champion's frozen
    // mechanism and theta, so an unchanged champion deserves an
    // unchanged (and unsimulated) picture.
    let mut champion_plot_cache: Option<(u32, Vec<u8>)> = None;

    for _ in 0..gens {
        let generation = workspace
            .latest_gen()
            .ok_or("workspace lost its generations")?
            + 1;

        // Start from the champion's idea. Legacy champions
        // (plugin-only workspaces from before the mechanism IR)
        // restart from the baseline mechanism.
        let mechanism_path =
            match workspace
                .copy_champion_mechanism_into(champion.generation, generation)
                .map_err(|error| format!("{error:#}"))?
            {
                Some(path) => path,
                None => {
                    println!(
                        "skill: champion gen {:04} has no mechanism.json (legacy workspace) \
                         — restarting from the baseline idea",
                        champion.generation,
                    );

                    let target = workspace.mechanism_path(generation);

                    std::fs::copy(&baseline, &target)
                        .map_err(|error| format!("copying baseline mechanism: {error}"))?;

                    target
                }
            };

        // The theorist's sandbox is the generation directory; drop
        // the reduced-sample measurement file next to the mechanism
        // it informs.
        let observation_csv = if train_subjects.is_empty() {
            None
        } else {
            Some(
                evolve::write_observation_csv(
                    &workspace.gen_dir(generation),
                    &train_subjects,
                    evolve::OBSERVATION_CSV_POINTS,
                )
                .map_err(|error| format!("{error:#}"))?,
            )
        };

        // The idea being refined this generation, hashed before the
        // theorist overwrites the file.
        let champion_mechanism =
            load_mechanism(&mechanism_path, &lexicon)
                .map_err(|error| format!("champion: {error}"))?;

        let champion_hash = mechanism_ir::mechanism_hash(&champion_mechanism);

        let parent = workspace
            .read_report(champion.generation)
            .ok_or_else(|| format!("champion gen {} has no report", champion.generation))?;

        // What the loop can say about each subject: identity, public
        // metadata, and the champion's frozen numbers for it — the
        // subjects' definition for the theorist and the referee.
        let subject_briefs: Vec<SubjectBrief> = indices
            .iter()
            .map(|&index| {
                let visible = &benchmark.discovery[index];

                let fitted = parent
                    .per_subject
                    .iter()
                    .find(|subject| subject.subject == visible.id)
                    .map(|subject| subject.theta.clone())
                    .unwrap_or_default();

                SubjectBrief {
                    id: visible.id.clone(),
                    title: visible.title.clone(),
                    detail: visible.detail.clone(),
                    fitted,
                }
            })
            .collect();

        // The theorist's diagnostic image: the champion's frozen fit
        // simulated at the training input levels against the measured
        // data — one row per subject, each with its own frozen
        // parameters. Written beside the mechanism as plot.png and
        // attached to the prompt as an image.
        let plot_png_base64 = if train_subjects.is_empty() {
            None
        } else if champion_plot_cache
            .as_ref()
            .is_some_and(|(cached, _)| *cached == champion.generation)
        {
            let (_, png) = champion_plot_cache
                .clone()
                .expect("cache presence checked above");

            let path = workspace.gen_dir(generation).join(plot::PLOT_PNG_NAME);

            if let Err(error) = std::fs::write(&path, &png) {
                eprintln!("skill: could not write {}: {error}", path.display());
            }

            Some(plot::encode_base64(&png))
        } else {
            let champion_thetas: Vec<Vec<(String, f64)>> = train_subjects
                .iter()
                .map(|(id, _)| {
                    parent
                        .per_subject
                        .iter()
                        .find(|subject| subject.subject == *id)
                        .map(|subject| subject.theta.clone())
                        .unwrap_or_default()
                })
                .collect();

            let panels: Vec<SubjectPanel> = train_subjects
                .iter()
                .zip(&champion_thetas)
                .map(|((id, curves), theta)| SubjectPanel {
                    id: id.clone(),
                    theta,
                    curves,
                })
                .collect();

            let figure = if pack.diagnostic_mode() == DiagnosticMode::Parity {
                plot::parity_diagnostic_plot_png(&champion_mechanism, &lexicon, &panels)
            } else {
                plot::multi_diagnostic_plot_png(&champion_mechanism, &lexicon, &panels, &policy.train)
            };

            match figure {
                Ok(png) => {
                    let path = workspace.gen_dir(generation).join(plot::PLOT_PNG_NAME);

                    if let Err(error) = std::fs::write(&path, &png) {
                        eprintln!("skill: could not write {}: {error}", path.display());
                    }

                    champion_plot_cache = Some((champion.generation, png.clone()));

                    Some(plot::encode_base64(&png))
                }
                Err(error) => {
                    eprintln!("skill: diagnostic plot failed: {error:#}");
                    None
                }
            }
        };

        // The same image doubles as the referee's view of the
        // champion when the judge step runs below.
        let champion_plot_png_base64 = plot_png_base64.clone();

        let context = GenContext {
            generation,
            mechanism_path: mechanism_path.clone(),
            parent: AuthorDiagnostics::from_report(&parent),
            observation_csv: observation_csv.clone(),
            plot_png_base64,
            ledger_digest: ledger::digest(&ledger::read(&ledger_path), LEDGER_DIGEST_LIMIT),
            subjects: subject_briefs.clone(),
            subject_table_intro: pack.subject_table_intro(),
            problem_definition: problem_definition.clone(),
            preamble: theorist_preamble.clone(),
            lexicon: lexicon.clone(),
            forced_mode: (flat_gens >= STALL_LIMIT).then_some(Mode::Explore),
            champion_hash: champion_hash.clone(),
        };

        // Theorize: three mechanism ideas, one chosen (skipped under
        // --no-evolve, which keeps the champion's mechanism — an
        // identity refit). An unreachable or silent theorist fails
        // the GENERATION, never the run: the LLM is the flakiest part
        // of the system and the loop must survive it.
        let mut theorize_error = None;

        let theory = match &llm {
            Some(config) => match evolve::theorize(config, &context) {
                Ok(theory) => Some(theory),
                Err(error) => {
                    theorize_error = Some(format!("theorize failed: {error:#}"));

                    None
                }
            },
            None => None,
        };

        let note = theory.as_ref().map_or(
            theorize_error.clone().unwrap_or_else(|| "--no-evolve identity refit".to_string()),
            |theory| theory.chosen_proposal().hypothesis.clone(),
        );

        // Lower the chosen idea. The mechanism file is the source of
        // truth (the doctor may repair it), so lower from disk; on
        // failure the doctor gets one repair round, then the
        // generation is recorded as failed and the loop moves on —
        // including when the doctor itself is the thing that fails.
        let lowered = match theorize_error.clone() {
            Some(error) => Err(error),
            None => match lower_mechanism_from(&mechanism_path, &lexicon, &note) {
                Ok(lowered) => Ok(lowered),
                Err(errors) => match &llm {
                    Some(config) => {
                        println!("skill: gen {generation:04} lowering failed — calling the doctor");

                        match evolve::doctor(config, &mechanism_path, &errors, &doctor_preamble) {
                            Ok(()) => {
                                lower_mechanism_from(&mechanism_path, &lexicon, &note).map_err(
                                    |more| {
                                        format!(
                                            "{errors}\nafter the doctor round, still failing:\n{more}"
                                        )
                                    },
                                )
                            }
                            Err(doctor) => Err(format!(
                                "{errors}\nthe doctor itself failed (generation abandoned):\n{doctor:#}"
                            )),
                        }
                    }
                    None => Err(format!("{errors}\n(--no-evolve: no doctor round)")),
                },
            },
        };

        let lowered = match lowered {
            Ok(lowered) => lowered,

            Err(error) => {
                let status = if theorize_error.is_some() {
                    "TheorizeFailed"
                } else {
                    "LowerFailed"
                };

                let report =
                    failed_report(generation, champion.generation, &note, status, &error);

                workspace
                    .write_report(&report)
                    .map_err(|e| format!("{e:#}"))?;

                print_report(&report);

                let hash = load_mechanism(&mechanism_path, &lexicon)
                    .map(|mechanism| mechanism_ir::mechanism_hash(&mechanism))
                    .unwrap_or_else(|_| champion_hash.clone());

                let (hypothesis, prediction, falsifier, cost, mode) =
                    proposal_fields(&theory, &note);

                ledger::append(
                    &ledger_path,
                    &LedgerEntry {
                        generation,
                        mechanism_hash: hash,
                        role: Role::Evaluated,
                        verdict: Verdict::Failed,
                        mode,
                        hypothesis,
                        prediction,
                        falsifier,
                        cost,
                        status: Some(report.status.clone()),
                        fitness: None,
                        delta: None,
                        error: Some(error),
                        note: None,
                    },
                )
                .map_err(|e| format!("{e:#}"))?;

                flat_gens += 1;

                continue;
            }
        };

        write_generation_artifacts(&workspace, generation, &lowered)?;

        // The mechanism as lowered (post-doctor) — the ledger's
        // record of what was actually implemented.
        let implemented = load_mechanism(&mechanism_path, &lexicon)
            .map_err(|error| format!("implemented mechanism: {error}"))?;

        let implemented_hash = mechanism_ir::mechanism_hash(&implemented);

        // Every subject is fitted at the full budget: the referee
        // compares simulated trajectories against the champion's, and
        // a starved fit would be an unfair picture it cannot see
        // through.
        let report = evaluate_gen(
            generation,
            &workspace,
            &benchmark,
            &indices,
            holdout.as_ref(),
            max_nfev,
            &lexicon,
            &policy,
        );

        workspace
            .write_report(&report)
            .map_err(|error| format!("{error:#}"))?;

        print_report(&report);

        // Champion selection by referee: the judge compares both
        // models' simulated trajectories against the measured data
        // and decides, deliberately unaided by the fitness number —
        // the number stays in the report and on the console for the
        // record. Numeric selection remains for --no-evolve and
        // failed evaluations.
        let decision = match (&llm, report.fitness) {
            (Some(config), Some(_)) if !train_subjects.is_empty() => {
                let candidate_thetas: Vec<Vec<(String, f64)>> = train_subjects
                    .iter()
                    .map(|(id, _)| {
                        report
                            .per_subject
                            .iter()
                            .find(|subject| subject.subject == *id)
                            .map(|subject| subject.theta.clone())
                            .unwrap_or_default()
                    })
                    .collect();

                let panels: Vec<SubjectPanel> = train_subjects
                    .iter()
                    .zip(&candidate_thetas)
                    .map(|((id, curves), theta)| SubjectPanel {
                        id: id.clone(),
                        theta,
                        curves,
                    })
                    .collect();

                let png = if pack.diagnostic_mode() == DiagnosticMode::Parity {
                    plot::parity_diagnostic_plot_png(&implemented, &lexicon, &panels)
                } else {
                    plot::multi_diagnostic_plot_png(&implemented, &lexicon, &panels, &policy.train)
                }
                .map_err(|error| format!("candidate plot failed: {error:#}"))?;

                let plot_path = workspace
                    .gen_dir(generation)
                    .join(plot::CANDIDATE_PLOT_PNG_NAME);

                std::fs::write(&plot_path, &png)
                    .map_err(|error| format!("writing {}: {error}", plot_path.display()))?;

                let judgement = match champion_plot_png_base64.clone() {
                    None => {
                        eprintln!(
                            "skill: gen {generation:04} champion plot unavailable — the referee \
                             cannot judge; falling back to the fitness comparison"
                        );

                        None
                    }
                    Some(champion_plot) => match evolve::judge(
                        config,
                        &JudgeContext {
                            generation,
                            champion_mechanism: &champion_mechanism,
                            candidate_mechanism: &implemented,
                            champion_plot_png_base64: champion_plot,
                            candidate_plot_png_base64: plot::encode_base64(&png),
                            subjects: subject_briefs.clone(),
                            subject_table_intro: pack.subject_table_intro(),
                            problem_definition: problem_definition.clone(),
                            preamble: referee_preamble.clone(),
                        },
                    ) {
                        Ok(decision) => Some(decision),
                        Err(error) => {
                            // A silent referee is not a dead run: without
                            // a judgement the fitness comparison decides,
                            // exactly as under --no-evolve.
                            eprintln!(
                                "skill: gen {generation:04} referee failed: {error:#} — falling \
                                 back to the fitness comparison"
                            );

                            None
                        }
                    },
                };

                if let Some(decision) = &judgement {
                    let decision_path = workspace.gen_dir(generation).join("decision.json");

                    std::fs::write(
                        &decision_path,
                        serde_json::to_string_pretty(decision).map_err(|error| format!("{error}"))?,
                    )
                    .map_err(|error| format!("writing {}: {error}", decision_path.display()))?;
                }

                judgement
            }
            _ => None,
        };

        // Ledger: the evaluated idea, then every unchosen proposal as
        // untested — the next theorist turn sees both. The verdict is
        // the referee's when it judged; the fitness comparison only
        // when it did not.
        let verdict = match (decision.as_ref(), report.fitness) {
            (Some(decision), _) if decision.promote_to_champion => Verdict::Promoted,
            (Some(_), _) => Verdict::Rejected,
            (None, Some(fitness)) if fitness < champion.fitness => Verdict::Promoted,
            (None, Some(_)) => Verdict::Rejected,
            (None, None) => Verdict::Failed,
        };

        let (hypothesis, prediction, falsifier, cost, mode) =
            proposal_fields(&theory, &note);

        ledger::append(
            &ledger_path,
            &LedgerEntry {
                generation,
                mechanism_hash: implemented_hash.clone(),
                role: Role::Evaluated,
                verdict,
                mode,
                hypothesis,
                prediction,
                falsifier,
                cost,
                status: Some(report.status.clone()),
                fitness: report.fitness,
                delta: report.fitness.map(|fitness| fitness - champion.fitness),
                error: report.error.as_deref().map(ledger::sanitize),
                note: decision.as_ref().map(|decision| decision.reasoning.clone()),
            },
        )
        .map_err(|e| format!("{e:#}"))?;

        if let Some(theory) = &theory {
            for (index, proposal) in theory.proposals.iter().enumerate() {
                if index == theory.chosen {
                    continue;
                }

                ledger::append(
                    &ledger_path,
                    &LedgerEntry {
                        generation,
                        mechanism_hash: proposal.hash(),
                        role: Role::Proposed,
                        verdict: Verdict::Untested,
                        mode: theory.mode,
                        hypothesis: proposal.hypothesis.clone(),
                        prediction: proposal.prediction.clone(),
                        falsifier: proposal.falsifier.clone(),
                        cost: proposal.cost.clone(),
                        status: None,
                        fitness: None,
                        delta: None,
                        error: None,
                        note: Some("proposed but not chosen this generation".to_string()),
                    },
                )
                .map_err(|e| format!("{e:#}"))?;
            }
        }

        let promoted = match (decision.as_ref(), report.fitness) {
            (Some(decision), _) => decision.promote_to_champion,
            (None, Some(fitness)) => fitness < champion.fitness,
            (None, None) => false,
        };

        if promoted {
            champion = Champion {
                generation,
                fitness: report.fitness.unwrap_or(champion.fitness),
            };

            workspace
                .set_champion(&champion)
                .map_err(|error| format!("{error:#}"))?;

            println!("skill: gen {generation:04} PROMOTED — new champion");
            observe(format!("gen {generation:04} PROMOTED — new champion"));

            flat_gens = 0;
        }

        if !promoted {
            match &decision {
                Some(_) => {
                    let line = format!(
                        "skill: gen {generation:04} discarded by the referee — next gen re-copies champion {:04}",
                        champion.generation,
                    );

                    observe(line.clone());
                    println!("{line}");
                }
                None if report.fitness.is_none() => {
                    let line = format!(
                        "skill: gen {generation:04} failed to simulate — nothing to \
                         judge, next gen re-copies champion {:04}",
                        champion.generation,
                    );

                    observe(line.clone());
                    println!("{line}");
                }
                None => {
                    let line = format!(
                        "skill: gen {generation:04} not an improvement — next gen re-copies champion {:04}",
                        champion.generation,
                    );

                    observe(line.clone());
                    println!("{line}");
                }
            }

            flat_gens += 1;

            if flat_gens >= STALL_LIMIT {
                println!(
                    "skill: {flat_gens} flat generations — the next theorist turn is forced explore",
                );
            }
        }
    }

    let champion_report = workspace.read_report(champion.generation);

    let line = format!(
        "skill: final champion gen {:04} fitness {:.4} (hidden {})",
        champion.generation,
        champion.fitness,
        champion_report
            .as_ref()
            .and_then(|report| report.evaluator_only.hidden_rmse)
            .map(|hidden| format!("{hidden:.4}"))
            .unwrap_or_else(|| "n/a".to_string()),
    );

    observe(line.clone());
    println!("{line}");

    if let Some(holdout) = champion_report.and_then(|report| report.evaluator_only.holdout) {
        println!(
            "skill: holdout transfer `{}` — norm validation rmse {:.4} (raw {:.4}, hidden {:.4}) \
             | mechanism hash the champion's, parameters fitted from the holdout's own train curves",
            holdout.subject,
            holdout.normalized_validation_rmse,
            holdout.validation_rmse,
            holdout.hidden_rmse,
        );
    }

    Ok(())
}

/// Read, parse, and validate a `mechanism.json`.
fn load_mechanism(path: &std::path::Path, lexicon: &Lexicon) -> Result<Mechanism, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;

    let mechanism: Mechanism = serde_json::from_str(&text)
        .map_err(|error| format!("parsing {}: {error}", path.display()))?;

    mechanism_ir::validate(&mechanism, lexicon)
        .map_err(|errors| format!("{} failed IR validation:\n{errors}", path.display()))?;

    Ok(mechanism)
}

/// Validate and lower the mechanism file in place, with the given
/// rationale note (flows into the generated plugin and report).
fn lower_mechanism_from(
    path: &std::path::Path,
    lexicon: &Lexicon,
    note: &str,
) -> Result<Lowered, String> {
    let mechanism = load_mechanism(path, lexicon)?;

    lower::lower(&mechanism, lexicon, note)
}

/// Persist the lowering artifacts: the Modelica source (for humans)
/// and the generated plugin (for the evaluator).
fn write_generation_artifacts(
    workspace: &Workspace,
    generation: u32,
    lowered: &Lowered,
) -> Result<(), String> {
    std::fs::write(workspace.gen_dir(generation).join("mechanism.mo"), &lowered.modelica)
        .map_err(|error| format!("writing mechanism.mo: {error}"))?;

    std::fs::write(workspace.plugin_path(generation), &lowered.plugin_source)
        .map_err(|error| format!("writing plugin.rs: {error}"))?;

    Ok(())
}

/// A report for a generation that never produced a compilable plugin.
fn failed_report(
    generation: u32,
    parent: u32,
    note: &str,
    status: &str,
    error: &str,
) -> GenReport {
    GenReport {
        generation,
        parent: Some(parent),
        source_hash: workspace::source_hash(""),
        note: Some(note.to_string()),
        status: status.to_string(),
        error: Some(error.to_string()),
        complexity: None,
        fitness: None,
        per_subject: Vec::new(),
        evaluator_only: Default::default(),
    }
}

/// The chosen proposal's theory fields, or identity-refit defaults
/// when there is no theory (`--no-evolve`).
fn proposal_fields(
    theory: &Option<evolve::Theory>,
    note: &str,
) -> (String, Option<String>, Option<String>, Option<String>, Mode) {
    theory.as_ref().map_or(
        (note.to_string(), None, None, None, Mode::Exploit),
        |theory| {
            let proposal = theory.chosen_proposal();

            (
                proposal.hypothesis.clone(),
                proposal.prediction.clone(),
                proposal.falsifier.clone(),
                proposal.cost.clone(),
                theory.mode,
            )
        },
    )
}

/// A sealed holdout subject: its roles (train curves for the
/// evaluator-only parameter fit, validation/hidden for the transfer
/// score) and the train-signal range used to normalize its RMSE.
struct HoldoutSetup {
    id: String,
    roles: CurveRoles,
    span: f64,
}

/// Dynamic range (max − min) of a set of measured curves — the
/// normalizer that puts subjects with different response amplitudes
/// on one scale.
fn signal_span(curves: &[Curve]) -> f64 {
    let (peak, floor) = curves.iter().fold(
        (f64::MIN, f64::MAX),
        |(peak, floor), curve| {
            let peak = peak.max(curve.raw.y.iter().fold(f64::MIN, |a, &b| a.max(b)));
            let floor = floor.min(curve.raw.y.iter().fold(f64::MAX, |a, &b| a.min(b)));
            (peak, floor)
        },
    );

    (peak - floor).max(1e-9)
}

/// Compile, fit, and score one generation's plugin, writing the full
/// report (author-safe fields + evaluator-only hidden scores).
fn evaluate_gen(
    generation: u32,
    workspace: &Workspace,
    benchmark: &BenchmarkDataset,
    indices: &[usize],
    holdout: Option<&HoldoutSetup>,
    max_nfev: usize,
    lexicon: &Lexicon,
    policy: &crate::pack::SplitPolicy,
) -> GenReport {
    let plugin_path = workspace.plugin_path(generation);

    let source = std::fs::read_to_string(&plugin_path).unwrap_or_default();

    let mut report = GenReport {
        generation,
        parent: None,
        source_hash: workspace::source_hash(&source),
        note: None,
        status: "CompileFailed".to_string(),
        error: None,
        complexity: None,
        fitness: None,
        per_subject: Vec::new(),
        evaluator_only: EvaluatorOnly::default(),
    };

    // Tier 1+2: the plugin's own rustc, then the mechanism's rumoca.
    let model = match load_candidate(&plugin_path)
        .map_err(|error| format!("{error:#}"))
        .and_then(|spec| {
            let model = CandidateModel::from_spec(spec.as_ref(), lexicon)
                .map_err(|error| format!("{error:#}"))?;

            Ok((spec, model))
        })
    {
        Ok((_, model)) => model,
        Err(error) => {
            report.error = Some(error);
            return report;
        }
    };

    report.note = model.note().map(str::to_string);
    report.complexity = Some(model.complexity());
    report.status = "Success".to_string();

    let mut normalized_total = 0.0;
    let mut hidden_total = 0.0;

    for &index in indices {
        let visible = &benchmark.discovery[index];
        let held_out = &benchmark.evaluation[index];

        let mut curves = visible.visible_curves.clone();
        curves.extend(held_out.held_out_curves.iter().cloned());

        let roles = split::split_roles(&curves, policy);

        let scored = bench::fit_and_score(&model, &roles, &visible.id, max_nfev, lexicon);

        let success = scored.is_success();

        if success {
            // Normalized by the subject's own train-signal range: a
            // subject responding at 8 units must not weigh 4x a 2
            // unit subject in the recorded mean just because its
            // residuals live on a bigger axis.
            normalized_total += scored.validation_rmse / signal_span(&roles.train);

            hidden_total += scored.hidden_rmse;
        } else {
            report.status = match &scored.status {
                crate::experiment::Status::SimulationFailed(_) => {
                    "SimulationFailed".to_string()
                }
                other => format!("{other:?}"),
            };

            report.error = Some(format!("{}: {:?}", visible.id, scored.status));
        }

        report.per_subject.push(SubjectReport {
            subject: scored.subject.clone(),
            status: format!("{:?}", scored.status),
            theta: scored.theta.clone(),
            train_loss: scored.train_loss,
            validation_rmse: scored.validation_rmse,
            hidden_rmse: scored.hidden_rmse,
            pinned: scored.pinned.clone(),
            modulation: scored.modulation,
        });
    }

    let all_succeeded = indices.len() == report.per_subject.len()
        && report
            .per_subject
            .iter()
            .all(|subject| subject.status.starts_with("Success"));

    if all_succeeded && !indices.is_empty() {
        report.fitness = Some(normalized_total / indices.len() as f64);
        report.evaluator_only.hidden_rmse = Some(hidden_total / indices.len() as f64);
    } else {
        report.fitness = None;
    }

    // Evaluator-only transfer test on the sealed holdout subject:
    // this mechanism was discovered without ever seeing the subject;
    // fit its parameters from its train curves and score the
    // prediction on its withheld input levels. A holdout failure
    // never changes the generation's status or fitness.
    if let Some(holdout) = holdout {
        let scored = bench::fit_and_score(&model, &holdout.roles, &holdout.id, max_nfev, lexicon);

        report.evaluator_only.holdout = Some(HoldoutScore {
            subject: scored.subject.clone(),
            status: format!("{:?}", scored.status),
            theta: scored.theta.clone(),
            validation_rmse: scored.validation_rmse,
            normalized_validation_rmse: if scored.is_success() {
                scored.validation_rmse / holdout.span
            } else {
                f64::NAN
            },
            hidden_rmse: scored.hidden_rmse,
        });
    }

    report
}

/// One-line-per-generation evidence: author-safe fields on the main
/// line, per-subject validation scores beneath, hidden only labeled
/// as evaluator-side. The headline fitness is the mean of the
/// per-subject validation RMSEs, each normalized by that subject's
/// signal range.
fn print_report(report: &GenReport) {
    let line = format!(
        "skill: gen {:04} [{}] hash {} | fitness (norm rmse) {} | complexity {}",
        report.generation,
        report.status,
        &report.source_hash[..8],
        report
            .fitness
            .map(|fitness| format!("{fitness:.4}"))
            .unwrap_or_else(|| "—".to_string()),
        report
            .complexity
            .map(|complexity| format!(
                "{}s/{}p/{}e",
                complexity.states, complexity.parameters, complexity.equations,
            ))
            .unwrap_or_else(|| "—".to_string()),
    );

    observe(line.clone());
    println!("{line}");

    if let Some(error) = &report.error {
        println!(
            "skill:   error: {}",
            error.chars().take(400).collect::<String>()
        );
    }

    for subject in &report.per_subject {
        println!(
            "skill:   {} | validation rmse {:.4} | status {}",
            subject.subject, subject.validation_rmse, subject.status,
        );

        // Sentinels: numeric self-checks the loop reports without
        // being asked — see bench::pinned_parameters / modulation_ratio.
        for pinned in &subject.pinned {
            println!("skill:   {} sentinel: {pinned}", subject.subject);
        }

        if subject.modulation.is_some_and(|m| m < 0.5) {
            println!(
                "skill:   {} sentinel: mechanism varies only {:.0}% of the data's \
                 response across the validation conditions — it largely ignores \
                 the condition axis",
                subject.subject,
                subject.modulation.unwrap_or_default() * 100.0,
            );
        }
    }

    println!(
        "skill:   (evaluator-only hidden rmse {})",
        report
            .evaluator_only
            .hidden_rmse
            .map(|hidden| format!("{hidden:.4}"))
            .unwrap_or_else(|| "—".to_string()),
    );

    if let Some(holdout) = &report.evaluator_only.holdout {
        println!(
            "skill:   (evaluator-only holdout `{}` | validation rmse {:.4} \
             | norm {:.4} | hidden {:.4} | {})",
            holdout.subject,
            holdout.validation_rmse,
            holdout.normalized_validation_rmse,
            holdout.hidden_rmse,
            holdout.status,
        );
    }
}
/// `skill report`: the numbers one generation actually produced, as
/// a table — per-subject train/validation/hidden losses and the
/// frozen parameters. The fit counterpart of `skill inspect` (which
/// shows only what the pack declares). No LLM, no endpoint needed.
///
/// Naming subjects reads back exactly the lineage `evolve` would
/// have used (`--subject=D` → `workspace/<pack>/D/`). Naming none
/// reports EVERY lineage of the pack — the unrestricted run and each
/// subject-restricted one — so "did it work on D? on T? on R?" is
/// one command.
pub fn report(handle: &PackHandle, args: &[String]) -> Result<(), String> {
    let flag = |prefix: &str| {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .map(str::to_string)
    };

    let mut subject_ids: Vec<String> = Vec::new();

    for arg in args {
        for prefix in ["--subject=", "--subjects="] {
            if let Some(value) = arg.strip_prefix(prefix) {
                subject_ids.extend(value.split(',').map(|id| id.trim().to_string()));
            }
        }
    }

    let generation = match flag("--gen=").map(|text| text.parse::<u32>()) {
        Some(Ok(generation)) => Some(generation),
        Some(Err(_)) => return Err("`--gen=` wants a number".to_string()),
        None => None,
    };

    if let Some(workspace) = flag("--workspace=").map(PathBuf::from) {
        return report_lineage(handle, &Workspace::new(workspace), generation);
    }

    if !subject_ids.is_empty() {
        let mut name = subject_ids.join("+");

        if let Some(id) = flag("--holdout-subject=").or_else(|| flag("--holdout=")) {
            name.push('+');
            name.push_str(&id);
        }

        return report_lineage(
            handle,
            &Workspace::new(
                crate::workspace_root()
                    .join("workspace")
                    .join(handle.pack.name())
                    .join(name),
            ),
            generation,
        );
    }

    // No subjects named: every lineage the pack has. The pack's own
    // directory is the unrestricted lineage; its subdirectories are
    // the subject-restricted ones (`D`, `D+T+R`, …).
    let root = crate::workspace_root()
        .join("workspace")
        .join(handle.pack.name());

    let mut lineages = Vec::new();

    let base = Workspace::new(&root);

    if base.has_generations() {
        lineages.push(base);
    }

    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();

        subdirs.sort();

        for dir in subdirs {
            let lineage = Workspace::new(dir);

            if lineage.has_generations() {
                lineages.push(lineage);
            }
        }
    }

    if lineages.is_empty() {
        return Err(format!("no generations under {}", root.display()));
    }

    for (index, lineage) in lineages.iter().enumerate() {
        if index > 0 {
            println!();
        }

        report_lineage(handle, lineage, generation)?;
    }

    Ok(())
}

/// One lineage's champion (or `--gen=`) report, as a table.
fn report_lineage(
    handle: &PackHandle,
    workspace: &Workspace,
    generation: Option<u32>,
) -> Result<(), String> {
    let generation = match generation {
        Some(generation) => generation,
        None => workspace
            .champion()
            .map(|champion| champion.generation)
            .or_else(|| workspace.latest_gen())
            .ok_or_else(|| format!("no generations in {}", workspace.root().display()))?,
    };

    let report = workspace
        .read_report(generation)
        .ok_or_else(|| format!("gen {generation:04} has no report.json"))?;

    let champion = workspace.champion();

    println!(
        "skill: pack `{}` — gen {generation:04} [{}]{}",
        handle.pack.name(),
        report.status,
        if champion.as_ref().is_some_and(|c| c.generation == generation) { " (CHAMPION)" } else { "" },
    );

    println!(
        "skill:   workspace {} | hash {}",
        workspace.root().display(),
        report.source_hash,
    );

    if let Some(note) = &report.note {
        println!("skill:   idea: {}", ledger::truncate(note, 300));
    }

    if let Some(error) = &report.error {
        println!("skill:   error: {}", ledger::sanitize(error));
    }

    let number = |value: f64| format!("{value:.4}");

    println!("skill:   subject | status | train loss | validation rmse | hidden rmse");

    for subject in &report.per_subject {
        // Statuses are Debug-formatted evaluations ("Success { … }").
        let ok = subject.status.starts_with("Success");

        println!(
            "skill:   {:>7} | {:>6} | {:>10} | {:>17} | {:>11}",
            subject.subject,
            if ok { "ok" } else { "FAILED" },
            if ok { number(subject.train_loss) } else { "—".to_string() },
            if ok { number(subject.validation_rmse) } else { "—".to_string() },
            if ok { number(subject.hidden_rmse) } else { "—".to_string() },
        );
    }

    if let Some(fitness) = report.fitness {
        println!(
            "skill:   fitness {fitness:.4} | hidden rmse {}",
            report
                .evaluator_only
                .hidden_rmse
                .map(number)
                .unwrap_or_else(|| "—".to_string()),
        );
    }

    for subject in &report.per_subject {
        if subject.theta.is_empty() {
            continue;
        }

        println!(
            "skill:   {} theta: {}",
            subject.subject,
            subject
                .theta
                .iter()
                .map(|(name, value)| format!("{name}={value:.3}"))
                .collect::<Vec<_>>()
                .join("  "),
        );

        for pinned in &subject.pinned {
            println!("skill:   {} sentinel: {pinned}", subject.subject);
        }

        if let Some(modulation) = subject.modulation {
            println!(
                "skill:   {} sentinel: varies {:.0}% of the data's response across \
                 the validation conditions{}",
                subject.subject,
                modulation * 100.0,
                if modulation < 0.5 { " — largely ignores the condition axis" } else { "" },
            );
        }
    }

    if let Some(score) = &report.evaluator_only.holdout {
        println!(
            "skill:   sealed holdout {} [{}]: validation rmse {:.4} | hidden rmse {:.4}",
            score.subject, score.status, score.validation_rmse, score.hidden_rmse,
        );
    }

    Ok(())
}
