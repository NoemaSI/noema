//! Host detection and executable probing.

use std::path::Path;
use std::process::Command;

/// The host platform, expressed as the names the pinned release assets
/// use (`os`: `linux` | `macos` | `windows`, `arch`: `x86_64` |
/// `aarch64`) plus the Rust target triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub os: &'static str,
    pub arch: &'static str,
    pub triple: &'static str,
}

/// Detect the host target. Errors on unsupported platforms instead of
/// guessing an asset.
pub fn host() -> anyhow::Result<Target> {
    let os = match std::env::consts::FAMILY {
        "unix" => match std::env::consts::OS {
            "linux" => "linux",
            "macos" => "macos",
            other => anyhow::bail!("unsupported operating system: {other}"),
        },
        "windows" => "windows",
        other => anyhow::bail!("unsupported platform family: {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => anyhow::bail!("unsupported CPU architecture: {other}"),
    };
    let triple = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        _ => anyhow::bail!("unsupported target: {os}/{arch}"),
    };
    Ok(Target { os, arch, triple })
}

impl Target {
    /// Executable suffix on this platform.
    pub fn exe_suffix(&self) -> &'static str {
        if self.os == "windows" { ".exe" } else { "" }
    }

    pub fn is_windows(&self) -> bool {
        self.os == "windows"
    }
}

/// Run `<path> --version` and return the raw combined output, if the
/// executable exists and exits successfully.
pub fn probe_output(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    Some(text)
}

/// Extract the version token from a `--version` line, e.g.
/// `("rumoca 0.9.20", "rumoca") -> Some("0.9.20")` and
/// `("rustc 1.89.0 (abcd 2025-08-14)", "rustc") -> Some("1.89.0")`.
pub fn parse_version(output: &str, name: &str) -> Option<String> {
    let line = output.lines().find(|line| line.trim_start().starts_with(name))?;
    let token = line.split_whitespace().nth(1)?;
    let version = token.trim_start_matches('v');
    if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        Some(version.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_version_lines() {
        assert_eq!(parse_version("rumoca 0.9.20\n", "rumoca").as_deref(), Some("0.9.20"));
        assert_eq!(parse_version("pixi 0.82.0", "pixi").as_deref(), Some("0.82.0"));
    }

    #[test]
    fn parses_rustc_version_with_metadata() {
        let out = "rustc 1.89.0 (3adabf179 2025-08-14)\nbinary: rustc\n";
        assert_eq!(parse_version(out, "rustc").as_deref(), Some("1.89.0"));
    }

    #[test]
    fn rejects_wrong_tool_or_garbage() {
        assert_eq!(parse_version("rustc 1.89.0", "pixi"), None);
        assert_eq!(parse_version("nope", "pixi"), None);
        assert_eq!(parse_version("pixi", "pixi"), None);
        assert_eq!(parse_version("pixi nightly", "pixi"), None);
    }

    #[test]
    fn host_detection_matches_compile_time_consts() {
        let target = host();
        match target {
            Ok(t) => assert!(t.triple.contains(t.arch)),
            Err(_) => {
                // Unsupported host: fine for running the test suite,
                // the installer itself must refuse to run.
            }
        }
    }
}