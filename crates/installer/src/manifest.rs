//! Pinned toolchain versions and their verified download sources.
//!
//! Everything the installer fetches is pinned to an exact version with
//! an exact sha256. Upgrading a component is a one-line change here
//! plus a checksum refresh, and the Docker end-to-end run (see
//! `docker/README.md`) verifies the result before it ships.
//!
//! Checksum sources, as collected when this manifest was written:
//! - rustup-init: the official `rustup-init*.sha256` sidecars on
//!   `static.rust-lang.org`;
//! - pixi: the `.sha256` sidecars / GitHub release asset digests of
//!   `prefix-dev/pixi` v0.82.0;
//! - rumoca: the digests published on the `CogniPilot/rumoca` v0.9.20
//!   release page.

use crate::detect::Target;

/// Rust toolchain installed through rustup (minimal profile). Must
/// stay at or above the workspace's own MSRV *and* the highest MSRV of
/// its locked dependencies (the aws-sdk stack requires 1.94.1+);
/// pinned to the stable release the workspace is developed against.
pub const RUST_TOOLCHAIN: &str = "1.99.1";

/// Pinned Pixi release.
pub const PIXI_VERSION: &str = "0.82.0";

/// Pinned Rumoca release. Note: deliberately ahead of the crates.io
/// release; we install the prebuilt binaries from the GitHub release.
pub const RUMOCA_VERSION: &str = "0.9.20";

// ---------------------------------------------------------------------------
// Noema itself: built from source inside the managed directory.
// ---------------------------------------------------------------------------

/// GitHub repository (`owner/name`) of the Noema app workspace.
pub const NOEMA_REPO: &str = "NoemaSI/noema";

/// Source ref built when the user does not pass `--ref`: a branch is
/// resolved to its exact commit through the GitHub API, so a setup run
/// is still reproducible.
pub const NOEMA_DEFAULT_REF: &str = "main";

/// Workspace packages the installer builds from source.
pub const NOEMA_BUILD_PACKAGES: [&str; 2] = ["ui", "pack"];

/// Source archive URL for a repo at an exact ref (branch, tag or sha).
pub fn source_zip_url(repo: &str, ref_name: &str) -> String {
    format!("https://github.com/{repo}/archive/{ref_name}.zip")
}

/// GitHub API endpoint resolving any ref (branch/tag/sha) to a commit.
pub fn commit_api_url(repo: &str, ref_name: &str) -> String {
    format!("https://api.github.com/repos/{repo}/commits/{ref_name}")
}

/// One downloadable, checksum-pinned binary asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// The hosted file name, used in the URL.
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

impl Asset {
    /// The name the binary gets once installed into the toolchain
    /// (`.exe` preserved on Windows).
    pub fn dest_name(&self) -> String {
        let base = match self.name {
            n if n.starts_with("rustup-init") => "rustup-init",
            n if n.starts_with("pixi-") => "pixi",
            n if n.starts_with("rumoca-") => "rumoca",
            other => other,
        };
        if self.name.ends_with(".exe") {
            format!("{base}.exe")
        } else {
            base.to_string()
        }
    }
}

// Release roots, kept for the URL sanity assertions in tests.
#[cfg(test)]
const RUSTUP_BASE: &str = "https://static.rust-lang.org/rustup/dist";
#[cfg(test)]
const PIXI_BASE: &str = "https://github.com/prefix-dev/pixi/releases/download";

/// rustup-init for the given host triple (Windows adds `.exe`).
pub fn rustup_init(target: &Target) -> Option<Asset> {
    Some(match (target.os, target.arch) {
        ("linux", "x86_64") => Asset {
            name: "rustup-init",
            url: "https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init",
            sha256: "dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71",
        },
        ("linux", "aarch64") => Asset {
            name: "rustup-init",
            url: "https://static.rust-lang.org/rustup/dist/aarch64-unknown-linux-gnu/rustup-init",
            sha256: "15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433",
        },
        ("macos", "x86_64") => Asset {
            name: "rustup-init",
            url: "https://static.rust-lang.org/rustup/dist/x86_64-apple-darwin/rustup-init",
            sha256: "259e2b84274434085163fe8d556510571772cda2aa6d87ca6aa664f57bc644e3",
        },
        ("macos", "aarch64") => Asset {
            name: "rustup-init",
            url: "https://static.rust-lang.org/rustup/dist/aarch64-apple-darwin/rustup-init",
            sha256: "ec1b9233e7f72990ecd8e62063fa7f6c3dfc2bec8e97f88bff165f9100ac696a",
        },
        ("windows", "x86_64") => Asset {
            name: "rustup-init.exe",
            url: "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe",
            sha256: "6f4bef66261261fcb43131be8720bab817d403a09edec7455c371974b90bdb7e",
        },
        ("windows", "aarch64") => Asset {
            name: "rustup-init.exe",
            url: "https://static.rust-lang.org/rustup/dist/aarch64-pc-windows-msvc/rustup-init.exe",
            sha256: "01aa49cf9574a8bd0ae52005d7de2590e8f27181ded6748236e702c92aef826d",
        },
        _ => return None,
    })
}

/// The raw (unpacked) pixi binary for the given host.
pub fn pixi(target: &Target) -> Option<Asset> {
    Some(match (target.os, target.arch) {
        ("linux", "x86_64") => Asset {
            name: "pixi-x86_64-unknown-linux-musl",
            url: "https://github.com/prefix-dev/pixi/releases/download/v0.82.0/pixi-x86_64-unknown-linux-musl",
            sha256: "1dff1db704884f078c44b4c7fb3c5bc6f946fc3d1fa679edf214ad79187a9c64",
        },
        ("linux", "aarch64") => Asset {
            name: "pixi-aarch64-unknown-linux-musl",
            url: "https://github.com/prefix-dev/pixi/releases/download/v0.82.0/pixi-aarch64-unknown-linux-musl",
            sha256: "c6e3c4d98bb41560b5cfdebbe6cbf766f3547603a0fb96e1450a6ccff74b5e14",
        },
        ("macos", "x86_64") => Asset {
            name: "pixi-x86_64-apple-darwin",
            url: "https://github.com/prefix-dev/pixi/releases/download/v0.82.0/pixi-x86_64-apple-darwin",
            sha256: "db4bd6dc3a5f88f2d94cf2e1f8a377b53d3aaac95b45b058854fbf53919d6d36",
        },
        ("macos", "aarch64") => Asset {
            name: "pixi-aarch64-apple-darwin",
            url: "https://github.com/prefix-dev/pixi/releases/download/v0.82.0/pixi-aarch64-apple-darwin",
            sha256: "92aa551c67a56ec0ba105fa20c3f48094b02586de523e24b96974ba1f092455b",
        },
        ("windows", "x86_64") => Asset {
            name: "pixi-x86_64-pc-windows-msvc.exe",
            url: "https://github.com/prefix-dev/pixi/releases/download/v0.82.0/pixi-x86_64-pc-windows-msvc.exe",
            sha256: "89c7455923dac401c1853310894d87fa06602911b838e389a46c7bc8df00171e",
        },
        ("windows", "aarch64") => Asset {
            name: "pixi-aarch64-pc-windows-msvc.exe",
            url: "https://github.com/prefix-dev/pixi/releases/download/v0.82.0/pixi-aarch64-pc-windows-msvc.exe",
            sha256: "d205d144618bdc83e25dba1a10c8cff8f39e9da85c06e142a9ec0d0c8f6d3559",
        },
        _ => return None,
    })
}

/// The prebuilt rumoca binary for the given host. Rumoca publishes no
/// Windows ARM64 build; callers must surface that clearly.
pub fn rumoca(target: &Target) -> Option<Asset> {
    match (target.os, target.arch) {
        ("linux", "x86_64") => Some(Asset {
            name: "rumoca-linux-x86_64",
            url: "https://github.com/CogniPilot/rumoca/releases/download/v0.9.20/rumoca-linux-x86_64",
            sha256: "12727925c304c5188651e8b6b90f45071c8ad318e76b163c7adee160c649ba1a",
        }),
        ("linux", "aarch64") => Some(Asset {
            name: "rumoca-linux-aarch64",
            url: "https://github.com/CogniPilot/rumoca/releases/download/v0.9.20/rumoca-linux-aarch64",
            sha256: "5a4008d0320b264d2bf180d83dd1d9dc9616719935b94c4910d1b75334a459a8",
        }),
        ("macos", "x86_64") => Some(Asset {
            name: "rumoca-macos-x86_64",
            url: "https://github.com/CogniPilot/rumoca/releases/download/v0.9.20/rumoca-macos-x86_64",
            sha256: "274121445c4964de99fb12468e69c17aec4e7fb4006bd26b1043540997f3403f",
        }),
        ("macos", "aarch64") => Some(Asset {
            name: "rumoca-macos-aarch64",
            url: "https://github.com/CogniPilot/rumoca/releases/download/v0.9.20/rumoca-macos-aarch64",
            sha256: "11431617a6d57f655fb5a300e8e863e1a4cf6f99f534bde726da910b17174429",
        }),
        ("windows", "x86_64") => Some(Asset {
            name: "rumoca-windows-x86_64.exe",
            url: "https://github.com/CogniPilot/rumoca/releases/download/v0.9.20/rumoca-windows-x86_64.exe",
            sha256: "5726f35dccbb5c3f8f06dbec2c287585e68d43f8a19a8224be8e052c4e60a05e",
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(os: &'static str, arch: &'static str) -> Target {
        Target {
            os,
            arch,
            triple: "test-triple",
        }
    }

    const SUPPORTED: [(&str, &str); 6] = [
        ("linux", "x86_64"),
        ("linux", "aarch64"),
        ("macos", "x86_64"),
        ("macos", "aarch64"),
        ("windows", "x86_64"),
        ("windows", "aarch64"),
    ];

    #[test]
    fn rustup_and_pixi_cover_every_supported_target() {
        for (os, arch) in SUPPORTED {
            let t = target(os, arch);
            let rustup = rustup_init(&t).expect("rustup-init asset");
            let pixi = pixi(&t).expect("pixi asset");
            assert!(rustup.url.starts_with(RUSTUP_BASE));
            assert!(pixi.url.starts_with(PIXI_BASE));
            assert_eq!(rustup.sha256.len(), 64, "{os}/{arch} rustup sha");
            assert_eq!(pixi.sha256.len(), 64, "{os}/{arch} pixi sha");
            assert!(rustup.url.ends_with(rustup.name));
            assert!(pixi.url.ends_with(pixi.name));
        }
    }

    #[test]
    fn rumoca_urls_and_digests_match_the_v0_9_20_release() {
        let t = target("linux", "x86_64");
        let asset = rumoca(&t).unwrap();
        assert_eq!(
            asset.url,
            "https://github.com/CogniPilot/rumoca/releases/download/v0.9.20/rumoca-linux-x86_64"
        );
        assert_eq!(
            asset.sha256,
            "12727925c304c5188651e8b6b90f45071c8ad318e76b163c7adee160c649ba1a"
        );
        assert_eq!(asset.dest_name(), "rumoca");
    }

    #[test]
    fn rumoca_is_absent_on_windows_arm64() {
        assert!(rumoca(&target("windows", "aarch64")).is_none());
    }

    #[test]
    fn unsupported_triples_yield_no_assets() {
        let t = target("linux", "riscv64");
        assert!(rustup_init(&t).is_none());
        assert!(pixi(&t).is_none());
        assert!(rumoca(&t).is_none());
    }
}