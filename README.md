# Subnet Rental Canister

## Running the Project
If you want to test the project locally, install `dfx` version 0.15.2 and use the following commands:

```bash
# Starts the replica, running in the background
dfx start --background

# Deploys your canisters to the replica and generates your candid interface
dfx deploy
```

To see the UI, run `npm start`, forward the port if necessary, and open http://<backend_canister_id>.localhost:8080/, likely http://bkyz2-fmaaa-aaaaa-qaaaq-cai.localhost:8080/

## Testing
To run the integration tests, first download PocketIC from [GitHub](https://github.com/dfinity/pocketic) and move the binary into [/src/subnet_rental_canister](/src/subnet_rental_canister/).
Then, build the subnet rental canister Wasm by running:

```bash
./scripts/build.sh
```
which will be placed in the root folder of the project.

Next, download the necessary NNS canister Wasms with:

```bash
./scripts/download_wasms.sh
```
A patched version of the CMC is already included in the project.

Finally, run the tests with:

```bash
cargo test --test integration_tests
```

## Reproducible Build
See [BUILD.md](BUILD.md) for instructions on how to build the canister Wasm reproducibly.
