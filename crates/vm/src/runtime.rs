use std::sync::OnceLock;

use smolmachines::{RuntimeAssets, configure_runtime_assets as smol_configure};

use crate::{Error, Result};

static RUNTIME: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Point the engine at assets you ship yourself (boot helper binary,
/// hypervisor libraries, guest agent rootfs) instead of auto-discovery.
///
/// Call this once before the first machine; it also marks the runtime as
/// initialized so [`ensure_runtime`] becomes a no-op.
pub fn configure_runtime(assets: RuntimeAssets) -> Result<()> {
    smol_configure(assets)?;
    let _ = RUNTIME.set(Ok(()));
    Ok(())
}

/// Point the engine at its runtime assets (boot helper binary, hypervisor
/// libraries, guest agent rootfs).
///
/// To boot a VM the engine re-executes a binary that knows how to be a VM.
/// In an embedded process that is not your program, so the assets must be
/// named before the first machine:
///
/// - an installed `smolvm` on PATH whose version matches this SDK, if any;
/// - otherwise the matching engine release, fetched once into the local cache.
///
/// Safe to call multiple times; the work happens once per process. If you
/// ship your own engine, call [`configure_runtime`] first and this will skip.
pub fn ensure_runtime() -> Result<()> {
    let init = RUNTIME.get_or_init(|| {
        let result: Result<()> = (|| {
            let assets = match RuntimeAssets::from_path_lookup() {
                Some(assets) => assets,
                None => RuntimeAssets::new().boot_binary(smolmachines::bootstrap::ensure_engine()?),
            };
            smol_configure(assets)?;
            Ok(())
        })();
        result.map_err(|e| e.to_string())
    });
    init.as_ref().map_err(|msg| Error::Runtime(msg.clone())).copied()
}