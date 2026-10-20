//! Noema toolchain bootstrapper.
//!
//! Sets up everything Noema needs to compile and run mechanisms on a
//! fresh machine — a pinned Rust toolchain, the Rumoca Modelica
//! compiler and Pixi — inside a single Noema-managed directory
//! (`~/.noema/toolchain` by default, overridable with `--prefix` or
//! `NOEMA_TOOLCHAIN_DIR`). Nothing outside that directory is ever
//! touched: no profile edits, no `~/.cargo`, no system installs.
//!
//! The binary entry point is `noema-setup`. Library consumers should
//! use [`toolchain::Toolchain`] to resolve the installed executables
//! by absolute path and build the environment overlay for spawning
//! them — the installer never relies on the process `PATH`.

pub mod component;
pub mod detect;
pub mod download;
pub mod manifest;
pub mod pixi;
pub mod prefix;
pub mod prereqs;
pub mod report;
pub mod rumoca;
pub mod rust;
pub mod setup;
pub mod state;
pub mod toolchain;