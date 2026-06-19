# syntax=docker/dockerfile:1
#
# Toolchain image to build the Nimiq web-client WASM locally, without installing
# Rust / Node / wasm tooling on the host. Used by docker-compose.yml ("build" service).
#
# It only sets up the toolchain; the repo is bind-mounted at run time and the build
# writes back to web-client/dist on the host.
FROM rust:1-bookworm

# clang provides the WebAssembly backend that cc-rs uses to compile C dependencies
# (e.g. clear_on_drop) to wasm32. Debian's clang ships the wasm target; Apple clang on
# macOS does not, which is why building directly on the host fails.
# nodejs/npm + yarn (via corepack) are needed by build.sh's launcher/lib steps (tsup),
# so that the full dist is regenerated consistently. They live only in this image.
RUN apt-get update \
    && apt-get install -y --no-install-recommends clang nodejs npm \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g yarn

RUN rustup target add wasm32-unknown-unknown

# Pin wasm-bindgen-cli to the version locked in Cargo.lock (matches CI).
ARG WASM_BINDGEN_VERSION=0.2.118
RUN cargo install --locked wasm-bindgen-cli@${WASM_BINDGEN_VERSION}

WORKDIR /repo/web-client
