#!/bin/bash

# Rust build:
export RUSTFLAGS="--remap-path-prefix $(readlink -f $(dirname ${0}))=/build --remap-path-prefix ${CARGO_HOME}=/cargo"
cargo build --locked --target wasm32-unknown-unknown --release
ic-wasm target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm -o subnet_rental_canister.wasm shrink
