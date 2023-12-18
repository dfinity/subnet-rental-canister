#!/bin/bash

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

COMMIT="41805f62cb4758ca30adec9bc4af936cf700e968"
DESTINATION_DIR="src/subnet_rental_canister/tests"
curl --output-dir $DESTINATION_DIR -sLO https://download.dfinity.systems/ic/$COMMIT/canisters/ledger-canister.wasm.gz
