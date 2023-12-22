#!/bin/bash

set -euo pipefail

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

# Build the Docker image and run it to build the canister
docker build -t subnet-rental-canister .
CONTAINER_ID=$(docker run --platform="linux/amd64" -d subnet-rental-canister)
docker cp $CONTAINER_ID:/subnet-rental-canister/target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm . 
docker rm -f $CONTAINER_ID
