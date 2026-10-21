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
**Current Status:** Spec v0 (milestone M0). This repo holds the mod API, the file formats, the network protocol and
the `bhmod` tool. Servers can't load mods yet: the [Blockheads Server](https://github.com/Blockheads-Club/blockheads-server)
gets Lua mods next (M1), then WASM mods (M2). Things may still change before 1.0.

Mods can run on the server, on the client, or both. Players who join a modded server see the list of mods, accept,
and the client part downloads straight from the server.

---

## 📦 Other Blockheads Club Repositories
- [**blockheads-server**](https://github.com/Blockheads-Club/blockheads-server) - Host your own Blockheads server
- [**blockheads-pc**](https://github.com/Blockheads-Club/blockheads-pc) - Play The Blockheads on your computer
- [**blockheads-patcher**](https://github.com/Blockheads-Club/blockheads-patcher) - Fix crashes and add small improvements to the game on your phone
- [**website**](https://github.com/Blockheads-Club/website) - The [Blockheads Club website](https://blockheads-club.github.io/website/): downloads, guides and docs

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

Copy the `.bhmod` into the server's `mods/` folder (once the server supports mods). Full examples:
[`examples/hello-lua`](examples/hello-lua) and [`examples/hello-rust`](examples/hello-rust).

---

## 📚 What's inside
| Path | What |
| --- | --- |
| [`wit/blockheads-api/`](wit/blockheads-api/) | The mod API (`blockheads:api@0.1.0`, WIT): events, commands, scheduler, storage, players, services, channels |
| [`docs/luau-api.md`](docs/luau-api.md) | How the API looks from Lua |
| [`docs/package.md`](docs/package.md) | The `.bhmod` package and its rules |
| [`schema/`](schema/) | JSON Schemas for `mod.toml`, index entries and the signed `index.json` |
| [`docs/protocol-0xbc.md`](docs/protocol-0xbc.md), [`vectors/`](vectors/) | Client ⇄ server mod protocol and its byte-exact test vectors |
| `crates/bhmod-manifest` | Reads and checks `mod.toml` and packages |
| `crates/bhmod-proto` | Encodes and decodes the 0xBC protocol |
| `crates/bhmod-cli` | The `bhmod` tool (`check`, `pack`, `new`) |
| `crates/bhmod-core` | The shared mod host (GPL, built from M1 on) |

**Developing:** `cargo test --workspace`. CI also runs clippy, rustfmt, the WIT check, the examples and a 100 % line
coverage gate (`cargo llvm-cov --workspace --fail-under-lines 100`).

---

## ⚖️ License
Your mod is yours: pick any licence you like. The API, schemas, tools and examples here are `MIT OR Apache-2.0`;
`bhmod-core` is GPL-3.0-or-later with an exception for mods. Details: [LICENSING.md](LICENSING.md).

Not affiliated with Majic Jungle or Noodlecake. Mods must not include original game files.
