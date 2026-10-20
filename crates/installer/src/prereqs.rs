//! Native build-prerequisite checks.
//!
//! Noema compiles plugin code on the user's machine with the managed
//! Rust toolchain, so a working system linker/compiler is required.
//! The installer never installs system packages (that needs admin
//! rights) — it detects what is missing and prints an actionable
//! message for the exact platform.

use std::process::Command;

use crate::detect::Target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrereqReport {
    /// A native compiler/linker usable by rustc was found.
    pub ok: bool,
    /// Human-readable findings; guidance lines when `ok` is false.
    pub notes: Vec<String>,
}

/// Check for a native toolchain rustc can link against. Pure probing:
/// spawns `--version`-style checks only, writes nothing.
pub fn check_native_build(target: &Target) -> PrereqReport {
    match target.os {
        "linux" => check_unix_cc(),
        "macos" => check_macos_command_line_tools(),
        "windows" => check_msvc(),
        other => PrereqReport {
            ok: false,
            notes: vec![format!("unsupported platform: {other}")],
        },
    }
}

fn command_exists(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn check_unix_cc() -> PrereqReport {
    let cc_found = ["cc", "clang", "gcc"].into_iter().find(|cc| command_exists(cc, &["--version"]));
    let Some(cc) = cc_found else {
        return PrereqReport {
            ok: false,
            notes: vec![
                "no native C compiler/linker found (cc, clang, gcc)".to_string(),
                "Noema compiles plugin code with rustc, which needs one.".to_string(),
                "install it, e.g.:".to_string(),
                "  Debian/Ubuntu: sudo apt install build-essential".to_string(),
                "  Fedora:        sudo dnf install gcc".to_string(),
                "  Arch:          sudo pacman -S base-devel".to_string(),
            ],
        };
    };
    let mut notes = vec![format!("found native compiler: {cc}")];
    let cpp_found = ["c++", "clang++", "g++"]
        .into_iter()
        .any(|cpp| command_exists(cpp, &["--version"]));
    if !cpp_found {
        notes.push(
            "WARNING: no C++ compiler found (c++, clang++, g++) — building Noema from source \
             needs one (Debian/Ubuntu: sudo apt install build-essential)."
                .to_string(),
        );
    }
    notes.extend(check_linux_openssl());
    PrereqReport { ok: true, notes }
}

/// The Noema source build compiles `openssl-sys`, which needs
/// pkg-config and OpenSSL development headers on Linux. Warn (but do
/// not fail) when they are missing — the build would stop later with a
/// far less actionable message.
fn check_linux_openssl() -> Option<String> {
    if !command_exists("pkg-config", &["--version"]) {
        return Some(
            "WARNING: pkg-config not found — the Noema build needs it plus OpenSSL headers \
             (Debian/Ubuntu: sudo apt install pkg-config libssl-dev)."
                .to_string(),
        );
    }
    if !command_exists("pkg-config", &["--modversion", "openssl"]) {
        return Some(
            "WARNING: OpenSSL development headers not found — the Noema build needs them \
             (Debian/Ubuntu: sudo apt install libssl-dev)."
                .to_string(),
        );
    }
    None
}

fn check_macos_command_line_tools() -> PrereqReport {
    if command_exists("xcode-select", &["-p"]) {
        return PrereqReport {
            ok: true,
            notes: vec!["Xcode Command Line Tools found".to_string()],
        };
    }
    PrereqReport {
        ok: false,
        notes: vec![
            "Xcode Command Line Tools are not installed".to_string(),
            "run: xcode-select --install".to_string(),
        ],
    }
}

fn check_msvc() -> PrereqReport {
    // vswhere is installed by every Visual Studio / Build Tools SKU
    // since 2017 and is the supported way to discover an MSVC install.
    let vswhere = {
        let mut candidates = Vec::new();
        if let Some(x86) = std::env::var_os("ProgramFiles(x86)") {
            candidates.push(std::path::PathBuf::from(x86));
        }
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            candidates.push(std::path::PathBuf::from(pf));
        }
        candidates.push(std::path::PathBuf::from(r"C:\Program Files (x86)"));
        candidates
            .into_iter()
            .map(|base| base.join(r"Microsoft Visual Studio\Installer\vswhere.exe"))
            .find(|path| path.is_file())
    };
    let Some(vswhere) = vswhere else {
        return PrereqReport {
            ok: false,
            notes: vec![
                "no Visual Studio C++ Build Tools found (vswhere.exe missing)".to_string(),
                "rustc on Windows needs the MSVC linker from Build Tools.".to_string(),
                "install \"Desktop development with C++\" from".to_string(),
                "  https://visualstudio.microsoft.com/visual-cpp-build-tools/".to_string(),
                "or a Visual Studio workload with the MSVC v143 toolset.".to_string(),
            ],
        };
    };
    let output = Command::new(&vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output();
    match output {
        Ok(output)
            if output.status.success()
                && !String::from_utf8_lossy(&output.stdout).trim().is_empty() =>
        {
            PrereqReport {
                ok: true,
                notes: vec![format!(
                    "MSVC toolset found: {}",
                    String::from_utf8_lossy(&output.stdout).trim()
                )],
            }
        }
        _ => PrereqReport {
            ok: false,
            notes: vec![
                "Visual Studio is installed but without the C++ toolset \
                 (Microsoft.VisualStudio.Component.VC.Tools.x86.x64)"
                    .to_string(),
                "add \"Desktop development with C++\" via the Visual Studio Installer."
                    .to_string(),
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_cc_when_a_compiler_is_present() {
        // The Unix CI/container this runs in always has cc; on Windows
        // this test asserts the probe at least produces a verdict.
        let target = Target { os: "linux", arch: "x86_64", triple: "x" };
        let report = check_native_build(&target);
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(report.ok, "expected a cc in the test environment: {report:?}");
        }
        assert!(!report.notes.is_empty());
    }

    #[test]
    fn unknown_os_is_not_ok() {
        let target = Target { os: "plan9", arch: "x86_64", triple: "x" };
        assert!(!check_native_build(&target).ok);
    }
}