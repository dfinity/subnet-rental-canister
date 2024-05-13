# Build the Wasm Module

To build the wasm module, make sure you have docker version 26.1.1 or later installed and running.
Then, run the `docker_build.sh` script which builds the wasm and puts it in the root folder of the project.

```bash
./scripts/docker_build.sh
```

## Verify the Build

To verify the build, you can use the `shasum` command to calculate the hash of the wasm file:

```bash
shasum -a 256 subnet_rental_canister.wasm
```
