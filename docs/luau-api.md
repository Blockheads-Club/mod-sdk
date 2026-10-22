# Luau API mapping (v0)

Lua mods use **Luau** (sandboxed, with a memory limit and an interrupt for time budgets). The API is the same
`blockheads:api` WIT ([`../wit/blockheads-api/`](../wit/blockheads-api/)), mapped by these rules. Since milestone M1
`bhmod-core` implements them (`crates/bhmod-core/src/lua.rs`); type declarations for editors are in
[`bh.d.luau`](bh.d.luau) (hand-written in M1, generated from the WIT later).

## Rules

| WIT | Luau |
| --- | --- |
| interface `foo-bar` | table `bh.foo_bar` (global `bh`) |
| function `run-later` | `bh.scheduler.run_later(...)` (kebab-case -> snake_case) |
| resource method `player.send-chat` | `player:send_chat(text)` (userdata with methods) |
| record | table with snake_case keys |
| enum / variant without payload | string, the WIT name (`"player-join"`, `"normal"`) |
| variant with payload | table `{ tag = "player", value = ... }`; `command-sender` is `{ player = <player> }` or `{ console = true }` |
| `option<T>` | `nil` or the value |
| `result<T, E>` | returns `T` on success; raises a Luau error with the `api-error` text on failure |
| `list<u8>` (bytes, CBOR `value`) | Luau string (binary); CBOR values are converted to and from plain Luau values |
| `u64` handles / ids | number |

## Callbacks instead of the `guest` export

WASM mods export `guest`; Luau mods pass functions:

| `guest` export | Luau |
| --- | --- |
| `on-enable`, `on-disable` | fields of the table returned by `main.luau` (`return { on_enable = fn, on_disable = fn }`) |
| `handle-event` | `bh.events.on(kind, fn(ev), opts?)`; `opts = { priority = "normal", ignore_cancelled = false }`. Returns the subscription id. The handler changes `ev.cancelled` / `ev.text` in place instead of returning an outcome. Custom events: `bh.events.on_custom(name, fn, opts?)` |
| `run-command` | `bh.commands.register(spec, fn(sender, label, args) -> { handled, reply })`; `spec.min_role` defaults to `"player"`, `spec.console` to `true`, `aliases` to `{}` |
| `run-task` | `bh.scheduler.run_later(delay, fn)`, `run_every(delay, period, fn)`, `run_next_poll(fn)` |
| `call-service` | `bh.services.provide(name, version, { method = fn, ... })` |
| `on-channel-message` | `bh.channels.open(name, fn(origin, data))` |

`bh.log` additionally has the shortcuts `bh.log.info(msg)`, `warn`, `error`, `debug`, `trace`; `print(...)` logs at `info`.
`bh.events.off(id)` is `unsubscribe`. Player handles compare equal by id and `tostring` to `player(<id>)`; on a
player who left, `alias()` returns `""`, `role()` `"player"`, and `send_chat` / `kick` raise `stale handle`.

## Host behaviour (M1)

- One Luau VM per mod, sandbox mode (globals and every `bh` table are read-only), 32 MiB memory limit.
- Each call into the mod has a 25 ms hard deadline enforced by the Luau interrupt (a `pcall` cannot swallow it:
  the interrupt keeps firing until the call unwinds) and a 5 ms soft limit that logs a warning. A Luau error, a
  timeout, running out of memory or a host panic is a fault; 3 faults within 60 s disable the mod.
- `main.luau` runs once, when the world has loaded, then `on_enable`. An error there disables the mod.
- In API 0.1 the custom events (`on_custom`, `fire_custom`), `bh.services` and `bh.channels` raise `unsupported`;
  they arrive with milestone M3.
- Admins grant permissions in `mods.toml` inside the mods directory (`[<mod-id>] grant = ["players:chat"]`,
  `enabled = false` to switch a mod off). A mod that asks for more than is granted is not loaded.

## Files

The entry point is `server/main.luau` or `client/main.luau`. Other `.luau` files of the same side can be loaded with
`require("./other")` (relative to the entry, inside the package only). No `io`, `os.execute`, `loadstring` or
`debug` library; `os.time`/`os.clock` are available.
