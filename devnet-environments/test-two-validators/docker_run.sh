#!/bin/bash

set -e

mkfifo /root/nimiq.log.pipe || true
cat /root/nimiq.log.pipe &
mkdir -p /root/.nimiq

function hex2bin () {
    sed 's/\([0-9A-F]\{2\}\)/\\\\\\x\1/gI' | xargs printf
}

if [[ ! -z "$NIMIQ_PEER_KEY" ]]; then
    export NIMIQ_PEER_KEY_FILE=/root/.nimiq/peer_key.dat
    echo "$NIMIQ_PEER_KEY" | hex2bin > $NIMIQ_PEER_KEY_FILE
fi

if [[ ! -z "$VALIDATOR_KEY" ]]; then
    export VALIDATOR_KEY_FILE=/root/.nimiq/validator_key.dat
    echo "$VALIDATOR_KEY" | hex2bin > $VALIDATOR_KEY_FILE
fi

./docker_config.sh > /root/.nimiq/client.toml

/bin/nimiq-client $@
