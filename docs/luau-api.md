# Luau API mapping (API 0.2)

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
| `on-channel-message` | `bh.channels.open(name, fn(origin, data))`; `origin` is `{ mod = "<id>" }`, `{ server = true }` or `{ player = <player> }` |

`bh.log` additionally has the shortcuts `bh.log.info(msg)`, `warn`, `error`, `debug`, `trace`; `print(...)` logs at `info`.
`bh.events.off(id)` is `unsubscribe`. Player handles compare equal by id and `tostring` to `player(<id>)`; on a
player who left, `alias()` returns `""`, `role()` `"player"`, and `send_chat` / `kick` raise `stale handle`.

## Values (`types.value`, CBOR)

Service arguments and results, custom event payloads and `bh.value.encode/decode` convert Luau values to canonical
CBOR and back:

| Luau | CBOR |
| --- | --- |
| `nil` | null |
| boolean | bool |
| number: integral and within ±2^53 | integer, otherwise a float (shortest exact width) |
| string: valid UTF-8 | text, otherwise bytes (bytes come back as a string) |
| table with keys exactly `1..n` (also the empty table) | list |
| table with only string keys | map (records use these keys) |
| player | host handle `{"$h": n}`; decoding gives the player back, or raises `stale handle` after they left |

Anything else (functions, threads, mixed keys, nesting deeper than 32) raises `bad value: ...`. A `nil` inside a
list ends the list; use a map when values can be missing.

## API 0.2 (milestone M3)

### Gameplay events

`bh.events.on(kind, fn, opts)` takes the new kinds. The table fields (positions are `{ x = .., y = .. }`):

| kind | fields | the handler may change |
| --- | --- | --- |
| `player-pre-login` | `player_id`, `alias`, `cancelled`, `reason` | `cancelled` (reject), `reason` (`"kicked"` by default; also `client-too-old`, `server-full`, `blacklisted`, `not-whitelisted`, `incorrect-password`, `banned`, `invalid-username`, `still-loading`) |
| `block-place` | `player`, `pos`, `item`, `data_a`, `data_b`, `cancelled` | `cancelled` |
| `block-break` | `player`, `pos`, `layer` (`"block"` / `"back-wall"`), `tile`, `cancelled` | `cancelled` |
| `block-changed` | `pos`, `tile`, `player` (or nil for simulation changes) | nothing (monitor only) |
| `object-place` | `player`, `pos`, `object` (`"workbench"` / `"interaction-object"`), `item`, `cancelled` | `cancelled` |
| `damage` | `player` (owner, or nil), `blockhead`, `amount`, `attacker` (PvP, or nil), `cancelled` | `cancelled`, `amount` |
| `death` | `player` (owner, or nil), `blockhead` | nothing |
| `player-move` | `player`, `from`, `to` (tiles) | nothing |

`player:pos()` returns the tile of the player's blockhead or `nil`.

### Services

```lua
-- provider (mod.toml: [provides] services = [{ name = "economy", version = "1.0.0", wit = "wit/economy.wit" }])
bh.services.provide("economy", "1.0.0", {
    balance = function(player) return balances[player:id()] or 0 end,
    pay = function(from, to, amount) ... end,
})
-- consumer (mod.toml: [dependencies] economy-mod = "^1", or soft-dependencies)
local eco = bh.services.get("economy", "^1")
local coins = eco.balance(player)                     -- or bh.services.call("economy", "^1", "balance", player)
```

- The name and version given to `provide` must be declared in `mod.toml` `[provides]`.
- A consumer may only call providers it depends or soft-depends on; the highest matching version wins.
- Calls are synchronous. A provider method raising `error("msg", 0)` makes the call raise
  `service economy.pay: provider-error: msg` in the caller (not a fault of the provider); running out of time or
  memory is a fault (`trap`). Other failures: `not-found`, `version-mismatch`, `not-permitted`, `reentrant`
  (A -> B -> A), `bad-value`.
- Only the first return value is passed back.

### Custom events

`bh.events.on_custom("<mod-id>:<name>", fn, opts)` subscribes to any mod's custom event with the normal priority and
cancel rules; `bh.events.fire_custom("<own-id>:<name>", payload)` fires one and returns whether it ended up
cancelled. Handlers see `ev.name`, `ev.source` (the firing mod), `ev.payload` (a Luau value) and `ev.cancelled`, and
may replace `ev.payload`. Handlers of mods that are running at that moment (the firing mod itself, or a mod that
called a service of the firing mod) are run after the outermost call returns, and their changes are ignored.

### Channels

`bh.channels.open("<mod-id>:<name>", fn(origin, data))` (needs the `channels` permission; the mod id must be the
caller's own or a dependency) returns a channel id; `bh.channels.send(id, target, data)` with target `"local"`
(every other mod that opened the name, delivered on the next poll or tick), a player, `"all-players"` or `"server"`.
The network targets arrive with milestone M5 and raise `unsupported` until then. `bh.channels.close(id)`.

### `bh.value`

`bh.value.encode(v) -> string` and `bh.value.decode(s) -> any` (handy for `bh.storage`).

## API versions

The host implements `blockheads:api@0.2.0` (`bh.host.api_version()`) and also serves 0.1: in 0.x a new minor only
adds events, functions and fields, so a mod with `api = "^0.1"` keeps loading and running unchanged. A mod that
uses anything from 0.2 declares `api = "^0.2"`. WASM hosts bind the frozen 0.1 WIT in
[`../wit/compat/blockheads-api-0.1.0/`](../wit/compat/blockheads-api-0.1.0/) for components built against it.

## Host behaviour (M1)

- One Luau VM per mod, sandbox mode (globals and every `bh` table are read-only), 32 MiB memory limit.
- Each call into the mod has a 25 ms hard deadline enforced by the Luau interrupt (a `pcall` cannot swallow it:
  the interrupt keeps firing until the call unwinds) and a 5 ms soft limit that logs a warning. A Luau error, a
  timeout, running out of memory or a host panic is a fault; 3 faults within 60 s disable the mod.
- `main.luau` runs once, when the world has loaded, then `on_enable`. An error there disables the mod.
- Admins grant permissions in `mods.toml` inside the mods directory (`[<mod-id>] grant = ["players:chat"]`,
  `enabled = false` to switch a mod off). A mod that asks for more than is granted is not loaded.

## Files

The entry point is `server/main.luau` or `client/main.luau`. Other `.luau` files of the same side can be loaded with
`require("./other")` (relative to the entry, inside the package only). No `io`, `os.execute`, `loadstring` or
`debug` library; `os.time`/`os.clock` are available.
