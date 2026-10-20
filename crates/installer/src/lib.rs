//! Noema toolchain bootstrapper.
//!
//! Sets up everything Noema needs to compile and run mechanisms on a
//! fresh machine — a pinned Rust toolchain, the Rumoca Modelica
//! compiler and Pixi — inside a single Noema-managed directory
//! (`~/.noema/toolchain` by default, overridable with `--prefix` or
//! `NOEMA_TOOLCHAIN_DIR`). Nothing outside that directory is ever
//! touched: no profile edits, no `~/.cargo`, no system installs.
//! It then builds Noema itself from source and installs the binaries.
//!
//! The binary entry point is `noema-setup`. The directory layout,
//! provenance state and executable resolver live in
//! [`noema_config::toolchain`] (shared with the Noema app); this crate
//! re-exports them and provides everything needed to *write* the
//! installation: downloads, checksums, components and the setup flow.

pub use noema_config::{detect, prefix, state, toolchain};

pub mod component;
pub mod download;
pub mod manifest;
pub mod noema;
pub mod pixi;
pub mod prereqs;
pub mod report;
pub mod rumoca;
pub mod rust;
pub mod setup;