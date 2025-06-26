#!/bin/bash

set -euo pipefail

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Build the Docker image and run it to build the canister
docker build -t localhost/subnet-rental-canister .
# Run the container and execute dfx build inside it
docker run --name subnet-rental-canister-build --platform="linux/amd64" -v "$(pwd):/app" localhost/subnet-rental-canister bash -c "dfx build --ic"
# Copy the Wasm file to the root directory
docker cp subnet-rental-canister-build:/app/.dfx/ic/canisters/subnet_rental_canister/subnet_rental_canister.wasm.gz ./subnet_rental_canister.wasm.gz
# Cleanup
docker rm -f subnet-rental-canister-build > /dev/null
