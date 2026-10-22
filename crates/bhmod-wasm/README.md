# bhmod-wasm

**Licence: GPL-3.0-or-later** (with the Mod API Exception in [`../../LICENSING.md`](../../LICENSING.md)).

The WASM component runtime of the mod host: `WasmRuntime` implements `bhmod_core::Runtime` for `runtime = "wasm"`
mods. wasmtime 50 with Cranelift on 64-bit hosts and the Pulley interpreter on 32-bit ones, epoch deadlines, memory
limits, a `.cwasm` compile cache and optional trusted precompiled artifacts (`server/mod.pulley32.cwasm`). Details and
numbers: [`docs/wasm.md`](../../docs/wasm.md).

```rust
let mut opts = bhmod_core::HostOptions::new(mods_dir, Side::Server);
opts.runtimes.push(Box::new(bhmod_wasm::WasmRuntime::new(bhmod_wasm::WasmConfig {
    cache_dir: Some(mods_dir.join(".cache")),
    ..Default::default()
})?));
```

| Feature | |
| --- | --- |
| `compiler` (default) | Compile `.wasm` on this host (Cranelift). Without it only trusted precompiled artifacts load. |

Tests need the guest components: run `scripts/build-wasm.sh` first, then `cargo test -p bhmod-wasm` (runtime tests
with `tests/guest`, and the Luau ⇄ WASM conformance suite). `cargo llvm-cov -p bhmod-wasm` is at 100 % lines.
