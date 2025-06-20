#!/bin/bash

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

COMMIT="ac7ff452684f84ea0cfc3fd0a27228220a368b33" # Jun 17, 2025
DESTINATION_DIR="src/subnet_rental_canister/tests"
curl --output-dir $DESTINATION_DIR -sLO https://download.dfinity.systems/ic/$COMMIT/canisters/ledger-canister.wasm.gz
