# Licensing

Short version: **your mod is yours. Pick any licence you like** (MIT, GPL, proprietary, ...).

## What is licensed how in this repo

| Part | Licence |
| --- | --- |
| API definitions (`wit/`), JSON Schemas (`schema/`), protocol spec and test vectors (`docs/`, `vectors/`) | `MIT OR Apache-2.0` |
| Language SDKs / bindings, templates, examples (`examples/`), `bhmod-manifest`, `bhmod-proto`, the `bhmod` CLI | `MIT OR Apache-2.0` |
| `crates/bhmod-core` (the shared mod host used by the GPL server, PC and Android hosts), `crates/bhmod-wasm` (its WASM runtime) and `crates/bhmod-voice` (the hosts' voice chat core) | `GPL-3.0-or-later` |
| `crates/bhmod-opus-sys` (voice codec binding used by the hosts): our Rust code and build script | `GPL-3.0-or-later` |
| `examples/voice-rust` (the official voice chat mod; a separate work like any mod, see below) | `GPL-3.0-or-later` |
| `crates/bhmod-opus-sys/opus/` (vendored libopus 1.5.2) | `BSD-3-Clause` (its `COPYING`; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)) |

`MIT OR Apache-2.0` means you may use those files under either licence, at your choice
([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)). `bhmod-core` and `bhmod-wasm` are under the GNU GPL version 3 or
later ([crates/bhmod-core/LICENSE](crates/bhmod-core/LICENSE)), like the Blockheads Club server and clients it is built into.

## Mods are separate works

A mod talks to the game only through the **published API**: the `blockheads:api` WIT package, the Lua (Luau) API
generated from it, the JVM interfaces generated from it, the `libbhmod` C ABI, the `mod.toml` format and the 0xBC
network protocol. All of these are permissively licensed so that anyone can build on them.

The Blockheads Club host programs (`bhmod-core`, the server, the PC port, the Android loader) load a mod at run
time, give it data through that API and get data back, the same way a browser runs a web page. We, the copyright
holders of those GPL programs, consider a mod that uses only the published API a **separate work**, not a work based
on the host. You may license such a mod under any terms you choose, including closed source, and distribute it
alone or through the public index.

## Exception for native and JVM mods

Native mods (`.so` libraries, ObjC/C hooks) and JVM mods run in the same process as the host and link against
interfaces it exposes, so the "separate work" reading could be doubted for them. To remove that doubt the copyright
holders of `bhmod-core` and of the Blockheads Club host programs grant this additional permission under section 7 of
the GNU GPL version 3:

> **Blockheads Club Mod API Exception, version 1.0**
>
> As a special exception, the copyright holders of this program give you permission to combine this program with a
> "Mod" and to convey the resulting combination, and the Mod alone, under terms of your choice, provided that the Mod
> interacts with this program only through the Published API. "Published API" means the `blockheads:api` WIT
> package, the `libbhmod` C ABI and its headers, the Lua and JVM bindings generated from them, and the `mod.toml`
> and 0xBC protocol specifications, as published in the Blockheads Club `mod-sdk` repository under a permissive
> licence. A Mod that copies, modifies or links against any other part of this program (for example internal Rust
> crates of `bhmod-core` or the server) is not covered by this exception. This exception does not change the licence
> of this program itself; if you modify this program you may extend this exception to your version, but you are not
> obliged to do so.

The exception is attached to `bhmod-core` here and will be attached to each GPL host repository that loads mods.

## Not covered

- The Blockheads game itself (binaries, assets, names) belongs to its owners and is not licensed by this repo. Mods
  must not ship original game files.
- Anything a mod author publishes is their own responsibility, including its licence file (the public index requires
  a `LICENSE` file in every listed package).

This page explains our intent; it is not legal advice.
