//! The component abstraction the setup flow runs on.
//!
//! Each component knows how to probe its own installed version and how
//! to install itself into the managed root. The [`decide`] function
//! turns the current state into a `Skip` / `Install` decision, which
//! both the real setup run and `--dry-run` consume — dry-run simply
//! never gets past the decisions.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::detect::Target;
use crate::download::Fetcher;
use crate::state::Installed;

/// What setup would do (or does) for one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The managed binary already reports the pinned version.
    Skip { version: String },
    /// Fetch and install the pinned version.
    Install { version: String, note: String },
    /// No pinned asset exists for this host.
    Unavailable { reason: String },
}

pub struct Ctx<'a> {
    pub root: &'a Path,
    pub target: Target,
    pub fetcher: &'a dyn Fetcher,
    /// The CLI always uses `Enforce`; only in-crate tests pass
    /// `TrustFetcher` for their synthetic stub binaries.
    pub verification: crate::download::Verification,
}

impl Ctx<'_> {
    pub fn exe(&self, name: &str) -> String {
        format!("{name}{}", self.target.exe_suffix())
    }
}

pub trait Component {
    fn name(&self) -> &'static str;

    /// The version the installer targets. For the source-built noema
    /// component this is a resolved commit sha, not a static pin.
    fn pinned_version(&self) -> String;

    /// Installed version inside `root`, if the managed binary exists
    /// and reports a parseable version.
    fn probe(&self, root: &Path) -> Option<String>;

    /// Human-readable note for the install plan (usually the source
    /// URL or a reason for unavailability).
    fn note(&self, target: &Target) -> String;

    /// Install into the managed root and return the provenance record.
    fn install(&self, ctx: &Ctx) -> Result<Installed>;
}

/// Decide what setup should do for one component: skip when the
/// installed binary already reports the pinned version, install
/// otherwise.
pub fn decide(component: &dyn Component, root: &Path, target: &Target) -> Action {
    let note = component.note(target);
    if note.is_empty() {
        return Action::Unavailable { reason: format!("no pinned {} asset for {}", component.name(), target.triple) };
    }
    match component.probe(root) {
        Some(version) if version == component.pinned_version() => Action::Skip { version },
        Some(installed) => Action::Install {
            version: component.pinned_version().to_string(),
            note: format!("replacing installed {installed} <- {note}"),
        },
        None => Action::Install {
            version: component.pinned_version().to_string(),
            note: format!("installing <- {note}"),
        },
    }
}

/// Absolute path of a managed binary: `<dir>/<name>[.exe]`.
pub fn managed_binary(dir: &Path, name: &str, target: &Target) -> PathBuf {
    dir.join(format!("{name}{}", target.exe_suffix()))
}

/// Install flow shared by the single-binary components (pixi, rumoca):
/// fetch the pinned asset, verify it, drop it into `<dir>/<name>`,
/// then probe the installed binary to confirm the pin.
pub fn install_single_binary(
    ctx: &Ctx,
    asset: &crate::manifest::Asset,
    dir: &Path,
    name: &str,
    pinned_version: &str,
) -> anyhow::Result<Installed> {
    let source = crate::download::fetch_cached(
        ctx.fetcher,
        asset,
        &crate::prefix::downloads(ctx.root),
        ctx.verification,
    )?;
    let destination = managed_binary(dir, name, &ctx.target);
    crate::download::install_binary(&source, &destination)?;

    let probed = crate::detect::probe_output(&destination)
        .and_then(|output| crate::detect::parse_version(&output, name));
    match probed.as_deref() {
        Some(v) if v == pinned_version => {}
        Some(v) => anyhow::bail!(
            "installed {name} at {} reports version {v}, expected {pinned_version}",
            destination.display()
        ),
        None => anyhow::bail!(
            "installed {name} at {} does not report a version",
            destination.display()
        ),
    }

    let mut binaries = std::collections::BTreeMap::new();
    binaries.insert(name.to_string(), destination);
    Ok(Installed {
        version: pinned_version.to_string(),
        binaries,
        sha256: asset.sha256.to_string(),
        installed_at: crate::state::now_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Fake {
        installed: Option<String>,
        available: bool,
    }

    impl Component for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn pinned_version(&self) -> String {
            "9.9.9".to_string()
        }
        fn probe(&self, _root: &Path) -> Option<String> {
            self.installed.clone()
        }
        fn note(&self, _target: &Target) -> String {
            if self.available { "https://example.invalid/fake".to_string() } else { String::new() }
        }
        fn install(&self, _ctx: &Ctx) -> Result<Installed> {
            Ok(Installed {
                version: self.pinned_version().to_string(),
                binaries: BTreeMap::new(),
                sha256: "x".repeat(64),
                installed_at: 0,
            })
        }
    }

    fn target() -> Target {
        Target { os: "linux", arch: "x86_64", triple: "x86_64-unknown-linux-gnu" }
    }

    #[test]
    fn matching_version_skips() {
        let fake = Fake { installed: Some("9.9.9".into()), available: true };
        assert_eq!(decide(&fake, Path::new("/r"), &target()), Action::Skip { version: "9.9.9".into() });
    }

    #[test]
    fn stale_version_installs_with_note() {
        let fake = Fake { installed: Some("1.0.0".into()), available: true };
        let Action::Install { version, note } = decide(&fake, Path::new("/r"), &target()) else {
            panic!("expected install");
        };
        assert_eq!(version, "9.9.9");
        assert!(note.contains("replacing installed 1.0.0"));
    }

    #[test]
    fn absent_version_installs() {
        let fake = Fake { installed: None, available: true };
        assert!(matches!(decide(&fake, Path::new("/r"), &target()), Action::Install { .. }));
    }

    #[test]
    fn missing_asset_is_unavailable() {
        let fake = Fake { installed: None, available: false };
        assert!(matches!(decide(&fake, Path::new("/r"), &target()), Action::Unavailable { .. }));
    }
}