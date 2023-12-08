#!/bin/bash

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Build canister
export RUSTFLAGS="--remap-path-prefix $(readlink -f $(dirname ${0}))=/build --remap-path-prefix ${CARGO_HOME}=/cargo"
cargo rustc -p subnet_rental_canister --crate-type=cdylib --locked --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm subnet_rental_canister.wasm
