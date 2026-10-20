//! Resolve the managed executables from library code.
//!
//! This is what other Noema crates should use instead of spawning
//! bare `pixi` / `rumoca` off the `PATH`:
//!
//! ```no_run
//! use installer::toolchain::Toolchain;
//!
//! let toolchain = Toolchain::discover()?;
//! let pixi = toolchain.pixi()?;
//! let mut command = std::process::Command::new(pixi);
//! command.args(["run", "python", "fit.py"]);
//! for (key, value) in toolchain.env_overlay() {
//!     command.env(key, value);
//! }
//! # Ok::<(), installer::toolchain::ToolchainError>(())
//! ```

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::detect::{self, Target};
use crate::prefix;
use crate::state::State;

#[derive(Debug, Error)]
pub enum ToolchainError {
    #[error("`{component}` is not installed in {} — run `noema-setup` first", root.display())]
    MissingComponent { component: &'static str, root: PathBuf },
    #[error("`{component}` is recorded at {path} but that file is not executable there")]
    BinaryGone { component: &'static str, path: PathBuf },
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

    pub fn pixi(&self) -> Result<PathBuf, ToolchainError> {
        self.resolve("pixi", &prefix::pixi_bin(&self.root), "pixi")
    }

    pub fn rumoca(&self) -> Result<PathBuf, ToolchainError> {
        self.resolve("rumoca", &prefix::rumoca_bin(&self.root), "rumoca")
    }

    pub fn rustc(&self) -> Result<PathBuf, ToolchainError> {
        self.resolve("rustc", &crate::rust::cargo_bin(&self.root), "rustc")
    }

    pub fn cargo(&self) -> Result<PathBuf, ToolchainError> {
        self.resolve("cargo", &crate::rust::cargo_bin(&self.root), "cargo")
    }

    pub fn rustup(&self) -> Result<PathBuf, ToolchainError> {
        self.resolve("rustup", &crate::rust::cargo_bin(&self.root), "rustup")
    }

    /// Environment overlay for spawning any managed executable:
    /// prepends the managed bin directories to `PATH` and sets
    /// `RUSTUP_HOME`/`CARGO_HOME` so the rustup shims resolve the
    /// managed toolchain no matter what the ambient environment says.
    pub fn env_overlay(&self) -> Vec<(String, OsString)> {
        let managed_bins = [
            crate::rust::cargo_bin(&self.root),
            prefix::pixi_bin(&self.root),
            prefix::rumoca_bin(&self.root),
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
        ]
    }

    /// The recorded path for a logical binary, falling back to the
    /// conventional managed location when `state.json` predates or
    /// lacks an entry. A path that does not exist is an error — the
    /// caller must not silently fall back to the system `PATH`.
    fn resolve(&self, logical: &'static str, dir: &Path, name: &str) -> Result<PathBuf, ToolchainError> {
        let candidate = self
            .components
            .get(logical)
            .and_then(|installed| installed.binaries.get(logical))
            .cloned()
            .unwrap_or_else(|| dir.join(format!("{name}{}", self.target.exe_suffix())));
        if candidate.is_file() {
            Ok(candidate)
        } else if self.components.contains_key(logical) {
            Err(ToolchainError::BinaryGone { component: logical, path: candidate })
        } else {
            Err(ToolchainError::MissingComponent { component: logical, root: self.root.clone() })
        }
    }
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
                binaries: [(logical("pixi"), ghost.clone())].into_iter().collect(),
                sha256: "x".repeat(64),
                installed_at: 0,
            },
        );
        state.save(root).unwrap();
        let toolchain = Toolchain::with_root(root).unwrap();
        let error = toolchain.pixi().unwrap_err();
        assert!(matches!(error, ToolchainError::BinaryGone { .. }), "{error}");
    }

    fn logical(name: &str) -> String {
        name.to_string()
    }

    #[test]
    fn env_overlay_sets_path_homes_and_keeps_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let toolchain = Toolchain::with_root(dir.path()).unwrap();
        let overlay = toolchain.env_overlay();
        let keys: Vec<&str> = overlay.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["PATH", "RUSTUP_HOME", "CARGO_HOME"]);
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