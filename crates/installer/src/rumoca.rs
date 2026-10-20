//! The managed Rumoca executable: a single pinned prebuilt binary
//! from the CogniPilot GitHub release (deliberately newer than the
//! crates.io release; installed without cargo).

use std::path::Path;

use anyhow::{Context, Result};

use crate::component::{self, Ctx, install_single_binary, managed_binary};
use crate::detect::{self, Target, parse_version, probe_output};
use crate::manifest::{self, RUMOCA_VERSION};
use crate::prefix;
use crate::state::Installed;

pub struct RumocaComponent;

impl component::Component for RumocaComponent {
    fn name(&self) -> &'static str {
        "rumoca"
    }

    fn pinned_version(&self) -> &'static str {
        RUMOCA_VERSION
    }

    fn probe(&self, root: &Path) -> Option<String> {
        let target = detect::host().ok()?;
        let rumoca = managed_binary(&prefix::rumoca_bin(root), "rumoca", &target);
        let output = probe_output(&rumoca)?;
        parse_version(&output, "rumoca")
    }

    fn note(&self, target: &Target) -> String {
        manifest::rumoca(target)
            .map(|asset| asset.url.to_string())
            .unwrap_or_default()
    }

    fn install(&self, ctx: &Ctx) -> Result<Installed> {
        let asset = manifest::rumoca(&ctx.target).with_context(|| {
            format!(
                "no prebuilt rumoca binary for {} — rumoca publishes no Windows ARM64 \
                 release asset; run setup --only rust,pixi on this host and wait for \
                 an upstream aarch64-windows build",
                ctx.target.triple
            )
        })?;
        install_single_binary(ctx, &asset, &prefix::rumoca_bin(ctx.root), "rumoca", RUMOCA_VERSION)
    }
}