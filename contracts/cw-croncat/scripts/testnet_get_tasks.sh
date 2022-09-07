#!/bin/bash

cd "$(dirname "$0")"
. ./testnet_init_vars.sh

if [ -z "$1" ]
then
    echo "Must provide contract address"
    exit 1
else 
    CONTRACT=$1
fi

GET_TASKS='{"get_tasks":{}}'
junod query wasm contract-state smart $CONTRACT "$GET_TASKS" $NODE
