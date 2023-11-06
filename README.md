### Circle stablecoins on NEAR
Implementation of a Circle stablecoin on NEAR following [near-contract-standards](https://github.com/near/near-sdk-rs/tree/master/near-contract-standards).

#### Build
```
$ make
```
Build code for production using local Rust installation (if no local Rust installation is found, Rust docker container will be used). The output file is `target/wasm32-unknown-unknown/release/fiat_token.wasm`.

```
$ make clean
```
Remove compiled artifacts.

#### Install Rust v1.69.0 for development
```
$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain=1.69.0 -y
```
or
```
$ brew install rustup
$ rustup install 1.69.0
```

Add to PATH
```
$ source $HOME/.cargo/env
``` 
