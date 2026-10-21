#!/usr/bin/env sh
# Builds the WASM component into server/mod.wasm. Needs: rustup target add wasm32-wasip2
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
mkdir -p server
cp target/wasm32-wasip2/release/{{crate}}.wasm server/mod.wasm
echo "built server/mod.wasm"
