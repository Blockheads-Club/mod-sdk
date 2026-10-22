# WASM mods

A WASM mod is a WebAssembly **component** of the `blockheads:api` world `server-mod` (or `client-mod`), version 0.2
(or the frozen 0.1 in `wit/compat/`, which the host still loads), declared in
`mod.toml` as `server = { runtime = "wasm", path = "server/mod.wasm" }`. Any language that can produce a component
works; this page covers Rust (with the SDK), C/C++/Zig and TinyGo, how the host runs components, and the
conformance suite that keeps the Luau and WASM runtimes identical.

## Rust: the `blockheads-mod` SDK

[`sdk/rust/blockheads-mod`](../sdk/rust/blockheads-mod) generates the bindings with `wit-bindgen` and adds a thin
layer so a Rust mod reads like its Luau twin: closures for events, commands and tasks, a `Command` builder and a
`Reply` type.

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
blockheads-mod = { git = "https://github.com/Blockheads-Club/mod-sdk" }
```

```rust
use blockheads_mod::events::On;
use blockheads_mod::prelude::*;

struct MyMod;

impl Mod for MyMod {
    fn on_enable() -> Result<(), String> {
        events::on_with(EventKind::Chat, On::priority(Priority::Low), |ev| {
            if ev.text().is_some_and(|t| t.contains("spam")) {
                ev.cancel();
            }
        })?;
        commands::register(Command::new("hello").alias("hi"), |sender, _label, _args| {
            Reply::lines([format!("Hello, {}!", sender.name())])
        })?;
        scheduler::run_every(0, 5 * 60 * scheduler::TICKS_PER_SECOND, || {
            let _ = players::broadcast("Five more minutes!");
        })?;
        Ok(())
    }
}

blockheads_mod::export_mod!(MyMod);
```

Build with `cargo build --release --target wasm32-wasip2` (after `rustup target add wasm32-wasip2`) and copy
`target/wasm32-wasip2/release/<name>.wasm` to `server/mod.wasm`. Notes:

- Handlers run one at a time on the host's main thread; keep state in the closures or in `thread_local!`.
- Player handles (`&Player`) are only valid during the call that received them. Keep `player.id()` and look the
  player up later with `players::get(&id)` (see `examples/scheduler-rust`).
- `println!` / `eprintln!` go to the mod's log (info / warn). A panic traps the instance: it is logged, counts as a
  fault and the instance cannot run again (the host disables the mod after 3 faults in 60 s or on reload).
- API 0.2: `events::on_custom` / `events::fire_custom`, `services::{provide, call}` (implement
  `Mod::call_service`), `channels::{open, send, close}` (implement `Mod::on_channel_message`), gameplay events with
  `EventCtx::set_reject_reason` / `set_damage`, `Player::pos` and value handles. Values are canonical CBOR.
- The raw generated bindings are in `blockheads_mod::bindings`.

Examples (each the twin of a Luau example): [`hello-rust`](../examples/hello-rust),
[`chat-filter-rust`](../examples/chat-filter-rust), [`scheduler-rust`](../examples/scheduler-rust).
`scripts/build-wasm.sh` builds them all.

## C, C++ and Zig

Generate C bindings with [`wit-bindgen`](https://github.com/bytecodealliance/wit-bindgen) (`cargo install
wit-bindgen-cli`):

```bash
wit-bindgen c wit/blockheads-api --world server-mod --out-dir bindings/
# -> server_mod.h, server_mod.c, server_mod_component_type.o
```

Implement the `exports_blockheads_api_guest_*` functions declared in `server_mod.h` (for example
`bool exports_blockheads_api_guest_on_enable(server_mod_string_t *err)`) and call the imports
(`blockheads_api_log_log(...)`, `blockheads_api_events_subscribe(...)`, ...). Build with
[wasi-sdk](https://github.com/WebAssembly/wasi-sdk) 25 or newer, which targets `wasm32-wasip2` directly:

```bash
$WASI_SDK/bin/clang --target=wasm32-wasip2 -O2 -mexec-model=reactor \
  -o mod.wasm my_mod.c bindings/server_mod.c bindings/server_mod_component_type.o
```

Zig uses the same header (`@cImport(@cInclude("server_mod.h"))`) and `zig build-exe -target wasm32-wasi
-fno-entry -rdynamic`, followed by `wasm-tools component new mod.wasm --adapt wasi_snapshot_preview1.reactor.wasm
-o mod.component.wasm` (the adapter ships with wasmtime releases). These toolchains are not part of CI yet.

## TinyGo

TinyGo 0.34+ builds components for `wasip2`. Generate Go bindings with `wit-bindgen go` (wit-bindgen-cli 0.63+) or
[`wit-bindgen-go`](https://github.com/bytecodealliance/go-modules):

```bash
go run go.bytecodealliance.org/cmd/wit-bindgen-go generate -o internal/ ./wit/blockheads-api
tinygo build -target=wasip2 --wit-package ./wit/blockheads-api --wit-world server-mod -o server/mod.wasm .
```

The SDK has no TinyGo example yet because no TinyGo toolchain was available where it was written; CI skips it.

## How the host runs components (`crates/bhmod-wasm`)

| | |
| --- | --- |
| Engine | wasmtime 50, component model. Cranelift (native code) on x86_64 / aarch64 / riscv64 / s390x; the **Pulley** interpreter on every other host (armv7, i686, …), whose bytecode Cranelift also emits. `WasmConfig.backend` forces one. |
| Time | Epoch interruption: a helper thread advances the epoch every 1 ms while a guest call runs; the store traps at the host's hard deadline (25 ms) → `CallError::Timeout`. Fuel is off. |
| Memory | `InstanceLimits.memory_bytes` (32 MiB by default) caps the instance's linear memories through `StoreLimits`; a failed `memory.grow` makes Rust's allocator abort, which is a trap. |
| WASI | p2 with an empty context: no files, network, environment or arguments; clocks and random work; stdout/stderr go to the log. |
| Versions | Components of `blockheads:api@0.2.0` and `@0.1.0` load side by side (two bindings, one implementation); a 0.1 mod never receives the 0.2-only events. |
| Traps | A trap or panic poisons the instance (the component model forbids re-entering it); later calls fail fast and count as faults. `on-enable` returning `err` disables the mod without a fault. |
| Cache | Compiled code is stored as `<cache_dir>/<sha256 of the .wasm>-<engine hash>.cwasm`; a later load only deserializes it (x86_64: 0.5 ms instead of ~230 ms for an 87 KB component). |
| Precompiled | With `WasmConfig.trust_precompiled`, a package may ship `<entry stem>.<target>.cwasm` next to the `.wasm`, for example `server/mod.pulley32.cwasm`, made on a build machine with `bhmod_wasm::precompile_for("pulley32", wasm)`. 32-bit devices then skip the on-device compile (2 s under qemu for the same component). Precompiled code is trusted like native code, so only enable this for packages you trust. Builds without the `compiler` feature (the future Android client loader) can only load such artifacts. |

AssemblyScript and other core-module (non-component) mods are not supported yet: they need the message ABI of the
design (`bh` / `env` imports) and are planned with the client loaders.

### Building hosts for 32-bit targets

- armv7 (Linux and Android) runs Pulley. Build with `RUSTFLAGS="--cfg pulley_assume_llvm_makes_tail_calls"`
  (1.3–2× faster interpretation); the server sets it in `.cargo/config.toml`.
- Android with the Luau runtime needs the static C++ runtime: `CXXSTDLIB_<target>=c++_static` and `-lc++abi` at link
  time (the server's `.cargo/config.toml` and build script do this).

### Numbers (hello-rust vs hello-lua, `ModHost` round trip, release build)

| | x86_64 Cranelift | x86_64 Pulley | x86_64 Luau | armv7 Pulley (qemu) | armv7 Luau (qemu) |
| --- | --- | --- | --- | --- | --- |
| chat event (1 handler, string work) | 5.1 µs | 21.7 µs | 14.9 µs | 63–80 µs | 85 µs |
| `/hello` command (alias, broadcast, reply) | 6.7 µs | 42 µs | 18.2 µs | 137–165 µs | 90 µs |
| engine create | 0.9 ms | | | 40–46 ms | |
| compile + instantiate, cold | 226 ms | | | 2.1 s | |
| cached / precompiled load + instantiate | 0.5 ms | | | 3.2–3.9 ms | |

qemu-user translates every instruction, so the armv7 numbers are only relative to each other; a real Cortex-A
device decides.

## Conformance suite

`crates/bhmod-wasm/tests/conformance.rs` runs the same scenarios against the Luau and the WASM twin of each example
(`hello-lua`/`hello-rust`, `chat-filter`/`chat-filter-rust`, `scheduler-lua`/`scheduler-rust`) through the real
`ModHost` and a fake game, and requires identical transcripts: command replies, event results, chat, broadcasts,
kicks and log lines (mod id masked). A new runtime (JVM, a client loader) adds its twins to the same table.

```bash
scripts/build-wasm.sh            # builds the Rust examples and the test guest (wasm32-wasip2)
cargo test -p bhmod-wasm         # runtime tests + conformance suite
```
