#!/bin/sh

set -e

# Ask for sudo password first
sudo service docker start

# Build config and docker-compose files
python scripts/devnet/devnet.py -t .github/devnet_topologies/four_validators.toml --env=docker_compose --networkname=webauthn.pos.nimiqwatch.com

# Build release binary and Docker image with it
cargo build --release --bin nimiq-client
docker build . -t core --progress=plain

# Copy generated genesis config into genesis builder to trigger a rebuild
cp .devnet/conf/dev-albatross.toml genesis/src/genesis/dev-albatross.toml

# Compile web-client files with generated genesis config
cd web-client
./scripts/build-web.sh && ./scripts/build-types.sh && ./scripts/build-launcher.sh
