//! Dynamic problem packs: compile a pack source file into a dylib at
//! load time and load it — the same loader pattern as
//! [`crate::plugin`], applied to the problem definition itself.
//!
//! The contract is exactly one function:
//!
//! ```ignore
//! pub fn load_pack() -> Box<dyn skill::pack::ProblemPack>
//! ```
//!
//! The SDK appends the FFI shim (`__noema_make_skill`) at compile
//! time, so a malformed pack fails at the rustc tier with the real
//! compiler output. Compilations are cached under the content hash of
//! the source in `$TMPDIR/skill-packs/<hash>/`.
//!
//! The pack's *home* (the directory holding its source file) travels
//! with the handle so baseline paths can stay relative to the pack.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use anyhow::{Context, Error, Result, bail};
use libloading::{Library, Symbol};

use crate::pack::ProblemPack;
use crate::plugin::{
    COMPILE_LOCK, cache_key, compile, dylib_name, loaded_libs, skill_rlib,
};

const GLUE: &[u8] = b"\n\n// ---- appended by skill::pack_loader (do not edit) ----\n\
    #[unsafe(no_mangle)]\n\
    pub extern \"C\" fn __noema_make_skill() -> *mut std::ffi::c_void {\n\
    \x20   Box::into_raw(Box::new(load_pack())) as *mut std::ffi::c_void\n\
    }\n";

/// A loaded pack plus the home directory its relative paths resolve
/// against.
pub struct PackHandle {
    pub pack: Box<dyn ProblemPack>,
    pub home: PathBuf,
}

impl PackHandle {
    /// Resolve a baseline path: absolute paths pass through, relative
    /// paths join the pack home.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.home.join(path)
        }
    }
}

/// Compile (or reuse the cached compilation of) `path` and return the
/// pack instance with its home directory.
pub fn load_pack(path: &Path) -> Result<PackHandle, Error> {
    let src = std::fs::read(path)
        .with_context(|| format!("error reading pack file {}", path.display()))?;

    let rlib = skill_rlib().with_context(|| {
        format!(
            "cannot find the skill rlib next to {} \
             (expected target/<profile>/deps/libskill-*.rlib — build the \
             workspace once so it exists); cannot compile pack file {}",
            std::env::current_exe()
                .map(|exe| exe.display().to_string())
                .unwrap_or_default(),
            path.display(),
        )
    })?;

    let key = cache_key(&src, &rlib);

    let dir = std::env::temp_dir().join("skill-packs").join(&key);

    let lib_path = dir.join(dylib_name());

    if !lib_path.exists() {
        let _guard = COMPILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        if !lib_path.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("creating pack cache dir {}", dir.display()))?;

            let glue_path = dir.join("pack_glue.rs");

            let mut source = src.clone();
            source.extend_from_slice(GLUE);

            std::fs::write(&glue_path, &source)
                .context("writing pack glue source")?;

            let tmp_lib =
                dir.join(format!("{}.tmp{}", dylib_name(), std::process::id()));

            let mut last_error: Option<Error> = None;

            for attempt in 1..=crate::plugin::MAX_COMPILE_ATTEMPTS {
                match compile(&glue_path, &rlib, &tmp_lib, "noema_skill_pack") {
                    Ok(()) => {
                        last_error = None;
                        break;
                    }
                    Err(error) => {
                        last_error = Some(error);

                        eprintln!(
                            "[pack] compile attempt {attempt}/{} \
                             for {} failed (will retry)",
                            crate::plugin::MAX_COMPILE_ATTEMPTS,
                            path.display(),
                        );
                    }
                }
            }

            if let Some(error) = last_error {
                bail!(
                    "compiling pack file {} failed after {} attempts: {error:#}",
                    path.display(),
                    crate::plugin::MAX_COMPILE_ATTEMPTS,
                );
            }

            if std::fs::rename(&tmp_lib, &lib_path).is_err() && !lib_path.exists() {
                bail!(
                    "could not move the compiled pack dylib into {}",
                    lib_path.display(),
                );
            }

            eprintln!(
                "[pack] compiled pack file {} -> {} (hash {})",
                path.display(),
                lib_path.display(),
                key,
            );
        }
    }

    let library = unsafe { Library::new(&lib_path) }
        .with_context(|| format!("loading pack dylib {}", lib_path.display()))?;

    let create: Symbol<unsafe extern "C" fn() -> *mut c_void> =
        unsafe { library.get(b"__noema_make_skill\0") }.with_context(|| {
            format!(
                "pack file {} does not define the contract function \
                 `pub fn load_pack() -> Box<dyn skill::pack::ProblemPack>`",
                path.display(),
            )
        })?;

    let ptr = unsafe { create() } as *mut Box<dyn ProblemPack>;

    if ptr.is_null() {
        bail!("load_pack returned null");
    }

    let pack = {
        let slot = unsafe { Box::from_raw(ptr) };
        *slot
    };

    loaded_libs().push(library);

    let fallback = || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let home = path
        .parent()
        .map(|parent| {
            if parent.is_absolute() {
                parent.to_path_buf()
            } else {
                fallback().join(parent)
            }
        })
        .unwrap_or_else(fallback);

    let lexicon = pack.lexicon();

    if let Err(problem) = lexicon.validate() {
        bail!("pack {} declares an invalid lexicon: {problem}", path.display());
    }

    Ok(PackHandle { pack, home })
}