### Circle stablecoins on NEAR
Implementation of a Circle stablecoin on NEAR following [near-contract-standards](https://github.com/near/near-sdk-rs/tree/master/near-contract-standards).

### Rust v1.70.0 incompatibility issues
As of Rust v1.70.0, `.wasm` targets by default enable opcodes (specifically `sign-ext`, due to [an update to LLVM](https://releases.rs/docs/1.70.0/#internal-changes)) that are currently not supported by the NearVM. NEAR has an [open issue](https://github.com/near/nearcore/issues/8358#issuecomment-1383247423) to figure out how to work with this.
They suggest using a flag from `wasm-opt` to disable sign extensions.

We have decided to use Rust v1.69.0 for now.

#### Install Rust v1.69.0 for development
```
$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain=1.69.0 -y
```
or
```
$ brew install rustup
$ rustup install 1.69.0
```

### Set Rust v1.69.0 as default
```
$ rustup default 1.69.0
```

### Testing
```
$ cargo test
```
If your `integration_test.rs` is failing, ensure you are on Rust v1.69.0. Once you have done so, clean your project with `make clean` before re-running `make` and then `cargo test`.

### Build for deploying on-chain
```
$ make
```
Build code for production using local Rust installation (if no local Rust installation is found, Rust docker container will be used). The output file is `target/wasm32-unknown-unknown/release/fiat_token.wasm`.

```
$ make clean
```
Remove compiled artifacts.

### Add to PATH
```
$ source $HOME/.cargo/env
``` 
