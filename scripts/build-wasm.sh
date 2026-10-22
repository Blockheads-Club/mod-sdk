#!/usr/bin/env sh
# Builds every Rust WASM component of the repository (examples and the bhmod-wasm test guest) for wasm32-wasip2 and
# puts each where its mod.toml / the tests expect it. The bhmod-wasm tests and the conformance suite need this first.
# Needs: rustup target add wasm32-wasip2
# Set CARGO_TARGET_DIR to share one build directory (much faster); defaults to target/wasm-guests.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/wasm-guests}"
# Coverage or sanitizer flags of the host build must not leak into the guests.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTDOCFLAGS CARGO_BUILD_RUSTFLAGS LLVM_PROFILE_FILE CARGO_LLVM_COV \
  CARGO_LLVM_COV_TARGET_DIR __CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER 2>/dev/null || true
locked="${LOCKED:-}"
for e in hello-rust chat-filter-rust scheduler-rust; do
  (cd "$root/examples/$e" && cargo build $locked --release --target wasm32-wasip2 -q)
  mkdir -p "$root/examples/$e/server"
  cp "$CARGO_TARGET_DIR/wasm32-wasip2/release/$(echo "$e" | tr - _).wasm" "$root/examples/$e/server/mod.wasm"
  echo "built examples/$e/server/mod.wasm"
done
g="$root/crates/bhmod-wasm/tests/guest"
(cd "$g" && cargo build $locked --release --target wasm32-wasip2 -q)
mkdir -p "$g/out"
cp "$CARGO_TARGET_DIR/wasm32-wasip2/release/bhmod_wasm_test_guest.wasm" "$g/out/guest.wasm"
echo "built crates/bhmod-wasm/tests/guest/out/guest.wasm"
