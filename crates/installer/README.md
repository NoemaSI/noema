# installer — the Noema toolchain bootstrapper

`noema-setup` prepares a fresh machine for Noema on **Linux, macOS and
Windows**: it installs a pinned Rust toolchain (via rustup), the
**Rumoca** Modelica compiler and **Pixi** into a managed directory,
then **builds Noema itself from source** and installs the resulting
binaries. One download, one command, nothing else touched.

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
  src/app/            # noema sources at the built commit
  noema/bin/{ui,pack} # the built Noema binaries
  downloads/          # download cache, pruned after a successful setup
```

## Pinned versions (`src/manifest.rs`)

| Component | Version | Source | Verified by |
| --- | --- | --- | --- |
| Rust toolchain | 1.89.0 (minimal profile) | static.rust-lang.org | official `rustup-init*.sha256` |
| Pixi | 0.82.0 | prefix-dev GitHub release | release `.sha256` / asset digests |
| Rumoca | 0.9.20 | CogniPilot GitHub release (prebuilt binaries, newer than crates.io) | release-page sha256 digests |
| Noema | `main` @ `NoemaSI/noema` (built from source) | GitHub source archive | commit resolved via API, digest recorded |

Every checksum-pinned download is verified before anything is executed.
Upgrading a component = one version bump + one checksum refresh in
`manifest.rs`, then re-run the Docker end-to-end test.

Windows ARM64: rust + pixi install normally; rumoca publishes no
`aarch64-windows` binary, so that component reports `unavailable`
instead of guessing.

## The `noema` component (source build)

`setup` resolves the requested ref (`--ref <branch|tag|sha>`, default
`main`) to an **exact commit** through the GitHub API, downloads the
immutable source archive for that commit, extracts it to
`<root>/src/app` and builds it with the **managed** cargo
(`cargo build --release -p ui -p pack`, list:
`NOEMA_BUILD_PACKAGES`). All of Noema's dependencies — `modelica` and
the vendored `reloaded-code` workspace — live inside the source tree,
so a single archive is self-sufficient. The binaries land in
`<root>/noema/bin/`, and `state.json` records the exact commit + source
archive digest.

- Idempotent: a rerun with an unchanged `main` **skips** (same resolved
  commit). New commits on `main` trigger a rebuild on the next run.
- First run compiles the whole workspace — expect 10–30 minutes and
  several GB of disk (crates.io downloads included); reruns are
  incremental only after the sources change.
- Private repo? Set `NOEMA_SETUP_TOKEN` (a GitHub token; sent to
  github.com hosts only).
- `verify` accepts whatever commit is installed;
  `verify --ref <branch|tag|sha>` requires that exact commit.

## CLI

```
noema-setup [setup] [--prefix DIR] [--only rust,pixi,rumoca,noema]
            [--ref <branch|tag|sha>] [--repo owner/name] [--dry-run] [--json]
noema-setup verify  [--prefix DIR] [--only ...] [--ref ...] [--json]
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

Note: a full `setup` now also **builds Noema from source** (10–30 min,
needs network for crates.io). For a quick toolchain-only check use:

```bash
docker run --rm -v noema-toolchain:/toolchain noema-setup setup --only rust,pixi,rumoca
```