# Build the noema-setup binary from the workspace, then run a full
# hermetic setup inside the container. Nothing touches the host: the
# toolchain lives in the /toolchain volume only.
#
#   docker build -f crates/installer/docker/Dockerfile -t noema-setup .
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
# To test another distro or architecture, change the two base images
# below (e.g. --build-arg RUNTIME=ubuntu:24.04) and rebuild.

FROM rust:1.89-slim-bookworm AS build
WORKDIR /src
COPY . .
# Offline unit/integration tests: synthetic binaries, zero network.
RUN cargo test --release -p installer

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
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