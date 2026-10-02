//! The skill binary: instantiate the discovery loop for one pack.
//!
//! ```text
//! skill new     # interview: author a new pack for your own data
//! skill evolve  --pack=packs/binding/pack.rs --subjects=a,b --gens=10
//! skill report  --pack=packs/binding/pack.rs   # per-subject fit numbers
//! skill inspect --pack=packs/binding/pack.rs
//! skill plot    --pack=packs/<name>/pack.rs [--subject=<id>] [--gen=N]
//! skill viewer  [addr]     # feature `viewer`: standalone SSE viewer page
//! ```
//!
//! `evolve` runs the full discovery loop (see [`pack::driver`]);
//! `inspect` loads the pack and prints the reality it declares, so a
//! freshly written pack can be smoke-tested before any LLM is called.
//! `evolve` also serves the viewer in-process (feature `viewer`).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let outcome = match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("skill: {message}");

            ExitCode::FAILURE
        }
    };

    outcome
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args.first().map(String::as_str).unwrap_or("");

    // The standalone viewer server needs no pack. Like the old
    // `binding viewer` it is a development aid for the page itself:
    // the broadcast channel is per-process, so a live loop's events
    // only reach the browser through the server `evolve` itself
    // starts (feature `viewer`).
    #[cfg(feature = "viewer")]
    if command == "viewer" {
        let addr = args
            .get(1)
            .cloned()
            .unwrap_or_else(pack::viewer::default_addr);

        println!("skill: viewer on http://{addr}/ (Ctrl-C to stop)");

        pack::viewer::publish("info", format!("viewer listening on {addr}"));
        pack::viewer::serve(&addr);

        std::thread::park();

        return Ok(());
    }

    let rest = &args[args.len().min(1)..];

    // Pack authoring needs no pack: it is how a pack comes to exist.
    if command == "new" {
        return pack::author::run(&args[args.len().min(1)..]);
    }

    // The interactive Plotly overlay of one generation (the old
    // `binding plot`): loads its own pack, so it joins before the
    // shared pack handling.
    if command == "plot" {
        return pack::plot::plot_command(rest);
    }

    // Law discovery needs no pack: it renders a table into a pack.
    if command == "curvecheck" {
        return pack::curvecheck::run(&args[args.len().min(1)..]).map_err(|error| format!("{error:#}"));
    }
    if command == "parampack" {
        return pack::parampack::run(&args[args.len().min(1)..]).map_err(|error| format!("{error:#}"));
    }

    let pack_path = rest
        .iter()
        .find_map(|arg| arg.strip_prefix("--pack="))
        .ok_or("usage: skill <evolve|inspect|viewer> --pack=<pack.rs> [flags...]")?;

    let handle = pack::pack_loader::load_pack(std::path::Path::new(pack_path))
        .map_err(|error| format!("{error:#}"))?;

    match command {
        "evolve" => pack::driver::run(&handle, rest),

        "report" => pack::driver::report(&handle, rest),

        "grade" => pack::grade::grade(&handle, rest),

        "export" => pack::export::export(&handle, rest).map_err(|error| format!("{error:#}")),

        "inspect" => {
            let pack = handle.pack.as_ref();

            println!("skill: pack `{}`", pack.name());
            println!("skill: problem: {}", pack.problem_statement());

            let lexicon = pack.lexicon();

            for channel in &lexicon.inputs {
                println!(
                    "skill: input channel `{}` ({}): {}",
                    channel.name, channel.units, channel.description,
                );
            }

            for channel in &lexicon.outputs {
                println!(
                    "skill: output channel `{}` ({}): {}",
                    channel.name, channel.units, channel.description,
                );
            }

            if !lexicon.roles.is_empty() {
                println!("skill: roles: {}", lexicon.roles.join(", "));
            }

            let policy = pack.split_policy();

            println!(
                "skill: split: train {:?} | validation {:?} | hidden {:?}",
                policy.train, policy.validation, policy.hidden,
            );

            let baseline = pack.baseline();

            println!(
                "skill: baseline mechanism {}",
                handle.resolve(&baseline.mechanism).display(),
            );

            if let Some(plugin) = &baseline.plugin {
                println!("skill: baseline plugin {}", handle.resolve(plugin).display());
            }

            let subjects = pack.subjects().map_err(|error| format!("{error:#}"))?;

            println!("skill: {} subjects", subjects.len());

            for subject in &subjects {
                println!(
                    "skill:   {} ({} curves)",
                    subject.id,
                    subject.curves.len(),
                );
            }

            Ok(())
        }

        _ => Err(
            "usage: skill <new|evolve|report|inspect|plot|viewer> [--pack=<pack.rs>] [flags...]\n\n\
             new subcommands: (no args) interview | survey --data=<file> \
             [--delim=,] [--json] | prepare --session=<session.json> | \
             split --session=<session.json> | \
             preview --session=<session.json> [--out=<png>] | \
             draft --session=<session.json> | generate --session=<session.json> [--force] | \
             validate --pack=<pack.rs> | smoke --pack=<pack.rs> [evolve flags]\n\n\
             plot flags: --pack=<pack.rs> [--subject=<id>] [--gen=N] \
             [--condition=a,b] [--t-end=X] [--workspace=<dir>] [--out=<file.html>]\n\n\
             report flags: --pack=<pack.rs> [--gen=N] [--subject=<id>…] [--workspace=<dir>]\n\n\
             export flags: --pack=<pack.rs> --workspace=<dir> (repeatable) [--gen=N] \
             [--covariates=<csv>] [--id-column=<col>] --out=<csv>\n\n\
             parampack flags: --table=<csv> --out-dir=<dir> --outputs=a,b,c \
             [--features=…] [--linear=…] [--ratio=70/15/15 | --train=… --validation=… \
             --hidden=…] [--name=<pack>]\n\n\
             evolve flags: --gens=N --subject=<id> (repeatable) --subjects=a,b,c \
             --holdout-subject=<id> --nfev=N --subjects-limit=N --workspace=<dir> \
             --mechanism=<file.json> --no-evolve\n\
             (subject ids: skill inspect --pack=<pack.rs>)\n\n\
             the `viewer` subcommand and the in-process viewer of `evolve` \
             require `--features viewer`"
                .to_string(),
        ),
    }
}
