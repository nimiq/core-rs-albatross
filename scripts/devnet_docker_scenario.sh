#!/bin/bash

MAX_VALIDATORS=4

usage()
{
cat << EOF
usage: $0 [-v|--validators COUNT] [-h|--help]

This script launches a scenario with N validators + spammer + seed node, each one running in its own docker container
This script should be executed from the root of your nimiq repository

OPTIONS:
   -h|--help       Show this message
   -v|--validators The number of validators, as a minimun 4 validators are created
EOF
}

while [ ! $# -eq 0 ]; do
    case "$1" in
        -v | --validators)
            if [ "$2" ]; then
                MAX_VALIDATORS=$2
                shift
            else
                echo '--validators requires a value'
                exit 1
            fi
            ;;
        -h | --help)
            usage
            exit
            ;;
        *)
            usage
            exit
            ;;
    esac
    shift
done

# Create devnet configuration
echo "Creating devnet configuration... "
python3 scripts/devnet_create.py $MAX_VALIDATORS

# Overwrite the docker compose and genesis
echo "Copying the genesis and docker compose files... "
cp -v /tmp/nimiq-devnet/dev-albatross.toml genesis/src/genesis/dev-albatross.toml
cp -v /tmp/nimiq-devnet/docker-compose.yml docker-compose.yml

#Compile the code
echo "Compiling the code"
cargo build --release

echo "Create docker images... "
docker buildx build . --pull -t core --progress=plain --build-arg BUILD=release
docker buildx build . --pull -t spammer --progress=plain --build-arg APP=spammer --build-arg BUILD=release

NETWORK_NAME=nimiq.local docker-compose -f docker-compose.yml up
