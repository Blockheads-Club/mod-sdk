<h1 align="center">Blockheads Mod SDK</h1>

<p align="center">
  Make mods for <b>The Blockheads</b>: new commands, chat tools, minigames and more, in Lua or Rust. 🧩
</p>

<p align="center">
  <img src="https://img.shields.io/maintenance/yes/2026?style=for-the-badge" alt="Maintained">
  <a href="https://github.com/Blockheads-Club/mod-sdk/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Blockheads-Club/mod-sdk/ci.yml?branch=dev&style=for-the-badge&label=CI" alt="CI"></a>
  <a href="LICENSING.md"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=for-the-badge" alt="License"></a>
</p>

---

## 🚀 Project Status
**Current Status:** Server mods in Lua and WASM (milestones M1 and M2). This repo holds the mod API, the file formats,
the network protocol, the `bhmod` tool, the shared mod host and the Rust SDK; the
[Blockheads Server](https://github.com/Blockheads-Club/blockheads-server) runs Lua and WASM mods with it. Things may
still change before 1.0.

Mods can run on the server, on the client, or both. Players who join a modded server see the list of mods, accept,
and the client part downloads straight from the server.

---

## 📦 Other Blockheads Club Repositories
- [**blockheads-server**](https://github.com/Blockheads-Club/blockheads-server) - Host your own Blockheads server
- [**blockheads-pc**](https://github.com/Blockheads-Club/blockheads-pc) - Play The Blockheads on your computer
- [**blockheads-patcher**](https://github.com/Blockheads-Club/blockheads-patcher) - Fix crashes and add small improvements to the game on your phone
- [**website**](https://github.com/Blockheads-Club/website) - The [Blockheads Club website](https://blockheads.club): downloads, guides and docs

---

## ⚡ Your first mod
Install the tool (needs [Rust](https://rustup.rs)):

```bash
cargo install --git https://github.com/Blockheads-Club/mod-sdk bhmod
```

**Lua (easiest):**

```bash
bhmod new my-mod             # creates my-mod/ with mod.toml and server/main.luau
cd my-mod
bhmod check .                # finds mistakes in mod.toml and the files
bhmod pack .                 # builds my-mod-0.1.0.bhmod
```

**Rust (WASM):**

```bash
rustup target add wasm32-wasip2
bhmod new my-mod --lang rust
cd my-mod && ./build.sh && bhmod pack .
```

Copy the `.bhmod` (or the unpacked mod folder) into the server's mods folder (`<data dir>/mods/`, or
`BLOCKHEADS_MODS_DIR`) and grant the permissions it asks for in `mods.toml` there:

```toml
[hello-lua]
grant = ["players:chat"]
```

Lua mods run on the server since milestone M1, WASM mods since M2. Every example comes as a Lua and a Rust twin
that behave exactly the same (a test checks it): [`hello-lua`](examples/hello-lua) /
[`hello-rust`](examples/hello-rust) (`/hello`), [`chat-filter`](examples/chat-filter) /
[`chat-filter-rust`](examples/chat-filter-rust) (cancel and rewrite chat), [`scheduler-lua`](examples/scheduler-lua) /
[`scheduler-rust`](examples/scheduler-rust) (`/countdown`, `/remind`). Rust, C, Zig and Go: [`docs/wasm.md`](docs/wasm.md).

A client mod can call a service of a server mod over the network (API 0.3.1, asynchronous):
[`prices-lua`](examples/prices-lua) is a price check with a server half that provides `prices` (`remote = true`) and a client
half (`/price <item>`), [`prices-rust`](examples/prices-rust) is the Rust twin of the client half.
The `bhmod new` templates (Luau, Rust, Java) carry a commented-out example of a `remote = true` service in `mod.toml` and the
entry file, and a README section with the limits.

Content mods (API 0.3) declare textures in `[content]`: [`cosmetics-hats`](examples/cosmetics-hats) is a hat and hair
set worn with `/hat` and `/hair` (see [`docs/content.md`](docs/content.md)); [`weapon-skins`](examples/weapon-skins) re-skins the held tool
(a golden spade and a flame pickaxe, PC client, native tier).

Java and Kotlin (milestone M9) are unsandboxed, so they need the `native` permission and a host with JVM support
turned on: `bhmod new my-mod --lang java`, the examples ending in `-java` (`shop-java` calls the Lua `economy`
service) and [`docs/jvm.md`](docs/jvm.md).

Native mods (milestone M10) hook the game itself through a small C ABI (`libbhmod`: Objective-C swizzles, inline hooks
on exported functions or addresses, import redirects) and are loaded only from the reviewed index, after the player
confirmed a red warning: [`docs/native.md`](docs/native.md), example [`sdk/native/hello-c`](sdk/native/hello-c).

---

## 📚 What's inside
| Path | What |
| --- | --- |
| [`wit/blockheads-api/`](wit/blockheads-api/) | The mod API (`blockheads:api@0.3.1`, WIT): events (incl. gameplay), commands, scheduler, storage, players, services (also across the network, client <-> server), channels, client UI (HUD, toasts, dialogs), mod content (items, blocks, cosmetics); the frozen 0.1, 0.2 and 0.3.0 copies are in [`wit/compat/`](wit/compat/) |
| [`docs/luau-api.md`](docs/luau-api.md) | How the API looks from Lua |
| [`docs/wasm.md`](docs/wasm.md) | WASM mods: Rust SDK, C/Zig/TinyGo, how the host runs them, the conformance suite |
| [`docs/jvm.md`](docs/jvm.md) | JVM mods: Java/Kotlin SDK (generated from the WIT), how the host runs them, Android (ART) |
| [`sdk/rust/blockheads-mod`](sdk/rust/blockheads-mod) | The Rust SDK (bindings + closures for events, commands, tasks) |
| [`docs/native.md`](docs/native.md) | Native mods: the `libbhmod` C ABI and header, hook kinds, who may load native code, weapon skins |
| [`docs/package.md`](docs/package.md) | The `.bhmod` package and its rules |
| [`schema/`](schema/) | JSON Schemas for `mod.toml`, index entries and the published `index.json` |
| [`docs/protocol-0xbc.md`](docs/protocol-0xbc.md), [`vectors/`](vectors/) | Client ⇄ server mod protocol and its byte-exact test vectors |
| `crates/bhmod-manifest` | Reads and checks `mod.toml` and packages |
| `crates/bhmod-proto` | Encodes and decodes the 0xBC protocol |
| `crates/bhmod-cli` | The `bhmod` tool (`check`, `pack`, `new`, `index`) |
| `crates/bhmod-native` | The `libbhmod` ABI, the armv7 inline-hook relocator and the hook engine both clients share |
| `crates/bhmod-index` | Checks, builds and signs the public mod index ([`docs/index.md`](docs/index.md)) |
| `crates/bhmod-core` | The shared mod host (GPL, built from M1 on) |
| `crates/bhmod-wasm` | The WASM runtime of the host: wasmtime, Cranelift / Pulley, deadlines, memory limits, compile cache (GPL) |

**Developing:** `scripts/build-wasm.sh` (needs `rustup target add wasm32-wasip2`), then `cargo test --workspace`. CI
also runs clippy, rustfmt, the WIT check, the examples, the Luau ⇄ WASM conformance suite and a 100 % line coverage
gate (`cargo llvm-cov --workspace --ignore-filename-regex 'bhmod-wasm/src/wit' --fail-under-lines 100`; the excluded files hold only wasmtime's generated bindings).

---

## ⚖️ License
Your mod is yours: pick any licence you like. The API, schemas, tools and examples here are `MIT OR Apache-2.0`;
`bhmod-core` and `bhmod-wasm` are GPL-3.0-or-later with an exception for mods. Details: [LICENSING.md](LICENSING.md).

Not affiliated with Majic Jungle or Noodlecake. Mods must not include original game files.
