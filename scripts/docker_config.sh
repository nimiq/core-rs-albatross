#!/bin/bash

function entry () {
    if [[ "$3" == "string" ]]; then
        echo "$1 = \"$2\""
    else
        echo "$1 = $2"
    fi
}

function required () {
    local var=${!2}
    if [[ -z "$var" ]]; then
        echo "\$$2 required but not present" >&2
        exit 1
    fi
    entry "$1" "$var" "$3"
}

function optional () {
    local var=${!2}
    if [[ ! -z "$var" ]]; then
        entry "$1" "$var" "$3"
    fi
}

echo '[network]'
required min_peers NIMIQ_MIN_PEERS number
required peer_key_file NIMIQ_PEER_KEY_FILE string
optional peer_key NIMIQ_PEER_KEY string

echo "listen_addresses = ["
addr=($LISTEN_ADDRESSES)
for node in "${addr[@]}"; do
    echo "\"$node\""
done
echo "]"


nodes_arr=($NIMIQ_SEED_NODES)
for node in "${nodes_arr[@]}"; do
    echo "[[network.seed_nodes]]"
    echo "address = \"$node\""
done
if [[ ! -z "$NIMIQ_SEED_LIST" ]]; then
    echo "[[network.seed_nodes]]"
    echo "list = \"$NIMIQ_SEED_LIST\""
    if [[ ! -z "$NIMIQ_SEED_LIST_PUBKEY" ]]; then
        echo "public_key = \"$NIMIQ_SEED_LIST_PUBKEY\""
    fi
fi

echo '[consensus]'
required network NIMIQ_NETWORK string

echo '[database]'
entry path "/home/nimiq/database" string
optional size NIMIQ_DATABASE_SIZE number
optional max_dbs NIMIQ_MAX_DBS number
optional no_lmdb_sync NIMIQ_NO_LMDB_SYNC boolean

echo '[log]'
optional level NIMIQ_LOG_LEVEL string
optional timestamps NIMIQ_LOG_TIMESTAMPS boolean
optional tags NIMIQ_LOG_TAGS object
optional statistics NIMIQ_LOG_STATISTICS number
optional file NIMIQ_LOG_FILE string

if [[ "$ROTATING_LOG_ENABLED" == "true" ]]; then
    echo '[log.rotating_trace_log]'
    optional path ROTATING_LOG_PATH string
    optional size ROTATING_LOG_SIZE number
    optional file_count ROTATING_LOG_FILE_COUNT number
fi

echo '[validator]'
optional validator_key_file VALIDATOR_KEY_FILE string
optional validator_key VALIDATOR_KEY string
optional cold_key_file COLD_KEY_FILE string
optional cold_key COLD_KEY string
optional warm_key_file WARM_KEY_FILE string
optional warm_key WARM_KEY string

if [[ "$RPC_ENABLED" == "true" ]]; then
    echo '[rpc-server]'
    entry bind 0.0.0.0 string
    optional username RPC_USERNAME string
    optional password RPC_PASSWORD string
fi


