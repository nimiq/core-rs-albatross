#!/bin/bash

set -e

mkdir -p /home/nimiq/.nimiq

if [[ -z "$NIMIQ_HOST" ]]; then
    export NIMIQ_HOST=$(hostname -i)
fi

if [[ ! -e "/home/nimiq/.nimiq/client.toml" || $OVERRIDE_CONFIG_FILE -eq 1 ]]; then
    ./docker_config.sh > /home/nimiq/.nimiq/client.toml
fi
export NIMIQ_OVERRIDE_DEVNET_CONFIG=/home/nimiq/dev-albatross-4-validators.toml
/usr/local/bin/nimiq-client $@
