# Luau API mapping (v0)

Lua mods use **Luau** (sandboxed, with a memory limit and an interrupt for time budgets). The API is the same
`blockheads:api` WIT ([`../wit/blockheads-api/`](../wit/blockheads-api/)), mapped by these rules. Milestone M1
implements them in `bhmod-core` and ships generated `.d.luau` type stubs; until then this page is the contract.

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

`bh.log` additionally has the shortcuts `bh.log.info(msg)`, `warn`, `error`, `debug`, `trace`.

## Files

The entry point is `server/main.luau` or `client/main.luau`. Other `.luau` files of the same side can be loaded with
`require("./other")` (relative to the entry, inside the package only). No `io`, `os.execute`, `loadstring` or
`debug` library; `os.time`/`os.clock` are available.
