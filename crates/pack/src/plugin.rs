//! Dynamic candidate plugins: compile a plugin source file into a
//! dylib at load time and load it — the warehouse plugin-loader pattern
//! (`warehouse_core::plugin`) applied to mechanism discovery.
//!
//! The contract is exactly one function:
//!
//! ```ignore
//! pub fn make_candidate() -> Box<dyn skill::candidate::CandidateSpec>
//! ```
//!
//! The plugin hands over the *source* of a mechanism, never an
//! execution: the harness compiles it with Rumoca, simulates it, fits
//! it. The LLM-editable file contains no glue; the SDK appends the FFI
//! shim (`__noema_make_candidate`) at compile time, so a malformed
//! plugin fails at the rustc tier with the real compiler output — the
//! first evidence tier of `COMPILE_FAILED`.
//!
//! Compilations are cached under the content hash of the source in
//! `$TMPDIR/skill-plugins/<hash>/`, so an evolution loop that keeps
//! re-evaluating unchanged generations pays the compiler exactly once.

use std::collections::hash_map::DefaultHasher;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

use anyhow::{Context, Error, Result, bail};
use libloading::{Library, Symbol};

use crate::candidate::CandidateSpec;

/// Max times to invoke `rustc` before giving up. A single link can fail
/// transiently; a rerun usually succeeds.
pub(crate) const MAX_COMPILE_ATTEMPTS: u32 = 3;

/// Serializes plugin compilation; concurrent rustc invocations race
/// their linkers over the same per-process temp files.
pub(crate) static COMPILE_LOCK: Mutex<()> = Mutex::new(());

/// Library handles for every plugin dylib loaded by this process,
/// leaked on purpose: the returned `Box<dyn CandidateSpec>` points
/// into the dylib's vtable and code.
pub(crate) fn loaded_libs() -> MutexGuard<'static, Vec<Library>> {
    static LIBS: Mutex<Vec<Library>> = Mutex::new(Vec::new());

    LIBS.lock().unwrap_or_else(|error| error.into_inner())
}

pub(crate) fn dylib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libplugin.dylib"
    } else if cfg!(windows) {
        "plugin.dll"
    } else {
        "libplugin.so"
    }
}

/// Locate an rlib this process was linked against (same search as the
/// warehouse loader: `deps/` next to the executable, or the test
/// binary's own directory).
pub(crate) fn find_rlib(prefix: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;

    let exe_dir = exe.parent()?;

    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;

    for dir in [exe_dir.join("deps"), exe_dir.to_path_buf()] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            let Some(name) = path.file_name().and_then(|name| name.to_str())
            else {
                continue;
            };

            if name.starts_with(prefix) && name.ends_with(".rlib") {
                if let Ok(modified) =
                    entry.metadata().and_then(|m| m.modified())
                {
                    if best.as_ref().is_none_or(|(best, _)| modified >= *best)
                    {
                        best = Some((modified, path));
                    }
                }
            }
        }
    }

    best.map(|(_, path)| path)
}

pub(crate) fn skill_rlib() -> Option<PathBuf> {
    find_rlib("libskill-")
}

pub(crate) fn rustc() -> String {
    std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string())
}

pub(crate) fn rustc_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();

    VERSION.get_or_init(|| {
        Command::new(rustc())
            .arg("--version")
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            })
            .unwrap_or_default()
    })
}

pub(crate) fn cache_key(src: &[u8], rlib: &Path) -> String {
    let mut hasher = DefaultHasher::new();

    src.hash(&mut hasher);

    if let Ok(metadata) = std::fs::metadata(rlib) {
        (
            metadata.len(),
            metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        )
            .hash(&mut hasher);
    }

    rustc_version().hash(&mut hasher);

    format!("{:016x}", hasher.finish())
}

const GLUE: &[u8] = b"\n\n// ---- appended by skill::plugin (do not edit) ----\n\
    #[unsafe(no_mangle)]\n\
    pub extern \"C\" fn __noema_make_candidate() -> *mut std::ffi::c_void {\n\
    \x20   Box::into_raw(Box::new(make_candidate())) as *mut std::ffi::c_void\n\
    }\n";

pub(crate) fn compile(glue: &Path, rlib: &Path, out: &Path, crate_name: &str) -> Result<()> {
    let compiler = rustc();

    let mut command = Command::new(&compiler);

    command
        .arg(glue)
        .arg("--crate-type=cdylib")
        .arg(format!("--crate-name={crate_name}"))
        .arg("--edition=2024")
        .arg("-O")
        .arg("--extern")
        .arg(format!("skill={}", rlib.display()));

    if let Some(deps) = rlib.parent() {
        command.arg("-L").arg(format!("dependency={}", deps.display()));
    }

    command.arg("-o").arg(out);

    let output = command
        .output()
        .with_context(|| format!("running `{compiler}` to compile the plugin file"))?;

    if !output.status.success() {
        bail!(
            "plugin file failed to compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

/// Compile (or reuse the cached compilation of) `path` and return a
/// fresh candidate spec instance from it.
pub fn load_candidate(path: &Path) -> Result<Box<dyn CandidateSpec>, Error> {
    let src = std::fs::read(path)
        .with_context(|| format!("error reading plugin file {}", path.display()))?;

    let rlib = skill_rlib().with_context(|| {
        format!(
            "cannot find the skill rlib next to {} \
             (expected target/<profile>/deps/libskill-*.rlib — build the \
             workspace once so it exists); cannot compile plugin file {}",
            std::env::current_exe()
                .map(|exe| exe.display().to_string())
                .unwrap_or_default(),
            path.display(),
        )
    })?;

    let key = cache_key(&src, &rlib);

    let dir = std::env::temp_dir().join("skill-plugins").join(&key);

    let lib_path = dir.join(dylib_name());

    if !lib_path.exists() {
        let _guard = COMPILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        if !lib_path.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("creating plugin cache dir {}", dir.display()))?;

            let glue_path = dir.join("plugin_glue.rs");

            let mut source = src.clone();
            source.extend_from_slice(GLUE);

            std::fs::write(&glue_path, &source)
                .context("writing plugin glue source")?;

            let tmp_lib =
                dir.join(format!("{}.tmp{}", dylib_name(), std::process::id()));

            let mut last_error: Option<Error> = None;

            for attempt in 1..=MAX_COMPILE_ATTEMPTS {
                match compile(&glue_path, &rlib, &tmp_lib, "noema_candidate_plugin") {
                    Ok(()) => {
                        last_error = None;
                        break;
                    }
                    Err(error) => {
                        last_error = Some(error);

                        eprintln!(
                            "[plugin] compile attempt {attempt}/{MAX_COMPILE_ATTEMPTS} \
                             for {} failed (will retry)",
                            path.display(),
                        );
                    }
                }
            }

            if let Some(error) = last_error {
                bail!(
                    "compiling plugin file {} failed after {MAX_COMPILE_ATTEMPTS} \
                     attempts: {error:#}",
                    path.display(),
                );
            }

            if std::fs::rename(&tmp_lib, &lib_path).is_err() && !lib_path.exists() {
                bail!(
                    "could not move the compiled plugin dylib into {}",
                    lib_path.display(),
                );
            }

            eprintln!(
                "[plugin] compiled plugin file {} -> {} (hash {})",
                path.display(),
                lib_path.display(),
                key,
            );
        }
    }

    let library = unsafe { Library::new(&lib_path) }
        .with_context(|| format!("loading plugin dylib {}", lib_path.display()))?;

    let create: Symbol<unsafe extern "C" fn() -> *mut c_void> =
        unsafe { library.get(b"__noema_make_candidate\0") }.with_context(|| {
            format!(
                "plugin file {} does not define the contract function \
                 `pub fn make_candidate() -> Box<dyn skill::candidate::CandidateSpec>`",
                path.display(),
            )
        })?;

    let ptr = unsafe { create() } as *mut Box<dyn CandidateSpec>;

    if ptr.is_null() {
        bail!("make_candidate returned null");
    }

    let spec = {
        let slot = unsafe { Box::from_raw(ptr) };
        *slot
    };

    loaded_libs().push(library);

    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_shipped_binding_pack_plugin_from_source() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("packs/binding/baseline_plugin.rs");

        let spec = match load_candidate(&path) {
            Ok(spec) => spec,
            Err(error) => {
                let message = format!("{error:#}");

                // A bare `cargo test -p skill` may not have produced
                // an rlib yet; that is an environment gap, not a
                // plugin defect.
                assert!(
                    message.contains("cannot find the skill rlib"),
                    "plugin load failed: {message}",
                );

                eprintln!("skipping plugin load test: {message}");

                return;
            }
        };

        assert_eq!(spec.name(), "Binding1to1");

        let source = spec.modelica();

        assert!(source.contains("model Binding1to1"));
        assert!(source.contains("input Real concentration"));
        assert!(source.contains("output Real signal"));
    }
}