#!/bin/bash

set -euo pipefail

CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Determine URLs for ic-wasm and candid-extractor
OSTYPE=$(uname -s) || OSTYPE=$OSTYPE
OSTYPE=$(echo $OSTYPE | tr '[:upper:]' '[:lower:]')
RUNNER_OS="${RUNNER_OS:-}"
if [[ "$OSTYPE" == "linux"* || "$RUNNER_OS" == "Linux" ]]; then
  URL_IC_WASM="https://github.com/dfinity/ic-wasm/releases/download/0.6.0/ic-wasm-linux64"
  URL_CANDID_EXTRACTOR="https://github.com/dfinity/cdk-rs/releases/download/candid-extractor-v0.1.3/candid-extractor-x86_64-unknown-linux-gnu.tar.gz"
elif [[ "$OSTYPE" == "darwin"* || "$RUNNER_OS" == "macOS" ]]; then
  URL_IC_WASM="https://github.com/dfinity/ic-wasm/releases/download/0.6.0/ic-wasm-macos"
  URL_CANDID_EXTRACTOR="https://github.com/dfinity/cdk-rs/releases/download/candid-extractor-v0.1.3/candid-extractor-x86_64-apple-darwin.tar.gz"
else
  echo "OS not supported: ${OSTYPE:-$RUNNER_OS}"
  exit 1
fi

# Download ic-wasm and candid-extractor
curl -sL "${URL_IC_WASM}" -o ic-wasm
chmod +x ic-wasm

curl -sL "${URL_CANDID_EXTRACTOR}" -o candid-extractor.tar.gz
tar -xzf candid-extractor.tar.gz
chmod +x candid-extractor

# Build canister
export RUSTFLAGS="--remap-path-prefix $(readlink -f $(dirname ${0}))=/build --remap-path-prefix ${CARGO_HOME}=/cargo"
cargo rustc -p subnet_rental_canister --crate-type=cdylib --locked --target wasm32-unknown-unknown --release

# auto-create the candid interface
./candid-extractor ./target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm > src/subnet_rental_canister/subnet_rental_canister.did

# include the candid interface in the canister WASM
./ic-wasm ./target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm -o ./target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm metadata candid:service -f src/subnet_rental_canister/subnet_rental_canister.did -v public

# copy the canister WASM into the root directory
cp target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm subnet_rental_canister.wasm

# build the mock exchange rate canister for testing
cargo rustc -p xrc_mock --crate-type=cdylib --locked --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/xrc_mock.wasm src/subnet_rental_canister/tests/exchange-rate-canister.wasm
gzip -f src/subnet_rental_canister/tests/exchange-rate-canister.wasm 
