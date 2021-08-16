# syntax = docker/dockerfile:1.2

# Builds a light albatross image from sources.
# Requires Docker buildkit.

ARG RUST_IMAGE=rust:1-alpine
FROM $RUST_IMAGE AS builder

# Switch to Rust nightly
RUN rustup update nightly; rustup default nightly;

# Fetch dependencies.
RUN apk update && apk add musl-dev libretls-dev protoc

# Copy sources.
COPY . /build
WORKDIR /build

# Build.
RUN \
  --mount=type=cache,target=/build/target \
  # See https://doc.rust-lang.org/cargo/guide/cargo-home.html
  --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo build --bin nimiq-client && cp /build/target/debug/nimiq-client /build/

# This is where the final image is created
FROM alpine

# Bash is required to run the start scripts
RUN apk add --no-cache bash

# Run as unprivileged user.
RUN adduser --disabled-password --home /home/nimiq --shell /bin/bash --uid 1001 nimiq
USER nimiq

WORKDIR /home/nimiq

# Create .nimiq, so that it has the right permissions. docker-compose might want to create this
# because we want to use this as a volume, but we don't want this directory to be owned by root.
RUN mkdir -p /home/nimiq/.nimiq
VOLUME /home/nimiq/.nimiq

# Pull necessary files from builder image
COPY --chown=root:root --from=builder /build/nimiq-client /usr/local/bin/nimiq-client
COPY ./scripts/docker_*.sh /home/nimiq/

EXPOSE 8443/tcp 8648/tcp

ENTRYPOINT [ "/bin/bash" ]
CMD [ "/home/nimiq/docker_run.sh" ]


# https://github.com/opencontainers/image-spec/blob/master/annotations.md
LABEL \
  org.opencontainers.image.title="Nimiq Albatross Alpine" \
  org.opencontainers.image.description="Rust implementation of the Nimiq Blockchain Core Albatross Branch (Buildkit Alpine image)" \
  org.opencontainers.image.url="https://github.com/nimiq/core-rs-albatross" \
  org.opencontainers.image.vendor="Nimiq Foundation" \
  org.opencontainers.image.licenses="Apache-2.0"
