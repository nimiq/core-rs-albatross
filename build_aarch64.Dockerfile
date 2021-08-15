# syntax = docker/dockerfile:1.2

# Builds a light Buster image from sources.
# Requires Docker buildkit.

ARG RUST_IMAGE=arm64v8/rust:slim-buster
FROM $RUST_IMAGE AS builder
RUN rustup install nightly

# Fetch dependencies.
RUN apt-get update && apt-get install -y libssl-dev pkg-config

# Copy sources.
COPY . /build
WORKDIR /build

# Build.
RUN \
  --mount=type=cache,target=/build/target \
  # See https://doc.rust-lang.org/cargo/guide/cargo-home.html
  --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo +nightly build --bin nimiq-client && cp /build/target/debug/nimiq-client /build/

# Light stage.
FROM ubuntu:20.04

# Install dependencies.
RUN apt-get update && apt-get install -y libssl1.1

# Run as unprivileged user.
RUN adduser --disabled-password --home /home/nimiq --shell /bin/bash --uid 1001 nimiq
USER nimiq

# Pull image from builder.
COPY --chown=root:root --from=builder /build/nimiq-client /usr/local/bin/nimiq-client
COPY ./scripts/docker_*.sh /home/nimiq/

WORKDIR /home/nimiq

# Create .nimiq, so that it has the right permissions. docker-compose might want to create this
# because we want to use this as a volume, but we don't want this directory to be owned by root.
RUN mkdir -p /home/nimiq/.nimiq
VOLUME /home/nimiq/.nimiq

EXPOSE 8443/tcp 8648/tcp

#ENTRYPOINT ["/usr/local/bin/nimiq-client"]
ENTRYPOINT [ "/bin/bash" ]
CMD [ "/home/nimiq/docker_run.sh" ]


# https://github.com/opencontainers/image-spec/blob/master/annotations.md
LABEL \
  org.opencontainers.image.title="Nimiq core-rs-albatross" \
  org.opencontainers.image.description="Rust implementation of the Nimiq Blockchain Core Albatross Branch (Buildkit Buster ARM64/AArch64 image)" \
  org.opencontainers.image.url="https://github.com/nimiq/core-rs-albatross" \
  org.opencontainers.image.vendor="Nimiq Foundation" \
  org.opencontainers.image.licenses="Apache-2.0"
