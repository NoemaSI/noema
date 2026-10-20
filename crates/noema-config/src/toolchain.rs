//! Resolve the managed executables from library code.
//!
//! This is what the Noema app uses instead of spawning bare `pixi` /
//! `rumoca` off the `PATH`. The one-liner for any spawn site:
//!
//! ```no_run
//! let mut command = noema_config::toolchain::command("pixi");
//! command.args(["run", "python", "fit.py"]);
//! ```
//!
//! When the managed toolchain is installed, the command runs its
//! absolute binary path with the environment overlay (PATH,
//! `RUSTUP_HOME`, `CARGO_HOME`, …). On a development machine without
//! one, the command is left as a plain `PATH` lookup and a one-time
//! warning is printed.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use thiserror::Error;

use crate::detect::{self, Target};
use crate::prefix;
use crate::state::State;

#[derive(Debug, Error)]
pub enum ToolchainError {
    #[error("`{component}` is not installed in {} — run `noema-setup` first", root.display())]
    MissingComponent { component: String, root: PathBuf },
    #[error("`{component}` is recorded at {path} but that file is not executable there")]
    BinaryGone { component: String, path: PathBuf },
    #[error("unknown managed binary '{0}'")]
    UnknownBinary(String),
    #[error(transparent)]
    State(#[from] anyhow::Error),
}

/// A discovered, installed Noema toolchain.
#[derive(Debug)]
pub struct Toolchain {
    pub root: PathBuf,
    target: Target,
    components: BTreeMap<String, crate::state::Installed>,
}

impl Toolchain {
    /// Discover the toolchain at the default resolution order
    /// (`NOEMA_TOOLCHAIN_DIR`, then `~/.noema/toolchain`).
    pub fn discover() -> Result<Toolchain, ToolchainError> {
        let root = prefix::resolve(None).map_err(ToolchainError::State)?;
        Self::with_root(root)
    }

    /// Load the toolchain rooted at an explicit directory (the
    /// `--prefix` equivalent for library users).
    pub fn with_root(root: impl AsRef<Path>) -> Result<Toolchain, ToolchainError> {
        let root = root.as_ref().to_path_buf();
        let target = detect::host().map_err(ToolchainError::State)?;
        let state = State::load(&root).map_err(ToolchainError::State)?;
        Ok(Toolchain { root, target, components: state.components })
    }

    /// Absolute path of a managed logical binary: `pixi`, `rumoca`,
    /// `rustc`, `cargo`, `rustup`, `ui`, `pack`.
    pub fn binary(&self, name: &str) -> Result<PathBuf, ToolchainError> {
        let dir = match name {
            "pixi" => prefix::pixi_bin(&self.root),
            "rumoca" => prefix::rumoca_bin(&self.root),
            "rustc" | "cargo" | "rustup" => prefix::cargo_bin(&self.root),
            "ui" | "pack" => prefix::noema_bin(&self.root),
            other => return Err(ToolchainError::UnknownBinary(other.to_string())),
        };
        self.resolve(name, &dir, name)
    }

    pub fn pixi(&self) -> Result<PathBuf, ToolchainError> {
        self.binary("pixi")
    }

    pub fn rumoca(&self) -> Result<PathBuf, ToolchainError> {
        self.binary("rumoca")
    }

    pub fn rustc(&self) -> Result<PathBuf, ToolchainError> {
        self.binary("rustc")
    }

    pub fn cargo(&self) -> Result<PathBuf, ToolchainError> {
        self.binary("cargo")
    }

    pub fn rustup(&self) -> Result<PathBuf, ToolchainError> {
        self.binary("rustup")
    }

    /// The built `ui` binary from the managed noema install.
    pub fn ui(&self) -> Result<PathBuf, ToolchainError> {
        self.binary("ui")
    }

    /// The built `pack` binary from the managed noema install.
    pub fn pack(&self) -> Result<PathBuf, ToolchainError> {
        self.binary("pack")
    }

    /// Environment overlay for spawning any managed executable:
    /// prepends the managed bin directories to `PATH` and sets
    /// `RUSTUP_HOME`/`CARGO_HOME` so the rustup shims resolve the
    /// managed toolchain no matter what the ambient environment says.
    /// Also the environment Rumoca needs to drive `cargo` internally.
    pub fn env_overlay(&self) -> Vec<(String, OsString)> {
        let managed_bins = [
            prefix::cargo_bin(&self.root),
            prefix::pixi_bin(&self.root),
            prefix::rumoca_bin(&self.root),
            prefix::noema_bin(&self.root),
        ];
        let mut path = OsString::new();
        for (i, dir) in managed_bins.iter().enumerate() {
            if i > 0 {
                path.push(OsStr::new(if self.target.is_windows() { ";" } else { ":" }));
            }
            path.push(dir);
        }
        if let Some(existing) = std::env::var_os("PATH") {
            path.push(OsStr::new(if self.target.is_windows() { ";" } else { ":" }));
            path.push(existing);
        }
        vec![
            ("PATH".to_string(), path),
            ("RUSTUP_HOME".to_string(), prefix::rustup_home(&self.root).into_os_string()),
            ("CARGO_HOME".to_string(), prefix::cargo_home(&self.root).into_os_string()),
            // The managed toolchain is pinned; rustup must never
            // self-update beneath it.
            ("RUSTUP_AUTO_UPDATE".to_string(), OsString::from("0")),
        ]
    }

    /// Apply the environment overlay to `command` (use together with a
/// program obtained from [`Toolchain::binary`]).
pub fn apply_env(&self, command: &mut Command) {
        for (key, value) in self.env_overlay() {
            command.env(key, value);
        }
    }

    /// The recorded path for a logical binary, falling back to the
    /// conventional managed location when `state.json` predates or
    /// lacks an entry. A path that does not exist is an error — the
    /// caller must not silently fall back to the system `PATH`.
    fn resolve(&self, logical: &str, dir: &Path, name: &str) -> Result<PathBuf, ToolchainError> {
        let candidate = self
            .components
            .get(logical)
            .and_then(|installed| installed.binaries.get(logical))
            .cloned()
            .unwrap_or_else(|| dir.join(format!("{name}{}", self.target.exe_suffix())));
        if candidate.is_file() {
            Ok(candidate)
        } else if self.components.contains_key(logical) {
            Err(ToolchainError::BinaryGone { component: logical.to_string(), path: candidate })
        } else {
            Err(ToolchainError::MissingComponent {
                component: logical.to_string(),
                root: self.root.clone(),
            })
        }
    }
}

/// Create a [`Command`] for a managed logical binary (`pixi`,
/// `rumoca`, …) with the environment overlay applied. Without an
/// installation the command falls back to the bare name (plain `PATH`
/// lookup) so development machines keep working; the reason is
/// printed once per process to keep probe loops quiet.
///
/// ```no_run
/// let mut command = noema_config::toolchain::command("pixi");
/// command.args(["run", "python", "fit.py"]);
/// ```
pub fn command(name: &str) -> Command {
    match Toolchain::discover() {
        Ok(toolchain) => match toolchain.binary(name) {
            Ok(path) => {
                let mut command = Command::new(path);
                toolchain.apply_env(&mut command);
                command
            }
            Err(error) => {
                warn_fallback(name, &error);
                Command::new(name)
            }
        },
        Err(error) => {
            warn_fallback(name, &error);
            Command::new(name)
        }
    }
}

fn warn_fallback(name: &str, error: &ToolchainError) {
    static WARNED: OnceLock<()> = OnceLock::new();
    WARNED.get_or_init(|| {
        eprintln!(
            "noema: managed toolchain unavailable ({error}); falling back to `{name}` on PATH"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_component_reports_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        let toolchain = Toolchain::with_root(dir.path()).unwrap();
        let error = toolchain.pixi().unwrap_err();
        assert!(error.to_string().contains("run `noema-setup` first"));
    }

    #[test]
    fn unknown_binary_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let toolchain = Toolchain::with_root(dir.path()).unwrap();
        assert!(matches!(
            toolchain.binary("vim"),
            Err(ToolchainError::UnknownBinary(_))
        ));
    }

    #[test]
    fn recorded_and_fallback_paths_both_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pixi = prefix::pixi_bin(root).join("pixi");
        std::fs::create_dir_all(pixi.parent().unwrap()).unwrap();
        std::fs::write(&pixi, b"#!/bin/sh\n").unwrap();

        // No state entry: the conventional path resolves.
        let toolchain = Toolchain::with_root(root).unwrap();
        assert_eq!(toolchain.pixi().unwrap(), pixi);

        // A state entry pointing elsewhere that no longer exists is an
        // error, never a silent fallback.
        let mut state = State::default();
        let ghost = root.join("deleted-pixi");
        state.record(
            "pixi",
            crate::state::Installed {
                version: "0.82.0".into(),
                binaries: [("pixi".to_string(), ghost.clone())].into_iter().collect(),
                sha256: "x".repeat(64),
                installed_at: 0,
            },
        );
        state.save(root).unwrap();
        let toolchain = Toolchain::with_root(root).unwrap();
        let error = toolchain.pixi().unwrap_err();
        assert!(matches!(error, ToolchainError::BinaryGone { .. }), "{error}");
    }

    #[test]
    fn apply_env_overlays_managed_environment() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pixi = prefix::pixi_bin(root).join("pixi");
        std::fs::create_dir_all(pixi.parent().unwrap()).unwrap();
        std::fs::write(&pixi, b"#!/bin/sh\n").unwrap();

        let toolchain = Toolchain::with_root(root).unwrap();
        let mut command = Command::new(toolchain.pixi().unwrap());
        toolchain.apply_env(&mut command);
        assert_eq!(command.get_program(), pixi);
        let env: Vec<(String, String)> = command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((key.to_string_lossy().into_owned(), value?.to_string_lossy().into_owned()))
            })
            .collect();
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"RUSTUP_HOME"), "{keys:?}");
        assert!(keys.contains(&"RUSTUP_AUTO_UPDATE"), "{keys:?}");
    }

    #[test]
    fn env_overlay_sets_path_homes_and_keeps_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let toolchain = Toolchain::with_root(dir.path()).unwrap();
        let overlay = toolchain.env_overlay();
        let keys: Vec<&str> = overlay.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["PATH", "RUSTUP_HOME", "CARGO_HOME", "RUSTUP_AUTO_UPDATE"]);
        let path = overlay
            .iter()
            .find(|(k, _)| k == "PATH")
            .unwrap()
            .1
            .to_string_lossy()
            .into_owned();
        assert!(path.contains(&prefix::pixi_bin(dir.path()).display().to_string()));
        if let Some(existing) = std::env::var_os("PATH") {
            assert!(path.contains(&existing.to_string_lossy().into_owned()));
        }
    }
}