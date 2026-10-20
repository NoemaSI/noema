//! The managed Pixi executable: a single pinned binary from the
//! prefix-dev GitHub release, no installer, no shell scripts.

use std::path::Path;

use anyhow::{Context, Result};

use crate::component::{self, Ctx, install_single_binary, managed_binary};
use crate::detect::{self, Target, parse_version, probe_output};
use crate::manifest::{self, PIXI_VERSION};
use crate::prefix;
use crate::state::Installed;

pub struct PixiComponent;

impl component::Component for PixiComponent {
    fn name(&self) -> &'static str {
        "pixi"
    }

    fn pinned_version(&self) -> &'static str {
        PIXI_VERSION
    }

    fn probe(&self, root: &Path) -> Option<String> {
        let target = detect::host().ok()?;
        let pixi = managed_binary(&prefix::pixi_bin(root), "pixi", &target);
        let output = probe_output(&pixi)?;
        parse_version(&output, "pixi")
    }

    fn note(&self, target: &Target) -> String {
        manifest::pixi(target)
            .map(|asset| asset.url.to_string())
            .unwrap_or_default()
    }

    fn install(&self, ctx: &Ctx) -> Result<Installed> {
        let asset = manifest::pixi(&ctx.target).context("no pixi asset for this platform")?;
        install_single_binary(ctx, &asset, &prefix::pixi_bin(ctx.root), "pixi", PIXI_VERSION)
    }
}