#!/bin/bash

set -euo pipefail

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Build the Docker image
docker build --platform linux/amd64 -t src-builder .
# Run the container and build the subnet rental canister inside it
docker run --rm -v "$(pwd):/app" src-builder bash -c "dfx build --ic"
# Copy the Wasm file to the root directory
cp .dfx/ic/canisters/subnet_rental_canister/subnet_rental_canister.wasm.gz ./subnet_rental_canister.wasm.gz
