# Subnet Rental Canister

## Running the Project
If you want to test your project locally, you can use the following commands:

```bash
# Starts the replica, running in the background
dfx start --background

# Deploys your canisters to the replica and generates your candid interface
dfx deploy
```

## Testing
To run the integration tests, first download PocketIC from [GitHub](https://github.com/dfinity/pocketic) and move the binary into [/src/subnet_rental_canister](/src/subnet_rental_canister/).
Then, build the canister Wasm by running:

```bash
./scripts/build.sh
```
which will be placed in the root folder of the project.
Finally, run the tests with:

```bash
cargo test --test integration_tests
```

## Reproducible Build
See [BUILD.md](BUILD.md) for instructions on how to build the canister Wasm reproducibly.
