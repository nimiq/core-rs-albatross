# syntax=docker/dockerfile:1
#
# Toolchain image to build the Nimiq web-client WASM locally, without installing
# Rust / Node / wasm tooling on the host. Used by docker-compose.yml ("build" service).
#
# It only sets up the toolchain; the repo is bind-mounted at run time and the build
# writes back to web-client/dist on the host. wasm-bindgen-cli is NOT baked in:
# build.sh installs the version pinned in the mounted Cargo.lock at run time (into
# the cargo target dir, i.e. the wasm-target volume), so bumping wasm-bindgen needs
# no image rebuild.

# Node.js is taken from the official image, because Debian bookworm's nodejs (18) is too
# old for the extras' dev dependencies (tsup needs >= 22.18, vitest >= 20).
FROM node:24-bookworm-slim AS node

FROM rust:1-bookworm

# clang provides the WebAssembly backend that cc-rs uses to compile C dependencies
# (e.g. zeroize) to wasm32. Debian's clang ships the wasm target; Apple clang on
# macOS does not, which is why building directly on the host fails.
RUN apt-get update \
    && apt-get install -y --no-install-recommends clang \
    && rm -rf /var/lib/apt/lists/*

# node/npm + yarn are needed by build.sh's launcher/lib steps (tsup), so that the full
# dist is regenerated consistently. They live only in this image.
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/npm
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && npm install -g yarn

RUN rustup target add wasm32-unknown-unknown

WORKDIR /repo/web-client
