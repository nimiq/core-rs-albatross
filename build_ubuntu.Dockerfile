# syntax = docker/dockerfile:1.2

# Builds an albatross image from sources.
# Requires Docker buildkit.

ARG RUST_IMAGE=rust:slim
FROM $RUST_IMAGE AS builder

# Switch to Rust nightly
RUN rustup update nightly && rustup default nightly

# Fetch dependencies.
RUN apt-get update \
    && apt-get --no-install-recommends -y install libssl-dev pkg-config clang\
    && rm -rf /var/lib/apt/lists/*

# Copy sources.
COPY . /build
WORKDIR /build

# Build.
RUN \
  --mount=type=cache,target=/build/target \
  # See https://doc.rust-lang.org/cargo/guide/cargo-home.html
  --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  NIMIQ_OVERRIDE_DEVNET_CONFIG=$PWD/genesis/src/genesis/dev-albatross-4-validators.toml cargo build --bin nimiq-client && cp /build/target/debug/nimiq-client /build/

# This is where the final image is created
FROM ubuntu:20.04

# Install dependencies and tini.
RUN apt-get update \
    && apt-get --no-install-recommends -y install libssl1.1 tini \
    && rm -rf /var/lib/apt/lists/*

# Run as unprivileged user.
RUN groupadd --system --gid 1001 nimiq \
    && adduser --system --home /home/nimiq --uid 1001 --gid 1001 nimiq
USER nimiq

WORKDIR /home/nimiq

# Create .nimiq, so that it has the right permissions. docker-compose might want to create this
# because we want to use this as a volume, but we don't want this directory to be owned by root.
RUN mkdir -p /home/nimiq/.nimiq
VOLUME /home/nimiq/.nimiq

COPY ./scripts/docker_*.sh /home/nimiq/

# Pull necessary files from builder image
COPY --chown=root:root --from=builder /build/nimiq-client /usr/local/bin/nimiq-client

EXPOSE 8443/tcp 8648/tcp

ENTRYPOINT [ "/usr/bin/tini", "--", "/home/nimiq/docker_run.sh" ]


# https://github.com/opencontainers/image-spec/blob/master/annotations.md
LABEL \
  org.opencontainers.image.title="Nimiq core-rs-albatross" \
  org.opencontainers.image.description="Rust implementation of the Nimiq Blockchain Core Albatross Branch (Buildkit Ubuntu image)" \
  org.opencontainers.image.url="https://github.com/nimiq/core-rs-albatross" \
  org.opencontainers.image.vendor="Nimiq Foundation" \
  org.opencontainers.image.licenses="Apache-2.0"
