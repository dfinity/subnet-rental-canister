#!/bin/bash

set -euo pipefail

# Make sure we always run from the root
SCRIPTS_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPTS_DIR/.."

docker build -t subnet-rental-canister .
CONTAINER_ID=$(docker run --platform="linux/amd64" -d --rm subnet-rental-canister)
docker cp $(echo $CONTAINER_ID):/subnet-rental-canister/target/wasm32-unknown-unknown/release/subnet_rental_canister.wasm . 
