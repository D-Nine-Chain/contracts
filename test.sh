#!/bin/bash

list=[\("5EYCAe5iKXhLT3vQD2JxgUKCdBEcYHrFq5RKmqQ4M82RYMfY",100\)]
manifest_path="./node-reward/Cargo.toml"

cargo contract call \
--contract "$NODE_REWARD_CONTRACT" \
-m update_rewards \
--args "105" $list \
--url "$D9_RUNTIME_TESTNET" \
--verbose \
-s "$D9_CONTRACT_WORKER" \
--manifest-path "$manifest_path"
