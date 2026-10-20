# installer — the Noema toolchain bootstrapper

`noema-setup` prepares a fresh machine for Noema on **Linux, macOS and
Windows**: a pinned Rust toolchain (via rustup), the **Rumoca** Modelica
compiler and **Pixi** — all installed into one Noema-managed directory.

Noema compiles plugin code on the user's machine, so the managed Rust
toolchain and a working native linker are hard requirements, not
optional extras.

## Hermetic by design

Every byte the installer writes lives under a single root:

1. `--prefix <DIR>`
2. `NOEMA_TOOLCHAIN_DIR`
3. `~/.noema/toolchain` (default)

It never edits shell profiles, never touches `~/.cargo` or `~/.pixi`,
never installs system packages and never requires admin rights. That
also means the whole thing runs inside a throwaway Docker container
(see [`docker/Dockerfile`](docker/Dockerfile)) without touching your
host.

```
<root>/
  state.json          # provenance: versions, binary paths, sha256s
  rust/rustup/        # RUSTUP_HOME (toolchains live here)
  rust/cargo/         # CARGO_HOME (rustup shims: rustc, cargo, rustup)
  pixi/bin/pixi       # pinned pixi binary
  rumoca/bin/rumoca   # pinned rumoca binary
  downloads/          # download cache, pruned after a successful setup
```

## Pinned versions (`src/manifest.rs`)

| Component | Version | Source | Verified by |
| --- | --- | --- | --- |
| Rust toolchain | 1.89.0 (minimal profile) | static.rust-lang.org | official `rustup-init*.sha256` |
| Pixi | 0.82.0 | prefix-dev GitHub release | release `.sha256` / asset digests |
| Rumoca | 0.9.20 | CogniPilot GitHub release (prebuilt binaries, newer than crates.io) | release-page sha256 digests |

Every download is checked against its pinned sha256 before anything is
executed. Upgrading a component = one version bump + one checksum
refresh in `manifest.rs`, then re-run the Docker end-to-end test.

Windows ARM64: rust + pixi install normally; rumoca publishes no
`aarch64-windows` binary, so that component reports `unavailable`
instead of guessing.

## CLI

```
noema-setup [setup] [--prefix DIR] [--only rust,pixi,rumoca] [--dry-run] [--json]
noema-setup verify  [--prefix DIR] [--only ...] [--json]
noema-setup uninstall [--prefix DIR] [--yes]
```

- **setup** — idempotent: probes each component first; a second run is
  pure verification. Missing prerequisites (no `cc` on Linux, no Xcode
  CLT on macOS, no MSVC Build Tools on Windows) are reported with exact
  install guidance, never auto-installed.
- **`--dry-run`** — prints the complete action plan (target triple,
  root, per-component skip/install decisions, URLs + pinned checksums,
  prerequisite verdict) and exits without downloading or writing a
  single byte.
- **`--json`** — one JSON event per line on stdout for UI/CI
  consumption.

## Library use (for other Noema crates)

The managed binaries are launched by absolute path — never off `PATH`:

```rust
use installer::toolchain::Toolchain;

let toolchain = Toolchain::discover()?; // same resolution order as --prefix
let mut command = std::process::Command::new(toolchain.pixi()?);
command.args(["run", "python", "fit.py"]);
for (key, value) in toolchain.env_overlay() {
    command.env(key, value); // PATH + RUSTUP_HOME + CARGO_HOME overlays
}
```

If a component is missing the resolver returns
`` `pixi` is not installed in <root> — run `noema-setup` first `` rather
than silently falling back to a system binary. (Wiring `pack`'s
`Command::new("pixi"/"rumoca")` call sites onto this API is a planned
follow-up.)

## Testing without touching your machine

Two layers, no host side effects:

```bash
# 1. Offline unit + integration tests (synthetic stub binaries, zero
#    network): covers install, checksum-mismatch rejection, idempotent
#    re-run, dry-run writes nothing, state round-trip, resolver.
#    The installer is a standalone workspace, so run cargo inside it:
cd crates/installer && cargo test --release

# 2. Real end-to-end, fully containerised:
docker build -f crates/installer/docker/Dockerfile -t noema-setup crates/installer
docker run --rm -v noema-toolchain:/toolchain noema-setup setup --dry-run
docker run --rm -v noema-toolchain:/toolchain noema-setup setup
docker run --rm -v noema-toolchain:/toolchain noema-setup verify
# inspect what landed:
docker run --rm -v noema-toolchain:/toolchain --entrypoint /bin/sh noema-setup \
  -c 'find /toolchain -maxdepth 3 | head -50; cat /toolchain/state.json'
```

The container's toolchain volume is the only thing a setup run can
affect; delete it (`docker volume rm noema-toolchain`) and you have a
factory-fresh machine again.