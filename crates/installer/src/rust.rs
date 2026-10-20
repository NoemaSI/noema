//! The managed Rust toolchain, installed through rustup.
//!
//! `rustup-init` is downloaded from static.rust-lang.org, checksummed,
//! and run with `RUSTUP_HOME`/`CARGO_HOME` pointed inside the managed
//! root and `--no-modify-path`, so the user's environment is never
//! touched. The resulting `rustc`/`cargo` shims live in
//! `<root>/rust/cargo/bin/` and need `RUSTUP_HOME` set when invoked.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::component::{Ctx, managed_binary};
use crate::detect::{Target, parse_version};
use crate::download::{fetch_cached, install_binary};
use crate::manifest::{self, RUST_TOOLCHAIN};
use crate::prefix;
use crate::state::Installed;

pub struct RustupComponent;

pub fn cargo_bin(root: &Path) -> PathBuf {
    prefix::cargo_home(root).join("bin")
}

fn run_rustc_version(root: &Path, target: &Target) -> Option<String> {
    let rustc = managed_binary(&cargo_bin(root), "rustc", target);
    let output = Command::new(&rustc)
        .arg("--version")
        .env("RUSTUP_HOME", prefix::rustup_home(root))
        .env("CARGO_HOME", prefix::cargo_home(root))
        .env("RUSTUP_AUTO_UPDATE", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&output.stdout), "rustc")
}

impl super::component::Component for RustupComponent {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn pinned_version(&self) -> &'static str {
        RUST_TOOLCHAIN
    }

    fn probe(&self, root: &Path) -> Option<String> {
        let target = crate::detect::host().ok()?;
        run_rustc_version(root, &target)
    }

    fn note(&self, target: &Target) -> String {
        manifest::rustup_init(target)
            .map(|asset| format!("{} (toolchain {RUST_TOOLCHAIN})", asset.url))
            .unwrap_or_default()
    }

    fn install(&self, ctx: &Ctx) -> Result<Installed> {
        let asset = manifest::rustup_init(&ctx.target)
            .context("no rustup-init asset for this platform")?;
        let source = fetch_cached(ctx.fetcher, &asset, &prefix::downloads(ctx.root), ctx.verification)?;

        // Stage the installer itself inside the managed root.
        let staged = ctx.root.join("rust").join(ctx.exe("rustup-init"));
        install_binary(&source, &staged)?;

        let rustup_home = prefix::rustup_home(ctx.root);
        let cargo_home = prefix::cargo_home(ctx.root);
        std::fs::create_dir_all(&rustup_home)
            .with_context(|| format!("creating {}", rustup_home.display()))?;
        std::fs::create_dir_all(&cargo_home)
            .with_context(|| format!("creating {}", cargo_home.display()))?;

        let status = Command::new(&staged)
            .arg("-y")
            .arg("--no-modify-path")
            .arg("--profile")
            .arg("minimal")
            .arg("--default-toolchain")
            .arg(RUST_TOOLCHAIN)
            // Current rustup-init removed the old --no-self-update
            // flag; RUSTUP_AUTO_UPDATE=0 is its successor.
            .env("RUSTUP_AUTO_UPDATE", "0")
            .env("RUSTUP_HOME", &rustup_home)
            .env("CARGO_HOME", &cargo_home)
            .status()
            .with_context(|| format!("running {}", staged.display()))?;
        let _ = std::fs::remove_file(&staged);
        anyhow::ensure!(status.success(), "rustup-init exited with {status}");

        let mut binaries = BTreeMap::new();
        for name in ["rustup", "rustc", "cargo"] {
            let path = managed_binary(&cargo_bin(ctx.root), name, &ctx.target);
            anyhow::ensure!(path.is_file(), "rustup-init did not produce {}", path.display());
            binaries.insert(name.to_string(), path);
        }
        Ok(Installed {
            version: RUST_TOOLCHAIN.to_string(),
            binaries,
            sha256: asset.sha256.to_string(),
            installed_at: crate::state::now_secs(),
        })
    }
}