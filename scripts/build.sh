#!/bin/bash

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Download ic-wasm
OSTYPE=$(uname -s) || OSTYPE=$OSTYPE
OSTYPE="${OSTYPE,,}"
RUNNER_OS="${RUNNER_OS:-}"
if [[ "$OSTYPE" == "linux"* || "$RUNNER_OS" == "Linux" ]]; then
  URL="https://github.com/dfinity/ic-wasm/releases/download/0.6.0/ic-wasm-linux64"
elif [[ "$OSTYPE" == "darwin"* || "$RUNNER_OS" == "macOS" ]]; then
  URL="https://github.com/dfinity/ic-wasm/releases/download/0.6.0/ic-wasm-macos"
else
  echo "OS not supported: ${OSTYPE:-$RUNNER_OS}"
  exit 1
fi
curl -sL "${URL}" -o ic-wasm || exit 1
chmod +x ic-wasm

# auto-create the candid interface
cargo install candid-extractor --version 0.1.3
candid-extractor subnet_rental_canister.wasm > src/subnet_rental_canister/subnet_rental_canister.did

# Build canister
export RUSTFLAGS="--remap-path-prefix $(readlink -f $(dirname ${0}))=/build --remap-path-prefix ${CARGO_HOME}=/cargo"
cargo rustc -p subnet_rental_canister --crate-type=cdylib --locked --target wasm32-unknown-unknown --release

# include the candid interface in the canister WASM
./ic-wasm ./target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm -o ./target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm metadata candid:service -f src/subnet_rental_canister/subnet_rental_canister.did -v public

# copy the canister WASM into the root directory
cp target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm subnet_rental_canister.wasm

# build the mock exchange rate canister for testing
cargo rustc -p xrc_mock --crate-type=cdylib --locked --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/xrc_mock.wasm src/subnet_rental_canister/tests/exchange-rate-canister.wasm
gzip -f src/subnet_rental_canister/tests/exchange-rate-canister.wasm 
