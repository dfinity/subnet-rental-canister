#!/bin/bash

set -euo pipefail

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Build the Docker image and run it to build the canister
docker build --platform linux/amd64 -t subnet-rental-canister .
# Run the container and execute dfx build inside it
docker run --rm -v "$(pwd):/app" subnet-rental-canister bash -c "dfx build --ic"
# Copy the Wasm file to the root directory
cp .dfx/ic/canisters/subnet_rental_canister/subnet_rental_canister.wasm.gz ./subnet_rental_canister.wasm.gz
