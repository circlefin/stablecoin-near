# Since this is the first target in the Makefile, running just `make` will execute this one.
target/wasm32-unknown-unknown/release/fiat_token.wasm: src/*.rs Cargo.toml Cargo.lock
	(make clean)
	if ! [ -x "$$(command -v rustup)" ]; then \
		echo "rustup not detected. Building and pulling Rust Docker image..."; \
		docker run --rm --user "$(id -u)":"$(id -g)" -v $(CURDIR):/usr/src/near-token -w /usr/src/near-token rust:1.69.0-buster /bin/bash -c "rustup target add wasm32-unknown-unknown && cargo build --all --target wasm32-unknown-unknown --release"; \
	else \
		echo "rustup detected. Building with local rust installation..."; \
		rustup target add wasm32-unknown-unknown && cargo build --all --target wasm32-unknown-unknown --release; \
	fi
	cp target/wasm32-unknown-unknown/release/*.wasm ./tests/data

test:
	cargo test --test-threads=3

clean:
	rm -rf target

all: target/wasm32-unknown-unknown/release/fiat_token.wasm
