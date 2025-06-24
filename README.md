# Subnet Rental Canister

## Running the Project
If you want to test the project locally, install `dfx` version 0.27.0 or later and use the following commands:

```bash
# Starts the replica, running in the background
dfx start --background

# Deploys your canisters to the replica and generates your candid interface
dfx deploy
```

## Testing
Build the subnet rental canister Wasm by running:

```bash
./scripts/build.sh
```
which will be placed in the root folder of the project.

Next, download the necessary NNS canister Wasms with:

```bash
./scripts/download_wasms.sh
```
Finally, run the tests with:

```bash
cargo test --test integration_tests
```

## Reproducible Build
See [BUILD.md](BUILD.md) for instructions on how to build the canister Wasm reproducibly.
