# bhmod-core

**Licence: GPL-3.0-or-later** (with the Mod API Exception in [`../../LICENSING.md`](../../LICENSING.md)).

Placeholder in milestone M0. From **M1** on this is the one mod host shared by the Blockheads Club server, the PC
port and the Android loader. Each of them adds only a thin `GameBridge`.

Planned contents (see `notes/mod-system.md` in the umbrella repo):

- package loading and validation (via `bhmod-manifest`), dependency order, permissions granted by the admin
- event bus with Spigot priorities and cancellation, custom events
- commands, scheduler (ticks and next-poll), per-mod KV storage
- time budgets (5 ms soft / 25 ms hard per handler, 20 ms per world step, 3 strikes in 60 s)
- runtimes: Luau (M1), WASM components via wasmtime / Pulley (M2), JVM (M9)
- service registry and channels (M3), the 0xBC protocol host side (M5, via `bhmod-proto`)

With no mods installed the host must have zero effect on the game.
