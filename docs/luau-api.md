# Luau API mapping (API 0.3.4)

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
| `call-service` | `bh.services.provide(name, version, { method = fn, ... })` (also what remote callers reach) |
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

`player:pos()` returns the tile of the player's blockhead or `nil`. `player:value_handle()` is the number in the
player's `{"$h": n}` value and `bh.players.from_value_handle(n)` turns it back into a player (`nil` once they left);
Luau converts players automatically, these are for hand-made values. WASM mods use `player.value-handle()` and
`players.from-value-handle()` the same way (see `examples/shop-rust`, a Rust mod paying through the Luau economy).

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

A proxy from `bh.services.get` goes stale when the providing mod is hot-reloaded ([`hot-reload.md`](hot-reload.md)): calling it raises `stale service handle ...`; fetch a new one. `bh.services.call` always looks the provider up again.

- The name and version given to `provide` must be declared in `mod.toml` `[provides]`.
- A consumer may only call providers it depends or soft-depends on; the highest matching version wins.
- Calls are synchronous. A provider method raising `error("msg", 0)` makes the call raise
  `service economy.pay: provider-error: msg` in the caller (not a fault of the provider); running out of time or
  memory is a fault (`trap`). Other failures: `not-found`, `version-mismatch`, `not-permitted`, `reentrant`
  (A -> B -> A), `bad-value`.
- Only the first return value is passed back.

### Remote services (API 0.3.1)

A client mod can call a service a **server** mod provides, and a server mod one a **client** mod provides, over the
network (0xBC `SERVICE_CALL` / `SERVICE_RESULT`, see [protocol-0xbc.md](protocol-0xbc.md)). The call is asynchronous:

```lua
-- provider (server mod; mod.toml: services = [{ name = "shop", version = "1.0.0", wit = "wit/shop.wit",
--   remote = true, remote-methods = ["price", "buy"] }]); remote-methods is optional (empty = every method)
bh.services.provide("shop", "1.0.0", {
    price = function(item) return prices[item] end,
    buy = function(item)
        local c = bh.services.caller()            -- { mod = "shop-ui", player = <player> }
        ...
    end,
})

-- caller (client mod, permission `channels`)
local id = bh.services.call_remote("server", "shop", "^1", "price", { "apple" }, function(ok, v)
    if ok then bh.ui.toast("apple costs " .. v) else print("failed: " .. v.kind .. " " .. v.message) end
end, 3000)                                          -- timeout in ms (default 5000, at most 30000)
bh.services.cancel_remote(id)                       -- the callback never runs
```

- `bh.services.call_remote(target, name, version_req, method, args, callback, timeout_ms?) -> id`. `target` is
  `"server"` (client mods) or a player (server mods); `args` is a list (or `nil`). The callback gets `(true, value)`
  or `(false, { kind, message })` exactly once, **later**: never inside `call_remote` itself, also not when the failure
  is known at once (`unavailable`). Errors the call itself can raise: `invalid argument` (names, requirement,
  arguments, timeout, wrong target), `limit exceeded` (16 open calls per mod, arguments over 32 KiB),
  `not permitted` (no `channels` permission), `stale handle` (the player left).
- `kind` is `not-found`, `version-mismatch`, `not-permitted` (the service or method is not `remote`),
  `busy` (rate limited), `provider-error` (`message` is the provider's `error("...", 0)` text, cut at 256 bytes and
  without traceback), `trap` (the provider ran out of time or crashed), `bad-value`, `too-large`, `internal` (answers
  of the other side) and `unavailable` (no modded server / player without the mod loader), `timeout`, `disconnected`
  (the player quit or the server went away) decided locally.
- The providing side decides what is reachable (`remote = true`, optionally `remote-methods`), limits each peer
  (token bucket: bursts of 20, 10 calls per second) and all peers together (200, 100 per second), and runs the method under the provider's own time budget (a trap
  counts as a fault of the provider, like any other call). A caller need not depend on the provider.
- `bh.services.caller()` inside a provider method returns `{ mod = "<calling mod id>", player = <player> | nil }`
  (`player` is set when a client called a server mod), or `nil` for a call from a mod on the same side. It also holds
  in services the provider calls locally while serving the remote call.
- Under the hood the answer is the custom event `<your-mod-id>:service-result`; Luau subscribes for you.

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
The network targets need the 0xBC protocol: the server sends to players running the mod loader (others are skipped
silently; a server mod cannot target `"server"`); client hosts raise `unsupported` until their loader lands (M6/M7).
`bh.channels.close(id)`.

### `bh.value`

`bh.value.encode(v) -> string` and `bh.value.decode(s) -> any` (handy for `bh.storage`).

## API 0.3 (client UI)

`bh.ui` draws on the player's screen; only client hosts implement it (on the server every call raises
`unsupported`). Mods send data, the loader draws it with its own kit (IG-HUD, IG-ModUI): same look for everybody,
no free placement, never taking focus from the game. It needs no permission; players can hide a mod's widgets.

| Function | |
| --- | --- |
| `bh.ui.set_widget(id, w)` | Shows or replaces a HUD widget. `id`: 1 to 32 characters of `a-z 0-9 _ . -`, at most 16 per mod. `w` is `{ slot = "top-left" or "top-right", kind = "text" or "counter" or "panel", icon = <kit icon name>, title = <panel title>, text = <at most 256 bytes> }`; `slot` defaults to `"top-left"`, `kind` to `"text"`. A `counter` shows `icon` and the number in `text`; a `panel` shows `title` and up to four lines of `text`. |
| `bh.ui.remove_widget(id) -> boolean` | Removes a widget. |
| `bh.ui.toast(text)` | A short message, bottom right, three visible at most, about four seconds each. |
| `bh.ui.notice(text, level)` | A message in the top centre, one at a time. `level`: `"info"` (default), `"warning"`, `"success"`, `"error"`. |
| `bh.ui.show_dialog(d) -> id` | Opens a dialog (one at a time per mod): `{ title, text, field = { placeholder, initial, max_length } (optional), buttons = { { label, kind = "primary" or "secondary" or "danger" }, ... } }` with one to three buttons. |
| `bh.ui.close_dialog(id) -> boolean` | Closes it without an answer. |

Text limits: toast and notice 200 bytes, dialog title 64, dialog text 512, button label 24, field 200 characters.

The answer to a dialog is the custom event `<mod-id>:ui-action` (subscribe with `bh.events.on_custom`), delivered to
the mod that opened the dialog and nobody else. Its payload is `{ dialog = <id>, button = <index from 0>, text =
<field text or nil> }`; `button` is `4294967295` when the dialog went away without an answer (back, leaving the
world). See `examples/hud-lua`.

### `bh.content` (0.3)

Content declared in `[content]` and cosmetics, see [content.md](content.md). `kind` is `"block"`, `"item"` or
`"cosmetic"`.

- `bh.content.id_of(kind, key) -> number?` and `bh.content.key_of(kind, id) -> string?`: the world's id of a key and back.
- `bh.content.cosmetics() -> { {key, mod_id, slot, id} }`: the cosmetics of all loaded mods.
- `bh.content.equip(player, key)`: puts one of **your mod's** cosmetics on a player (replacing the one in its slot).
  Another mod's key raises `not-permitted`, an unknown key `invalid-argument`, a player who left `stale-handle`;
  clients raise `unsupported`.
- `bh.content.unequip(player, slot)`, `bh.content.equipped(player) -> { [slot]: key }`.
- `bh.content.give_item(player, key, count?)` (0.3.2): gives a player `count` (default 1, at most 64) of **your mod's** item.
  It goes into the inventory of the player's blockhead, or drops next to it when that is full. Server only.
- `bh.content.place_block(x, y, key)` (0.3.2): replaces the block at the tile with **your mod's** variant block `key`
  (its `base` block carrying the variant's id). Server only; the tile must be loaded.
- `bh.content.block_at(x, y) -> string?` (0.3.2): the variant block key at a tile, `nil` for a plain block.

### `bh.settings` (0.3.3)

The mod's own settings, declared as `[settings.<key>]` in `mod.toml` and set by the admin (see
[settings.md](settings.md)). Values are plain Luau values: booleans, numbers, strings, a list of role names
(`roles`), `{ [row] = { [col] = cell } }` (`matrix`) and a list of tables (`list`).

- `bh.settings.get(key) -> any`: the current value; `nil` for a key the mod does not declare.
- `bh.settings.all() -> { [string]: any }`: every setting.
- `bh.settings.set(key, value)`: changes one setting. The value is checked against the declaration (an error raises
  `invalid argument: <key>: <why>`), stored in `mods.toml` and announced like an admin change.
- After every change (admin command, host app, an edit of `mods.toml`, or `set`) the host fires the custom event
  `<mod-id>:settings-changed` with the payload `{ keys = { ... } }`. Subscribe with
  `bh.events.on_custom("<mod-id>:settings-changed", fn)`; other mods may listen to it too.

### `bh.voice`, `bh.terrain`, `bh.input` and the `bh.ui` additions (0.3.4)

See [voice.md](voice.md) for the whole API, the permission `voice` and the events. The mapping is the usual one:
players are player handles, `list<mode>` is a list of strings (`{ "proximity", "global" }`), records are tables with
snake_case keys, missing optional fields take the defaults.

- `bh.voice.*` (permission `voice`): `state(p)`, `icons()`; server: `set_routes(speaker, { { listener = p, modes = {...},
  gain, pan, muffle } })`, `mute(p, until_ms?)`, `unmute(p)`, `set_speaker_gain(p, n)`, `set_caps{...}`,
  `set_transport{ udp_port }` (`mode` is ignored: frames always use the voice UDP port), `set_allowed(p, modes,
  activations)`, `set_icons{...}`,
  `set_broadcast_overrides_deafen(b)`; client: `set_default_mode(m)`, `set_activation(a)`, `talk(mode?, pressed)`,
  `set_mic(b)`, `set_self_mute(b)`, `set_deafen(b)`, `set_vad_threshold(db)`, `set_master(n)`, `master()`,
  `set_player_volume(p, n)`, `player_volume(p)`, `mute_player(p, b)`, `player_muted(p)`, `allowed()`,
  `self_muted()`, `deafened()`, `mic()`.
- `bh.terrain.solid_count(x1, y1, x2, y2)` (server), `bh.terrain.world_width()`.
- `bh.input.bind(action, default_key)`, `unbind(action)`, `key_of(action)` (client); presses arrive as
  `<mod-id>:input { action, pressed }`.
- `bh.ui.set_overhead(p, icon)`, `clear_overhead(p)`, `set_button(id, { icon, label, x, y, size, opacity })`,
  `remove_button(id)`, `open_settings(mod_id)`, `has_touch()`; button input as `<mod-id>:ui-button { id, action }`,
  picks of blockheads (a middle click on PC, a 500 ms long press on Android) as `<mod-id>:ui-pick { player }`. Dialogs take up to four buttons.
- `bh.host.platform()`, `player:sleeping()`, `player:alive()` (server; events `player:sleep`, `player:wake`,
  `player:respawn` with `{ player }`), `bh.players.local_player()` (client), `bh.players.has_mod(p, mod_id)` (server).

## API versions

The host implements `blockheads:api@0.3.4` (`bh.host.api_version()`) and also serves 0.3.3, 0.3.2, 0.3.1, 0.3.0, 0.2 and 0.1: in 0.x a new
minor only adds events, functions and fields, so a mod with `api = "^0.1"` keeps loading and running unchanged. A mod
that uses anything from 0.2 declares `api = "^0.2"`, one that uses `bh.ui` or `bh.content` declares `api = "^0.3"`, one
that uses remote services (`bh.services.call_remote`) or `remote = true` declares `api = "^0.3.1"`. 0.3.1 is a patch
release because it only adds (a function group in `services`, no new export, no changed type): components built for
0.3.0 link and run on a 0.3.1 host without a new binding, which wasmtime matches semver-compatibly. 0.3.2 adds the
same way `bh.content.give_item`, `place_block` and `block_at` (a mod that uses them declares `api = "^0.3.2"`), and
0.3.3 the `settings` interface with `[settings]` in `mod.toml` (`api = "^0.3.3"`), 0.3.4 `voice`, `terrain`,
`input`, the `ui`/`players`/`host` additions and the `voice` permission (`api = "^0.3.4"`). WASM hosts bind the
frozen 0.2 and 0.1 WIT in [`../wit/compat/blockheads-api-0.2.0/`](../wit/compat/blockheads-api-0.2.0/) and
[`../wit/compat/blockheads-api-0.1.0/`](../wit/compat/blockheads-api-0.1.0/) for components built against them.

## Host behaviour (M1)

- One Luau VM per mod, sandbox mode (globals and every `bh` table are read-only), 32 MiB memory limit.
- Each call into the mod has a 25 ms hard deadline (1 s for `on_enable` and the first call after a load: a cold
  start) enforced by the Luau interrupt (a `pcall` cannot swallow it:
  the interrupt keeps firing until the call unwinds) and a 5 ms soft limit that logs a warning. A Luau error, a
  timeout, running out of memory or a host panic is a fault; 3 faults within 60 s disable the mod.
- `main.luau` runs once, when the world has loaded, then `on_enable`. An error there disables the mod.
- Admins grant permissions in `mods.toml` inside the mods directory (`[<mod-id>] grant = ["players:chat"]`,
  `enabled = false` to switch a mod off). A mod that asks for more than is granted is not loaded.

## Files

The entry point is `server/main.luau` or `client/main.luau`. Other `.luau` files of the same side can be loaded with
`require("./other")` (relative to the entry, inside the package only). No `io`, `os.execute`, `loadstring` or
`debug` library; `os.time`/`os.clock` are available.
