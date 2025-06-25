#!/bin/bash

set -euo pipefail

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Build the XRC mock canister
cargo build -p xrc_mock --locked --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/xrc_mock.wasm src/subnet_rental_canister/tests/exchange-rate-canister.wasm

# Build the subnet rental canister
dfx build --ic
cp target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm subnet_rental_canister.wasm
