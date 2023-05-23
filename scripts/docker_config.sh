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
optional peer_key_file NIMIQ_PEER_KEY_FILE string
optional peer_key NIMIQ_PEER_KEY string

echo "listen_addresses = ["
addr=($LISTEN_ADDRESSES)
for node in "${addr[@]}"; do
    echo "\"$node\""
done
echo "]"


if [[ ! -z "$ADVERTISED_ADDRESSES" ]]; then
    echo "advertised_addresses = ["
    addr=($ADVERTISED_ADDRESSES)
    for node in "${addr[@]}"; do
        echo "\"$node\""
    done
    echo "]"
fi

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
optional sync_mode NIMIQ_SYNC_MODE string

echo '[database]'
entry path "/home/nimiq/database" string
optional size NIMIQ_DATABASE_SIZE number
optional max_dbs NIMIQ_MAX_DBS number

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

if [[ ! -z "${VALIDATOR_ADDRESS+x}" ]]; then
    echo '[validator]'
    required validator_address VALIDATOR_ADDRESS string
    optional voting_key_file VOTING_KEY_FILE string
    optional voting_key VOTING_KEY string
    optional fee_key_file FEE_KEY_FILE string
    optional fee_key FEE_KEY string
    optional signing_key_file SIGNING_KEY_FILE string
    optional signing_key SIGNING_KEY string
fi

if [[ "$RPC_ENABLED" == "true" ]]; then
    echo '[rpc-server]'
    entry bind 0.0.0.0 string
    optional username RPC_USERNAME string
    optional password RPC_PASSWORD string
fi

echo '[mempool]'
optional tx_fee MEMPOOL_TX_FEE number
optional tx_fee_per_byte MEMPOOL_TX_FEE_PER_BYTE number
optional tx_value MEMPOOL_TX_VALUE number
optional tx_value_total MEMPOOL_TX_VALUE_TOTAL number
optional contract_fee MEMPOOL_CONTRACT_FEE number
optional contract_fee_per_byte MEMPOOL_CONTRACT_FEE_PER_BYTE number
optional contract_value MEMPOOL_CONTRACT_VALUE number
optional creation_fee MEMPOOL_CREATION_FEE number
optional creation_fee_per_byte MEMPOOL_CREATION_FEE_PER_BYTE number
optional creation_value MEMPOOL_CREATION_VALUE number
optional recipient_balance MEMPOOL_RECIPIENT_BALANCE number
optional sender_balance MEMPOOL_SENDER_BALANCE number
