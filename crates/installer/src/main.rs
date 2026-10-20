//! `noema-setup` — the Noema toolchain bootstrapper.
//!
//! Usage:
//!   noema-setup [setup] [--prefix DIR] [--only rust,pixi,rumoca]
//!               [--dry-run] [--json]
//!   noema-setup verify  [--prefix DIR] [--only ...] [--json]
//!   noema-setup uninstall [--prefix DIR] [--yes]
//!
//! Everything is installed under one managed directory (--prefix,
//! NOEMA_TOOLCHAIN_DIR, or ~/.noema/toolchain). The installer never
//! modifies the user's PATH, shell profiles, or system directories.

use std::io::Write;

use anyhow::{Result, bail};
use installer::{download, prefix, setup};

fn usage() -> &'static str {
    "usage: noema-setup [setup|verify|uninstall] [--prefix DIR] [--only rust,pixi,rumoca,noema]\n\
     \x20                [--ref <branch|tag|sha>] [--repo owner/name] [--dry-run] [--json] [--yes]\n"
}

struct Args {
    command: String,
    prefix: Option<std::path::PathBuf>,
    only: Vec<String>,
    dry_run: bool,
    json: bool,
    yes: bool,
    noema_ref: Option<String>,
    noema_repo: Option<String>,
}

fn parse_args(argv: &[String]) -> Result<Args> {
    let mut args = Args {
        command: "setup".to_string(),
        prefix: None,
        only: Vec::new(),
        dry_run: false,
        json: false,
        yes: false,
        noema_ref: None,
        noema_repo: None,
    };
    let mut argv = argv.iter().peekable();
    if let Some(first) = argv.peek() {
        if !first.starts_with('-') {
            args.command = (*first).clone();
            argv.next();
        }
    }
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--prefix" => {
                let value = argv.next().ok_or_else(|| anyhow::anyhow!("--prefix needs a value"))?;
                args.prefix = Some(std::path::PathBuf::from(value));
            }
            "--only" => {
                let value = argv.next().ok_or_else(|| anyhow::anyhow!("--only needs a value"))?;
                args.only = value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
            }
            "--ref" => {
                args.noema_ref = Some(argv.next().cloned().ok_or_else(|| anyhow::anyhow!("--ref needs a value"))?);
            }
            "--repo" => {
                args.noema_repo = Some(argv.next().cloned().ok_or_else(|| anyhow::anyhow!("--repo needs a value"))?);
            }
            "--dry-run" => args.dry_run = true,
            "--json" => args.json = true,
            "--yes" => args.yes = true,
            "-h" | "--help" => bail!("{}", usage()),
            other => bail!("unknown argument '{other}'\n{}", usage()),
        }
    }
    if !matches!(args.command.as_str(), "setup" | "verify" | "uninstall") {
        bail!("unknown command '{}'\n{}", args.command, usage());
    }
    if args.dry_run && args.command != "setup" {
        bail!("--dry-run applies to setup only");
    }
    Ok(args)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(exit_ok) => std::process::exit(if exit_ok { 0 } else { 1 }),
        Err(error) => {
            eprintln!("noema-setup: {error:#}");
            std::process::exit(1);
        }
    }
}

fn run(argv: &[String]) -> Result<bool> {
    let args = parse_args(argv)?;
    let root = prefix::resolve(args.prefix.as_deref())?;

    match args.command.as_str() {
        "verify" => {
            let outcome = setup::verify(
                &root,
                args.json,
                &args.only,
                args.noema_ref,
                args.noema_repo,
                &download::HttpFetcher,
            )?;
            print_summary(&outcome);
            Ok(outcome.ok())
        }
        "uninstall" => {
            if !args.yes {
                eprint!("delete the managed toolchain at {}? [y/N] ", root.display());
                std::io::stderr().flush().ok();
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer).ok();
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("aborted");
                    return Ok(true);
                }
            }
            setup::uninstall(&root)?;
            eprintln!("removed {}", root.display());
            Ok(true)
        }
        _ => {
            let options = setup::Options {
                root,
                only: args.only,
                dry_run: args.dry_run,
                json: args.json,
                verification: download::Verification::Enforce,
                noema_ref: args.noema_ref,
                noema_repo: args.noema_repo,
            };
            let outcome = setup::run(&download::HttpFetcher, &options)?;
            print_summary(&outcome);
            Ok(outcome.ok())
        }
    }
}

fn print_summary(outcome: &setup::Outcome) {
    if outcome.results.is_empty() {
        return;
    }
    for (name, status) in &outcome.results {
        let line = match status {
            setup::Status::Skipped { version } => format!("{name:<8} ok       v{version}"),
            setup::Status::Installed { version } => format!("{name:<8} installed v{version}"),
            setup::Status::Unavailable { reason } => format!("{name:<8} unavailable — {reason}"),
            setup::Status::Failed { message } => format!("{name:<8} FAILED — {message}"),
        };
        eprintln!("{line}");
    }
    if outcome.ok() {
        eprintln!("the noema toolchain is ready");
    } else {
        eprintln!("setup finished with problems (see above)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn defaults_to_setup() {
        let parsed = parse_args(&args("")).unwrap();
        assert_eq!(parsed.command, "setup");
        assert!(!parsed.dry_run);
    }

    #[test]
    fn parses_all_flags() {
        let parsed = parse_args(&args("verify --prefix /tmp/t --only pixi,rumoca --json")).unwrap();
        assert_eq!(parsed.command, "verify");
        assert_eq!(parsed.prefix.unwrap(), std::path::Path::new("/tmp/t"));
        assert_eq!(parsed.only, ["pixi", "rumoca"]);
        assert!(parsed.json);
    }

    #[test]
    fn parses_noema_source_flags() {
        let parsed = parse_args(&args("setup --ref v1.2.0 --repo Some/Repo")).unwrap();
        assert_eq!(parsed.noema_ref.as_deref(), Some("v1.2.0"));
        assert_eq!(parsed.noema_repo.as_deref(), Some("Some/Repo"));
        assert!(parse_args(&args("setup --ref")).is_err());
    }

    #[test]
    fn rejects_unknown_things() {
        assert!(parse_args(&args("frobnicate")).is_err());
        assert!(parse_args(&args("--wat")).is_err());
        assert!(parse_args(&args("verify --dry-run")).is_err());
        assert!(parse_args(&args("setup --prefix")).is_err());
    }
}