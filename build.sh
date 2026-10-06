#!/usr/bin/env bash

if ! command -v cargo >/dev/null 2>&1; then
	echo "error: cargo not found. Install Rust first." >&2
	exit 1
fi

# cargo install wasm-bindgen-cli
# rustup target add wasm32-unknown-unknown

cargo build --release --target wasm32-unknown-unknown --lib
wasm-bindgen --target web --out-dir public/pkg \
	target/wasm32-unknown-unknown/release/hachifont_tool.wasm

echo "done: public/pkg"
