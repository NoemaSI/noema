//! Noema itself, built from source inside the managed directory.
//!
//! Flow: resolve the requested ref (branch/tag/sha) to an exact commit
//! through the GitHub API, download the immutable source archive for
//! that commit, extract it to `<root>/src/app`, and build it with the
//! managed cargo. All of Noema's dependencies (modelica, vendored
//! ReloadedCode) live inside the source tree, so one archive is all it
//! takes. The resulting binaries are copied to `<root>/noema/bin/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::component::{Ctx, managed_binary};
use crate::detect::Target;
use crate::download::{fetch_dynamic, sha256_hex};
use crate::manifest::{NOEMA_BUILD_PACKAGES, NOEMA_REPO, self};
use crate::prefix;
use crate::state::{Installed, State, now_secs};

/// Sentinel pinned version meaning "accept whatever is installed"
/// (used by `verify` when the user did not name a ref).
pub const ANY_VERSION: &str = "*";

pub fn src_dir(root: &Path) -> PathBuf {
    root.join("src")
}

pub fn app_dir(root: &Path) -> PathBuf {
    root.join("src").join("app")
}

pub fn noema_bin(root: &Path) -> PathBuf {
    root.join("noema").join("bin")
}

/// Resolve a branch, tag or sha to an exact 40-char commit sha, so a
/// `main` build is still reproducible and idempotent.
pub fn resolve_ref(fetcher: &dyn crate::download::Fetcher, repo: &str, ref_name: &str) -> Result<String> {
    let is_sha = ref_name.len() == 40 && ref_name.bytes().all(|b| b.is_ascii_hexdigit());
    if is_sha {
        return Ok(ref_name.to_lowercase());
    }
    let url = manifest::commit_api_url(repo, ref_name);
    let body = fetcher.fetch(&url).with_context(|| {
        format!("resolving ref '{ref_name}' of {repo} — is the repo private? set NOEMA_SETUP_TOKEN")
    })?;
    let json: serde_json::Value = serde_json::from_slice(&body)
        .with_context(|| format!("parsing commit metadata from {url}"))?;
    let sha = json
        .get("sha")
        .and_then(|v| v.as_str())
        .context("commit metadata has no 'sha'")?
        .to_string();
    anyhow::ensure!(
        sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "resolved sha {sha:?} is not a commit sha"
    );
    Ok(sha.to_lowercase())
}

pub struct NoemaComponent {
    pub repo: String,
    /// The exact commit sha to install, or [`ANY_VERSION`] to accept
    /// whatever is recorded in `state.json`.
    pub pinned: String,
    /// The ref the user asked for, for display only.
    pub requested_ref: String,
}

impl NoemaComponent {
    pub fn new(repo: Option<String>, pinned: String, requested_ref: String) -> Self {
        Self {
            repo: repo.unwrap_or_else(|| NOEMA_REPO.to_string()),
            pinned,
            requested_ref,
        }
    }
}

impl crate::component::Component for NoemaComponent {
    fn name(&self) -> &'static str {
        "noema"
    }

    fn pinned_version(&self) -> String {
        self.pinned.clone()
    }

    /// Installed commit sha, if the recorded binaries all exist. In
    /// ANY_VERSION mode any valid install matches.
    fn probe(&self, root: &Path) -> Option<String> {
        let state = State::load(root).ok()?;
        let installed = state.components.get("noema")?;
        if installed.binaries.is_empty() {
            return None;
        }
        let all_present = installed.binaries.values().all(|path| path.is_file());
        if !all_present {
            return None;
        }
        if self.pinned == ANY_VERSION {
            return Some(ANY_VERSION.to_string());
        }
        Some(installed.version.clone())
    }

    fn note(&self, _target: &Target) -> String {
        format!(
            "{} ref {} -> build {} from source",
            manifest::source_zip_url(&self.repo, &self.requested_ref),
            self.requested_ref,
            NOEMA_BUILD_PACKAGES.join(", ")
        )
    }

    fn install(&self, ctx: &Ctx) -> Result<Installed> {
        anyhow::ensure!(
            self.pinned != ANY_VERSION,
            "install requires a resolved commit sha, not ANY_VERSION"
        );

        // 1. App sources at the resolved commit (immutable URL).
        let app_url = manifest::source_zip_url(&self.repo, &self.pinned);
        let app_bytes = fetch_dynamic(ctx.fetcher, &app_url, &prefix::downloads(ctx.root))?;
        let app_sha = sha256_hex(&app_bytes);
        crate::download::extract_zip(&app_bytes, &app_dir(ctx.root), true)?;

        // 2. Build with the managed toolchain.
        let cargo = managed_binary(
            &crate::rust::cargo_bin(ctx.root),
            "cargo",
            &ctx.target,
        );
        anyhow::ensure!(
            cargo.is_file(),
            "noema needs the managed rust toolchain — run setup with the rust component first"
        );
        let mut command = Command::new(&cargo);
        command
            .current_dir(app_dir(ctx.root))
            .arg("build")
            .arg("--release");
        for package in NOEMA_BUILD_PACKAGES {
            command.arg("-p").arg(package);
        }
        command
            .env("RUSTUP_HOME", prefix::rustup_home(ctx.root))
            .env("CARGO_HOME", prefix::cargo_home(ctx.root))
            .env("RUSTUP_AUTO_UPDATE", "0")
            .env("CARGO_TERM_PROGRESS_WHEN", "never");
        let status = command
            .status()
            .with_context(|| format!("running managed cargo at {}", cargo.display()))?;
        anyhow::ensure!(status.success(), "noema source build exited with {status}");

        // 3. Install the built binaries.
        let mut binaries = BTreeMap::new();
        for package in NOEMA_BUILD_PACKAGES {
            let built = app_dir(ctx.root)
                .join("target")
                .join("release")
                .join(format!("{package}{}", ctx.target.exe_suffix()));
            anyhow::ensure!(built.is_file(), "build produced no {package} binary");
            let destination = managed_binary(&noema_bin(ctx.root), package, &ctx.target);
            crate::download::install_binary(&built, &destination)?;
            binaries.insert(package.to_string(), destination);
        }

        Ok(Installed {
            version: self.pinned.clone(),
            binaries,
            sha256: app_sha,
            installed_at: now_secs(),
        })
    }
}