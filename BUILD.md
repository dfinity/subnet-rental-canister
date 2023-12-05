# Build the Wasm Module

To build the wasm module, make sure you have docker installed and running.
Then, run the `docker_build.sh` script which builds the wasm and puts it in the root folder of the project.

```bash
./scripts/docker_build.sh
```

## Verify the Build

To verify the build, you can use the `sha256sum` command to verify the hash of the wasm file:
```bash
sha256sum subnet_rental_canister.wasm 
```
or on macOS with `shasum`:

```bash
shasum -a 256 subnet_rental_canister.wasm
```
