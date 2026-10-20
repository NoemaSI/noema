//! `state.json`: the provenance record of what the installer put into
//! the managed directory. The setup flow reconciles against it and the
//! `toolchain` resolver reads it; humans may inspect or delete it — a
//! missing entry only forces a re-verification, never a crash.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const STATE_VERSION: u32 = 1;

/// One installed component, as recorded after a successful install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    /// The pinned version that was installed.
    pub version: String,
    /// Logical name (`rustc`, `cargo`, `pixi`, …) -> absolute path.
    pub binaries: BTreeMap<String, PathBuf>,
    /// sha256 of the downloaded source asset.
    pub sha256: String,
    /// Unix timestamp (seconds) of the install.
    pub installed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct State {
    pub version: u32,
    pub components: BTreeMap<String, Installed>,
}

impl State {
    /// Load `state.json` from the toolchain root. A missing file is a
    /// fresh (empty) state; a corrupt file is an error the user can
    /// resolve by deleting it.
    pub fn load(root: &Path) -> anyhow::Result<State> {
        let path = crate::prefix::state_file(root);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parsing {} (delete it to start fresh)", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(error) => {
                Err(anyhow::Error::new(error).context(format!("reading {}", path.display())))
            }
        }
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            root != Path::new(""),
            "toolchain root must be resolved before saving state"
        );
        std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
        let path = crate::prefix::state_file(root);
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }

    pub fn record(&mut self, name: &str, installed: Installed) {
        self.version = STATE_VERSION;
        self.components.insert(name.to_string(), installed);
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = State::default();
        let mut binaries = BTreeMap::new();
        binaries.insert("pixi".to_string(), PathBuf::from("/toolchain/pixi/bin/pixi"));
        state.record(
            "pixi",
            Installed {
                version: "0.82.0".to_string(),
                binaries,
                sha256: "ab12".to_string(),
                installed_at: 1_700_000_000,
            },
        );
        state.save(dir.path()).unwrap();

        let loaded = State::load(dir.path()).unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn missing_file_is_fresh_state() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = State::load(dir.path()).unwrap();
        assert_eq!(loaded, State::default());
    }

    #[test]
    fn corrupt_file_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(crate::prefix::state_file(dir.path()), "{ nope").unwrap();
        let error = State::load(dir.path()).unwrap_err();
        assert!(error.to_string().contains("delete it to start fresh"));
    }
}