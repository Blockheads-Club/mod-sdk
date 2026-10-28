#!/usr/bin/env bash
# Builds every official example mod as a .bhmod package, reproducibly, into $OUT (default: dist/mods).
# Rust components are built with --locked and with absolute paths remapped, so the same source gives the same bytes
# on any machine. Prints a table "id version sha256 size" (SHA256SUMS in $OUT).
# `bhmod pack` is deterministic (fixed timestamps, sorted entries). The Rust bytes are identical for the same source
# checkout path (rustc derives symbol hashes from it), so rebuild at the same path to verify a published hash.
# Needs: rustup target add wasm32-wasip2, and bhmod (BHMOD=path, default: cargo run -p bhmod).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
out="${OUT:-$root/dist/mods}"
mkdir -p "$out"
if [ -z "${BHMOD:-}" ]; then
  cargo build --locked -q -p bhmod --manifest-path "$root/Cargo.toml"
  BHMOD="${CARGO_TARGET_DIR:-$root/target}/debug/bhmod"
fi
export CARGO_TARGET_DIR="${WASM_TARGET_DIR:-$root/target/official-mods}"
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTDOCFLAGS CARGO_BUILD_RUSTFLAGS RUSTC_WRAPPER 2>/dev/null || true
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$root=/mod-sdk$(printf '\x1f')--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo$(printf '\x1f')--remap-path-prefix=$(rustc --print sysroot)=/rust"
export SOURCE_DATE_EPOCH=0
: > "$out/SHA256SUMS"
for d in "$root"/examples/*/; do
  e="$(basename "$d")"
  if [ -f "$d/Cargo.toml" ]; then
    (cd "$d" && cargo build --locked --release --target wasm32-wasip2 -q)
    # Place the component at every wasm path mod.toml's [entry] names (server/mod.wasm and/or client/mod.wasm).
    rels="$(sed -n 's/^\(server\|client\) = { runtime = "wasm", path = "\(.*\)" }.*/\2/p' "$d/mod.toml")"
    [ -n "$rels" ] || { echo "$e: no wasm entry in mod.toml" >&2; exit 1; }
    for rel in $rels; do
      mkdir -p "$(dirname "$d/$rel")"
      cp "$CARGO_TARGET_DIR/wasm32-wasip2/release/$(echo "$e" | tr - _).wasm" "$d/$rel"
    done
  fi
  "$BHMOD" pack "$d" -o "$out/$e.tmp.bhmod" >/dev/null
  id="$(sed -n 's/^id = "\(.*\)"/\1/p' "$d/mod.toml" | head -1)"
  ver="$(sed -n 's/^version = "\(.*\)"/\1/p' "$d/mod.toml" | head -1)"
  mv "$out/$e.tmp.bhmod" "$out/$id-$ver.bhmod"
  (cd "$out" && sha256sum "$id-$ver.bhmod" >> SHA256SUMS)
  echo "$id $ver $(sha256sum "$out/$id-$ver.bhmod" | cut -d' ' -f1) $(stat -c %s "$out/$id-$ver.bhmod")"
done
