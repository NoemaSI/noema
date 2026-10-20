# Build the noema-setup binary and run a full hermetic setup inside
# the container. Nothing touches the host: the toolchain lives in the
# /toolchain volume only.
#
# The installer is a standalone cargo workspace (no dependency on the
# main noema workspace or its external path deps), so the build
# context is just crates/installer:
#
#   docker build -f crates/installer/docker/Dockerfile -t noema-setup crates/installer
#
# The build stage runs the *offline* test suite (stubbed fetcher, no
# network). Setup + verify run when you start the container:
#
#   docker run --rm -v noema-toolchain:/toolchain noema-setup setup --dry-run
#   docker run --rm -v noema-toolchain:/toolchain noema-setup setup
#   docker run --rm -v noema-toolchain:/toolchain noema-setup verify
#   docker run --rm -it -v noema-toolchain:/toolchain \
#     --entrypoint /bin/sh noema-setup
#
# To test another distro or architecture, change the base images
# below and rebuild.

FROM rust:1.99-slim-bookworm AS build
WORKDIR /src
COPY . .
# Offline unit/integration tests: synthetic binaries, zero network.
RUN cargo test --release && cargo build --release

FROM debian:bookworm-slim AS runtime
# System build dependencies for compiling Noema in this container
# (derived from the workspace's -sys crates):
#   build-essential      cc + c++ (aws-lc-sys, zstd-sys, plugin compiles)
#   pkg-config libssl-dev  openssl-sys / native-tls
#   libfontconfig1-dev libfreetype-dev  gpui text stack
#   libclang-dev         bindgen-driven build scripts
#   cmake                aws-lc-sys / onnx build scripts
# On a real host the user installs the equivalent via their package
# manager; the prereq check warns about the core pieces up front.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       build-essential pkg-config libssl-dev libfontconfig1-dev libfreetype-dev \
       libclang-dev cmake ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/noema-setup /usr/local/bin/noema-setup

# Everything the installer writes stays under /toolchain.
ENV NOEMA_TOOLCHAIN_DIR=/toolchain
VOLUME /toolchain

# Default: full setup (toolchain + Noema source build; 10-30 min)
# followed by verification. For a quick toolchain-only check:
#   docker run --rm -v noema-toolchain:/toolchain noema-setup setup --only rust,pixi,rumoca
ENTRYPOINT ["/usr/local/bin/noema-setup"]
CMD ["setup"]

# Convenience aliases are plain CLI subcommands, e.g.:
#   docker run --rm noema-setup verify
#   docker run --rm noema-setup setup --dry-run
# For a shell inside the container, override the entrypoint:
#   docker run --rm -it --entrypoint /bin/sh noema-setup