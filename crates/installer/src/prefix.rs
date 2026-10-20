//! Resolution of the managed toolchain root directory.
//!
//! Resolution order:
//! 1. the explicit `--prefix` flag,
//! 2. the `NOEMA_TOOLCHAIN_DIR` environment variable,
//! 3. `~/.noema/toolchain`.
//!
//! Every byte the installer writes lives under this root — that is
//! what makes the whole setup hermetic and container-testable.

use std::path::{Path, PathBuf};

use anyhow::Context;

pub const ENV_KEY_TOOLCHAIN_DIR: &str = "NOEMA_TOOLCHAIN_DIR";

/// Resolve the toolchain root from an optional explicit prefix and an
/// optional environment value. Pure function so callers (and tests)
/// control both inputs.
pub fn resolve_with(prefix: Option<&Path>, env_value: Option<&str>) -> anyhow::Result<PathBuf> {
    if let Some(prefix) = prefix {
        return Ok(prefix.to_path_buf());
    }
    if let Some(value) = env_value {
        anyhow::ensure!(!value.is_empty(), "{ENV_KEY_TOOLCHAIN_DIR} is set but empty");
        return Ok(PathBuf::from(value));
    }
    let home = dirs::home_dir().context("no home directory found; pass --prefix or set NOEMA_TOOLCHAIN_DIR")?;
    Ok(home.join(".noema").join("toolchain"))
}

/// Resolve the toolchain root from the process environment.
pub fn resolve(prefix: Option<&Path>) -> anyhow::Result<PathBuf> {
    let env_value = std::env::var_os(ENV_KEY_TOOLCHAIN_DIR);
    let env_value = env_value.as_deref().and_then(|v| v.to_str());
    resolve_with(prefix, env_value)
}

/// `RUSTUP_HOME` inside the managed root.
pub fn rustup_home(root: &Path) -> PathBuf {
    root.join("rust").join("rustup")
}

/// `CARGO_HOME` inside the managed root. Rustup's shims (rustc, cargo,
/// rustup) live in its `bin/` directory.
pub fn cargo_home(root: &Path) -> PathBuf {
    root.join("rust").join("cargo")
}

/// Directory holding the managed `pixi` executable.
pub fn pixi_bin(root: &Path) -> PathBuf {
    root.join("pixi").join("bin")
}

/// Directory holding the managed `rumoca` executable.
pub fn rumoca_bin(root: &Path) -> PathBuf {
    root.join("rumoca").join("bin")
}

/// Download cache; populated during setup, pruned when a full setup
/// succeeds.
pub fn downloads(root: &Path) -> PathBuf {
    root.join("downloads")
}

/// Path of the `state.json` provenance file.
pub fn state_file(root: &Path) -> PathBuf {
    root.join("state.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_prefix_wins() {
        let resolved = resolve_with(Some(Path::new("/tmp/mine")), Some("/tmp/from-env")).unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/mine"));
    }

    #[test]
    fn env_beats_default() {
        let resolved = resolve_with(None, Some("/toolchain")).unwrap();
        assert_eq!(resolved, PathBuf::from("/toolchain"));
    }

    #[test]
    fn empty_env_is_an_error() {
        assert!(resolve_with(None, Some("")).is_err());
    }

    #[test]
    fn default_is_under_home() {
        let resolved = resolve_with(None, None).unwrap();
        assert!(resolved.ends_with(Path::new(".noema").join("toolchain")));
    }
}