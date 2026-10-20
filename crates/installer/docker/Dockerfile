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

FROM rust:1.89-slim-bookworm AS build
WORKDIR /src
COPY . .
# Offline unit/integration tests: synthetic binaries, zero network.
RUN cargo test --release && cargo build --release

FROM debian:bookworm-slim AS runtime
# gcc + libc6-dev: the native linker/compiler rustc needs to build
# plugin code. Without them the prereq check (correctly) warns that
# this machine cannot compile — on a real host the user installs them
# via their package manager, in the container we bake them in.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/noema-setup /usr/local/bin/noema-setup

# Everything the installer writes stays under /toolchain.
ENV NOEMA_TOOLCHAIN_DIR=/toolchain
VOLUME /toolchain

# Default: full setup followed by verification.
ENTRYPOINT ["/usr/local/bin/noema-setup"]
CMD ["setup"]

# Convenience aliases are plain CLI subcommands, e.g.:
#   docker run --rm noema-setup verify
#   docker run --rm noema-setup setup --dry-run
# For a shell inside the container, override the entrypoint:
#   docker run --rm -it --entrypoint /bin/sh noema-setup