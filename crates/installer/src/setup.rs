//! Setup orchestration shared by the `noema-setup` binary and tests.
//!
//! `--dry-run` is the same code path minus the install arm: it loads
//! state, probes the machine and prints the full action plan without
//! writing or downloading a single byte.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::component::{self, Action, Component, Ctx};
use crate::detect::{self, Target};
use crate::download::{self, Fetcher};
use crate::pixi::PixiComponent;
use crate::prefix;
use crate::prereqs;
use crate::report::Report;
use crate::rumoca::RumocaComponent;
use crate::rust::RustupComponent;
use crate::state::{State, now_secs};

pub const COMPONENT_NAMES: [&str; 3] = ["rust", "pixi", "rumoca"];

pub fn components() -> Vec<Box<dyn Component>> {
    vec![Box::new(RustupComponent), Box::new(PixiComponent), Box::new(RumocaComponent)]
}

pub struct Options {
    pub root: PathBuf,
    /// Restrict to these components (validated against COMPONENT_NAMES).
    pub only: Vec<String>,
    pub dry_run: bool,
    pub json: bool,
    /// The `noema-setup` binary always uses `Enforce`.
    pub verification: download::Verification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Skipped { version: String },
    Installed { version: String },
    Unavailable { reason: String },
    Failed { message: String },
}

#[derive(Debug)]
pub struct Outcome {
    pub target: Target,
    pub results: Vec<(String, Status)>,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.results.iter().all(|(_, status)| {
            matches!(status, Status::Skipped { .. } | Status::Installed { .. })
        })
    }
}

pub fn run(fetcher: &dyn Fetcher, options: &Options) -> Result<Outcome> {
    let report = Report::new(options.json);
    let target = detect::host()?;
    let root = &options.root;

    if options.dry_run {
        report.note(&format!("dry run — nothing will be downloaded or written"));
    }
    report.note(&format!("target:   {}", target.triple));
    report.note(&format!("root:     {}", root.display()));

    let prereqs = prereqs::check_native_build(&target);
    for note in &prereqs.notes {
        report.note(&format!("prereq:   {note}"));
    }
    if !prereqs.ok {
        report.note("prereq:   WARNING — noema cannot compile plugin code without a native toolchain");
    }

    let mut state = State::load(root)?;
    let mut results = Vec::new();
    let mut requested = Vec::new();

    for component in components() {
        if !options.only.is_empty() && !options.only.iter().any(|n| n == component.name()) {
            continue;
        }
        requested.push(component.name());
        let action = component::decide(component.as_ref(), root, &target);
        let (status, failed) = match action {
            Action::Skip { version } => {
                report.step(component.name(), "skip", &format!("already installed v{version}"));
                (Status::Skipped { version }, false)
            }
            Action::Unavailable { reason } => {
                report.step(component.name(), "unavailable", &reason);
                (Status::Unavailable { reason }, true)
            }
            Action::Install { version, note } => {
                if options.dry_run {
                    report.step(component.name(), "install (dry run)", &format!("v{version} — {note}"));
                    (Status::Skipped { version }, false)
                } else {
                    report.step(component.name(), "install", &note);
                    let ctx = Ctx { root, target, fetcher, verification: options.verification };
                    match component.install(&ctx) {
                        Ok(mut installed) => {
                            if installed.installed_at == 0 {
                                installed.installed_at = now_secs();
                            }
                            state.record(component.name(), installed.clone());
                            state.save(root)?;
                            report.done(component.name(), &installed.version);
                            (Status::Installed { version: installed.version }, false)
                        }
                        Err(error) => {
                            report.failure(component.name(), &error);
                            (Status::Failed { message: format!("{error:#}") }, true)
                        }
                    }
                }
            }
        };
        let _ = failed;
        results.push((component.name().to_string(), status));
    }

    validate_only(&options.only, &requested)?;

    // Only a fully successful real run may discard the download cache.
    if !options.dry_run && results.iter().all(|(_, s)| matches!(s, Status::Skipped { .. } | Status::Installed { .. })) {
        download::prune_cache(&prefix::downloads(root))?;
    }

    Ok(Outcome { target, results })
}

fn validate_only(only: &[String], requested: &[&'static str]) -> Result<()> {
    for name in only {
        anyhow::ensure!(
            COMPONENT_NAMES.contains(&name.as_str()),
            "unknown component '{name}'; choose from {}",
            COMPONENT_NAMES.join(", ")
        );
    }
    anyhow::ensure!(!requested.is_empty(), "--only matched no components");
    Ok(())
}

/// Probe every managed component and report whether it matches the
/// pinned version. Exits nonzero (via the caller) when anything is
/// missing or stale.
pub fn verify(root: &Path, json: bool, only: &[String]) -> Result<Outcome> {
    let report = Report::new(json);
    let target = detect::host()?;
    report.note(&format!("root:     {}", root.display()));
    let mut results = Vec::new();
    for component in components() {
        if !only.is_empty() && !only.iter().any(|n| n == component.name()) {
            continue;
        }
        let status = match component::decide(component.as_ref(), root, &target) {
            Action::Skip { version } => {
                report.done(component.name(), &version);
                Status::Skipped { version }
            }
            Action::Install { version, note } => {
                report.step(component.name(), "stale or missing", &format!("want v{version} — {note}"));
                Status::Failed { message: "not installed at pinned version".to_string() }
            }
            Action::Unavailable { reason } => {
                report.step(component.name(), "unavailable", &reason);
                Status::Unavailable { reason }
            }
        };
        results.push((component.name().to_string(), status));
    }
    Ok(Outcome { target, results })
}

/// Delete the managed toolchain directory.
pub fn uninstall(root: &Path) -> Result<()> {
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(anyhow::Error::new(error).context(format!("removing {}", root.display())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::sha256_hex;

    /// A fetcher that serves fake binaries which report the pinned
    /// versions, so the full flow runs offline. For rustup-init it
    /// serves a shell script that fabricates the shim layout rustup
    /// would leave behind.
    struct FakeToolchainFetcher {
        scripts: Vec<(&'static str, Vec<u8>)>,
    }

    impl FakeToolchainFetcher {
        fn new() -> Self {
            let fake = |name: &str, version: &str| -> Vec<u8> {
                format!("#!/bin/sh\necho \"{name} {version}\"\n").into_bytes()
            };
            let rustup_init = br#"#!/bin/sh
# Minimal rustup-init stand-in: creates the shim layout and exits.
test "$1" = "-y" || exit 2
mkdir -p "$CARGO_HOME/bin"
for tool in rustup rustc cargo; do
    printf '#!/bin/sh\necho "rustc 1.89.0 (faked 2025-08-14)"\n' > "$CARGO_HOME/bin/$tool"
    chmod +x "$CARGO_HOME/bin/$tool"
done
exit 0
"#.to_vec();
            Self {
                scripts: vec![
                    ("rustup-init", rustup_init),
                    ("pixi-", fake("pixi", "0.82.0")),
                    ("rumoca-", fake("rumoca", "0.9.20")),
                ],
            }
        }
    }

    impl Fetcher for FakeToolchainFetcher {
        fn fetch(&self, url: &str) -> Result<Vec<u8>> {
            for (needle, body) in &self.scripts {
                if url.contains(needle) {
                    return Ok(body.clone());
                }
            }
            anyhow::bail!("unexpected url {url}")
        }
    }

    fn options(root: &Path) -> Options {
        Options {
            root: root.to_path_buf(),
            only: vec![],
            dry_run: false,
            json: false,
            // Tests serve synthetic stub binaries; the CLI enforces.
            verification: download::Verification::TrustFetcher,
        }
    }

    fn require_host_supported() -> Option<Target> {
        detect::host().ok()
    }

    #[cfg(unix)]
    #[test]
    fn full_setup_installs_records_state_and_skips_on_rerun() {
        let Some(target) = require_host_supported() else { return };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("toolchain");

        let outcome = run(&FakeToolchainFetcher::new(), &options(&root)).unwrap();
        assert!(outcome.ok(), "{:?}", outcome.results);
        assert_eq!(outcome.results.len(), 3);

        // state.json records all three components at the pinned versions.
        let state = State::load(&root).unwrap();
        assert_eq!(state.components["rust"].version, crate::manifest::RUST_TOOLCHAIN);
        assert_eq!(state.components["pixi"].version, crate::manifest::PIXI_VERSION);
        assert_eq!(state.components["rumoca"].version, crate::manifest::RUMOCA_VERSION);

        // The binaries exist where the resolver expects them.
        assert!(crate::rust::cargo_bin(&root).join("rustc").is_file());
        assert!(prefix::pixi_bin(&root).join("pixi").is_file());
        assert!(prefix::rumoca_bin(&root).join("rumoca").is_file());

        // Download cache pruned after the successful run.
        assert!(!prefix::downloads(&root).exists());

        // Re-running is pure verification: everything skips.
        let rerun = run(&FakeToolchainFetcher::new(), &options(&root)).unwrap();
        assert!(rerun.ok());
        for (name, status) in &rerun.results {
            assert!(matches!(status, Status::Skipped { .. }), "{name} should skip");
        }

        // verify agrees.
        let checked = verify(&root, false, &[]).unwrap();
        assert!(checked.ok());
        let _ = target;
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_writes_nothing() {
        let Some(_target) = require_host_supported() else { return };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("toolchain");

        struct ExplodingFetcher;
        impl Fetcher for ExplodingFetcher {
            fn fetch(&self, url: &str) -> Result<Vec<u8>> {
                panic!("dry run must not fetch {url}")
            }
        }

        let mut opts = options(&root);
        opts.dry_run = true;
        let outcome = run(&ExplodingFetcher, &opts).unwrap();
        assert_eq!(outcome.results.len(), 3);
        assert!(!root.exists(), "dry run must not create the toolchain root");
    }

    #[cfg(unix)]
    #[test]
    fn only_restricts_components_and_validates_names() {
        let Some(_target) = require_host_supported() else { return };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("toolchain");

        let mut opts = options(&root);
        opts.only = vec!["pixi".to_string()];
        let outcome = run(&FakeToolchainFetcher::new(), &opts).unwrap();
        assert_eq!(outcome.results.len(), 1);
        assert_eq!(outcome.results[0].0, "pixi");
        assert!(prefix::pixi_bin(&root).join("pixi").is_file());
        assert!(!prefix::rumoca_bin(&root).exists());

        let mut opts = options(&root);
        opts.only = vec!["nope".to_string()];
        let error = run(&FakeToolchainFetcher::new(), &opts).unwrap_err();
        assert!(error.to_string().contains("unknown component"));
    }

    #[cfg(unix)]
    #[test]
    fn stale_installation_is_upgraded() {
        let Some(_target) = require_host_supported() else { return };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("toolchain");

        // Seed a stale pixi at the managed location.
        let stale = prefix::pixi_bin(&root).join("pixi");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::write(&stale, "#!/bin/sh\necho \"pixi 0.1.0\"\n").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&stale, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let outcome = run(&FakeToolchainFetcher::new(), &options(&root)).unwrap();
        assert!(outcome.ok());
        let state = State::load(&root).unwrap();
        assert_eq!(state.components["pixi"].version, crate::manifest::PIXI_VERSION);
        assert_eq!(
            std::fs::read_to_string(&stale).unwrap(),
            "#!/bin/sh\necho \"pixi 0.82.0\"\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn checksum_mismatch_fails_the_component_without_installing() {
        let Some(_target) = require_host_supported() else { return };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("toolchain");

        struct BadBytes;
        impl Fetcher for BadBytes {
            fn fetch(&self, _url: &str) -> Result<Vec<u8>> {
                Ok(b"tampered".to_vec())
            }
        }

        let mut opts = options(&root);
        opts.only = vec!["pixi".to_string()];
        // The checksum-mismatch test asserts production behaviour.
        opts.verification = download::Verification::Enforce;
        let outcome = run(&BadBytes, &opts).unwrap();
        assert!(!outcome.ok());
        assert!(matches!(outcome.results[0].1, Status::Failed { .. }));
        assert!(!prefix::pixi_bin(&root).join("pixi").exists());
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_removes_everything() {
        let Some(_target) = require_host_supported() else { return };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("toolchain");
        run(&FakeToolchainFetcher::new(), &options(&root)).unwrap();
        assert!(root.exists());
        uninstall(&root).unwrap();
        assert!(!root.exists());
        // Idempotent.
        uninstall(&root).unwrap();
    }

    #[test]
    fn sha_helper_sanity() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}