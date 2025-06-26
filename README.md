# Subnet Rental Canister

The subnet rental canister is deployed on [ICP](https://dashboard.internetcomputer.org/canister/qvhpv-4qaaa-aaaaa-aaagq-cai) with the canister ID `qvhpv-4qaaa-aaaaa-aaagq-cai`.

## Running the Project
If you want to test the project locally, install `dfx` version 0.27.0 or later and the [Candid Extractor](https://github.com/dfinity/candid-extractor) and use the following commands:

```bash
# Starts the replica, running in the background
dfx start --clean --background

# Deploys your canisters to the replica and generates your candid interface
dfx deploy
```

## Testing
Build the subnet rental canister Wasm by running:

```bash
./scripts/build.sh
```
which will be placed in the root folder of the project.

Next, get the necessary NNS canister Wasms with:

```bash
./scripts/get_wasms.sh
```
Finally, run the tests with:

```bash
cargo test --test integration_tests
```

## Reproducible Build
See [BUILD.md](BUILD.md) for instructions on how to build the canister Wasm reproducibly.
