#!/bin/bash

set -euo pipefail

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Build the subnet rental canister
dfx build --ic
cp .dfx/ic/canisters/subnet_rental_canister/subnet_rental_canister.wasm.gz subnet_rental_canister.wasm.gz
