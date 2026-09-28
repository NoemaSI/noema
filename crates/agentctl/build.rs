//! Embeds the guest source digest (written by jobctl's EnsureVm before the
//! build) into the binary, so `/healthz` can reveal a stale running server.

use std::path::Path;

const SHA_FILE: &str = "/opt/noema/agentctl.sha";

fn main() {
    println!("cargo:rerun-if-changed={SHA_FILE}");
    let sha = std::fs::read_to_string(SHA_FILE)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=AGENTCTL_SHA={sha}");
    // Host builds do not have the marker; silence unused warnings for Path.
    let _ = Path::new(SHA_FILE);
}