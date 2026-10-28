#!/usr/bin/env sh
# Builds the WASM component and places it where mod.toml expects it: the same component serves both sides (it checks
# host::current_side()).
# Needs: rustup target add wasm32-wasip2
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
mkdir -p server client
cp "${CARGO_TARGET_DIR:-target}"/wasm32-wasip2/release/voice_rust.wasm server/mod.wasm
cp "${CARGO_TARGET_DIR:-target}"/wasm32-wasip2/release/voice_rust.wasm client/mod.wasm
echo "built server/mod.wasm and client/mod.wasm"
