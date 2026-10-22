# bhmod-core

**Licence: GPL-3.0-or-later** (with the Mod API Exception in [`../../LICENSING.md`](../../LICENSING.md)).

The one mod host shared by the Blockheads Club server, the PC port and the Android loader. Each of them adds only a
thin `GameBridge` and calls the `ModHost` at its hook points.

| Module | What |
| --- | --- |
| `discover`, `config`, `resolve` | `.bhmod` zips and unpacked mod dirs (checked with `bhmod-manifest`), `mods.toml` (enable/disable, permission grants), dependency order |
| `bus`, `commands`, `scheduler` | Spigot priorities `lowest`..`monitor` with cancellation and `ignore_cancelled`; command registry with `min-role`; delayed / repeating / next-poll tasks |
| `storage` | per-mod per-world KV with the API limits; `FileStorage`, `MemoryStorage`, or a host backend (`KvBackend`) |
| `budget`, `host` | 5 ms soft / 25 ms hard per call, 20 ms per world step, 3 faults in 60 s disable a mod; panics are caught |
| `runtime` | `Runtime` / `Instance` (the WIT `guest` export) / `HostApi` (the WIT imports): the seam for WASM (M2) and JVM (M9) |
| `lua` (feature `luau`, default) | Luau via mlua: sandbox, memory limit, interrupt deadline, the `bh` table of `docs/luau-api.md` |

With no mods installed the host must have zero effect on the game: hosts construct a `ModHost` only when the mods
directory exists and install it only when `!is_empty()`.

`cargo llvm-cov -p bhmod-core --fail-under-lines 100` is the coverage gate (fakes for the bridge, clock, runtime and
storage live in `tests/`).
