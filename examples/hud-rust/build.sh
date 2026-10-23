#!/usr/bin/env sh
# Builds the WASM component and places it where mod.toml expects it.
# Needs: rustup target add wasm32-wasip2
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
mkdir -p client
cp "${CARGO_TARGET_DIR:-target}"/wasm32-wasip2/release/hud_rust.wasm client/mod.wasm
echo "built client/mod.wasm"
