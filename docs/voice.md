# Voice chat, overhead icons, buttons, key bindings (API 0.3.4)

`blockheads:api@0.3.4` adds what a voice chat mod needs, and the seams every host implements. Plan and decisions:
`notes/voice-mod.md` (maintainer repo); wire format: [protocol-0xbc.md](protocol-0xbc.md) ops `0x15`-`0x1b`.

The **host** does capture, voice activity detection (VAD), Opus, encryption, the server relay and the client mix. A
**mod** never sees audio: on the server it publishes policy (who hears whom, mutes, gains, caps, allowed modes), on
the client it drives the player's controls (push-to-talk, activation, local volumes) and draws the overhead icons.
No API can read decoded audio, and the server has none (it relays sealed Opus frames).

Versioning: 0.3.4 is a patch release like 0.3.1-0.3.3: it adds the interfaces `voice`, `terrain` (both worlds) and
`input` (client world), and adds functions and types to `ui`. No existing type changes, so components built for any
0.3.x keep linking. A mod that uses anything below declares `api = "^0.3.4"`.

## Permission `voice`

`[permissions] voice = true` in `mod.toml`, granted in `mods.toml` as `grant = ["voice"]` (safe permission, shown in
the accept dialog: "Use voice chat: microphone and speakers through the game"). Every `voice.*` function needs it
(`not-permitted("voice")`), and the `voice:*` events are delivered only to mods holding it. `ui`, `input` and
`terrain` need no permission.

## WIT

- [`voice.wit`](../wit/blockheads-api/voice.wit): `mode`, `activation`, `voice-state`, `route`, `caps`, `transport`,
  `transport-config`, `icon-visibility`; functions below.
- [`input.wit`](../wit/blockheads-api/input.wit): `bind`, `unbind`, `key-of`.
- [`terrain.wit`](../wit/blockheads-api/terrain.wit): `solid-count` (named `terrain`: `world` is a WIT keyword).
- [`ui.wit`](../wit/blockheads-api/ui.wit): `overhead-icon`, `set-overhead`, `clear-overhead`, `button`,
  `set-button`, `remove-button`, `open-settings`.

### `voice` (permission `voice`)

| Function | Side | Meaning |
| --- | --- | --- |
| `state(who) -> voice-state` | both | `none`, `ready`, `talking`, `self-muted`, `deafened`, `admin-muted` |
| `icons() -> icon-visibility` | both | Overhead icon visibility (server: as set; client: as received, all `true` before) |
| `set-routes(speaker, list<route>)` | server | Who hears `speaker`: `route {listener, modes: list<mode>, gain 0..1, pan -1..1, muffle 0..1}`. Empty `modes` = every mode. Empty list = nobody. ≤ 256 routes. About 5 Hz. |
| `mute(who, until-ms: option<u64>)` | server | Admin mute until Unix time `until-ms` (ms) or until `unmute`. Kept across reconnects while the server runs; the mod persists it across restarts. |
| `unmute(who) -> bool` | server | False when not muted. |
| `set-speaker-gain(who, percent)` | server | 0..=200, default 100, applied to everything `who` says. |
| `set-caps(caps)` | server | `{max-bitrate 6000..=64000, max-speakers 1..=64, max-frames-per-sec 50..=200}`, default 24000 / 8 / 60. |
| `set-transport({mode: auto\|inband\|udp, udp-port})` | server | Applies to voice sessions started afterwards; the host opens or closes the port. |
| `set-allowed(who, modes, activations)` | server | What the client offers (PTT keys, mode picker, activations). Default: no modes, all activations. Routes decide what is heard. |
| `set-icons(icon-visibility)` | server | `{speaking, no-voice, self-mute, deafen, admin-mute}`, default all `true`. |
| `set-broadcast-overrides-deafen(bool)` | server | Default `false`. |
| `set-default-mode(mode)` | client | Mode of VAD, toggle and the main PTT (default `proximity`). |
| `set-activation(activation)` | client | `ptt`, `vad`, `toggle`; not allowed by the server -> `invalid-argument`. |
| `talk(option<mode>, pressed)` | client | PTT / toggle input (`none` = default mode). |
| `set-mic(on)` | client | Opens / closes the microphone; first `true` asks the OS (Android). Result: `voice:mic`. Off until a mod turns it on. |
| `set-self-mute(on)`, `set-deafen(on)` | client | Sent to the server (`VOICE_CTRL`). |
| `set-vad-threshold(db)` | client | -80.0..=0.0 dBFS, default -45.0. |
| `set-master(percent)`, `master()` | client | 0..=200, default 100; stored on the device. |
| `set-player-volume(who, percent)`, `player-volume(who)` | client | 0..=200, default 100; stored on the device by player id. |
| `mute-player(who, muted)`, `player-muted(who)` | client | Local mute; stored on the device by player id. |
| `allowed() -> allowance{modes, activations, max-bitrate}` | client | What the server allows this player (from `VOICE_STATE`). |
| `self-muted()`, `deafened()` | client | The player's own switches. |
| `mic() -> mic-state` | client | `off`, `on`, `denied`. |

Wrong side -> `unsupported`; a host without voice -> `unsupported`; out-of-range numbers -> `invalid-argument`;
a player who left -> `stale-handle`. The server never learns client volumes or local mutes.

Heard level = server speaker gain × route gain (relay, the frame's `gain` byte) × client master × client per-player,
soft-limited.

### Events (custom events fired by the host)

Delivered only to mods holding `voice`; subscribe with `events.subscribe-custom` / `bh.events.on_custom`. `player`
is the vanilla `playerID` text (`players.get` gives the handle).

| Name | Payload | When |
| --- | --- | --- |
| `voice:talk-start` | `{player, mode}` | Server: first relayed frame of a talk spurt. Client: first frame heard from that speaker. |
| `voice:talk-stop` | `{player, mode}` | 300 ms without a frame. |
| `voice:state` | `{player, state}` | A `voice-state` changed (state names as in WIT: `self-muted`, ...). |
| `voice:mic` | `{state: "off" \| "on" \| "denied"}` | Client: the microphone opened, closed, or the OS refused. |
| `voice:allowed` | `{modes: [text], activations: [text], max-bitrate}` | Client: what the server allows changed. |
| `voice:mode` | `{mode}` | Client: the default mode changed (`set-default-mode` or the loader's mode picker). |

### `terrain` (no permission)

`solid-count(x1, y1, x2, y2) -> u32`: solid tiles on the Bresenham line between two tiles, both end tiles excluded,
x wrapping the short way round the world, unloaded tiles not solid, cut after 256 tiles. Server only.
`world-width() -> option<s32>`: the width `x` wraps around (for distance and pan the short way round).

### Other 0.3.4 additions (no permission)

- `host.platform() -> string`: `<host>-<os>-<arch>` (`server-linux-x86_64`, `pc-windows-x86_64`, `android-armv7`, ...).
- `player.sleeping() -> bool`, `player.alive() -> bool` (server; false on clients and for players who left). The host
  fires the custom events `player:sleep`, `player:wake` and `player:respawn` with `{player}` to every subscriber.
- `players.local-player() -> option<player>` (client; `local` is a Luau keyword), `players.has-mod(who, mod-id) -> bool`
  (server: the player's client runs that mod; tells vanilla from "modded, voice not started").
- `ui.has-touch() -> bool`; `<mod-id>:ui-pick {player}` to every client mod when the player taps or clicks a
  blockhead (per-player volume by clicking someone); dialogs take up to **four** buttons.

### `input` (client, no permission)

`bind(action, default-key)`, `unbind(action) -> bool`, `key-of(action) -> option<string>`. Action ids follow the
widget id rules, ≤ 16 per mod. Key names are listed in [`input.wit`](../wit/blockheads-api/input.wit). The player
rebinds in the loader's settings screen (stored on the device). Event `<mod-id>:input {action, pressed}` to the
binding mod only, key down (no repeat) and key up, only while no text field is open.

### `ui` additions (client, no permission)

- `set-overhead(who, icon)` / `clear-overhead(who) -> bool`: one icon per (mod, player). Icons `speaking`,
  `no-voice`, `self-mute`, `deafen`, `admin-mute`. The loader hides icons the server's admin turned off.
- `set-button(id, button)` / `remove-button(id) -> bool`: round on-screen button `{icon, label?, x, y, size, opacity}`
  (`x`, `y` = default centre as screen fractions; `size` 32..=160 dp; `opacity` 10..=100 %), ≤ 4 per mod, draggable,
  the dropped position stored on the device per (mod, id). Event `<mod-id>:ui-button {id, action}` with `action` =
  `press` (touch down), `long-press` (held 500 ms without moving; still pressed), `release` (touch up; also before a
  drag starts).
- `open-settings(mod-id)`: the loader's settings screen for that mod: its `scope = "client"` settings (stored on the
  device), its key bindings, and for an admin its server settings (from `SETTINGS_SCHEMA`).

### Client-scope settings

On a client, `settings.get/all/set` use a settings file on the device (`HostOptions::client_settings`, one TOML
table per mod id), not `mods.toml`: values hold on every server. Only `scope = "client"` keys can be set there
(`settings.set` and the settings screen); `scope = "server"` keys read their default on the client. The server keeps
both scopes in `mods.toml` as before (V1).

## Protocol additions (compatible, proto stays 1)

`VOICE_STATE` (0x17) gains optional keys for the receiving player only: `modes` (uint mask, bit 0 proximity, 1 global,
2 channel, 3 broadcast: what `set-allowed` allows this player), `activations` (uint mask, bit 0 PTT, 1 VAD, 2 toggle)
and `bitrate` (uint, the encoder bitrate = `max-bitrate`), plus `names` (map playerID -> alias of players listed for
the first time; the full state names everyone) so a client knows every player. `icons` gains the key `deafen`.
`SETTINGS_RESULT` (0x1b) gains optional `mod` and `key` naming the answered `SETTINGS_SET`. Absent = unchanged (in a
delta) or the defaults (in a full state: no modes, all activations, 24000).

## Luau

`bh.voice.*`, `bh.terrain.solid_count`, `bh.input.*` and the `bh.ui` additions follow the usual mapping
([luau-api.md](luau-api.md)): players are player userdata, `list<mode>` is a list of strings, records are tables
(`{ listener = p, modes = {"proximity"}, gain = 0.5, pan = 0, muffle = 0 }`), `option` is `nil`.

## Host seams (Rust, `bhmod-core` / `bhmod-voice` / `bhmod-client`)

Zero-effect rule: every new seam has a default that reports `unsupported` (or does nothing), so a host that does not
implement it compiles and behaves exactly as before; without the voice mod no new byte goes on the wire.

### `bhmod-core`

- `bhmod_core::voice` (types mirroring the WIT: `Mode`, `Activation`, `VoiceState`, `Route` (listener = player id),
  `Caps`, `Transport`, `TransportConfig`, `Icons`, `MicState`, `VoiceEvent`) and the trait **`VoiceHost`**: one
  method per `voice` function, all `&self`, all defaulting to `Err(ApiError::Unsupported)`. The core checks the
  permission, the side, ranges and that players are online before calling it.
- `GameBridge` gains (all with defaults):
  - `fn voice(&self) -> Option<&dyn VoiceHost>` (default `None` = no voice);
  - `fn tile_solid(&self, x: i32, y: i32) -> Option<bool>` and `fn world_width(&self) -> Option<i32>` for
    `terrain.solid-count` (the Bresenham walk is in the core);
  - `fn input(&self, mod_id: &str, cmd: &InputCommand) -> Result<(), ApiError>` (`Bind {action, default_key}`,
    `Unbind {action}`, `ClearMod`) and `fn input_key(&self, mod_id: &str, action: &str, default_key: &str) ->
    Option<String>` (the player's current key; the default returns `default_key`);
  - `fn platform(&self) -> String` (`unknown`), `fn player_sleeping(&self, id) -> bool`, `fn player_alive(&self, id)
    -> bool`, `fn local_player(&self) -> Option<String>`, `fn player_has_mod(&self, id, mod_id) -> bool`,
    `fn has_touch(&self) -> bool` (all false / `None` by default).
  - New `UiCommand` variants through the existing `GameBridge::ui`: `SetOverhead {player, icon}`,
    `ClearOverhead {player}`, `SetButton {id, button}`, `RemoveButton {id}`, `OpenSettings {mod_id}` (`ClearMod`
    also removes the mod's icons and buttons).
- `ModHost` inputs from the host: `voice_event(&VoiceEvent)`, `ui_button(mod_id, id, ButtonAction) -> bool`,
  `input_event(mod_id, action, pressed) -> bool`, `ui_pick(player)`, `player_event(PlayerEvent::{Sleep, Wake,
  Respawn}, player)`.
- Settings commands: `ModHost::settings_command(is_admin, &args) -> Option<Vec<String>>` for the server's
  `/mods settings <id>` and `/mods set <id> <key> <value>` (the vanilla `/mod <player>` is not touched).
- Client-scope settings: `HostOptions::client_settings: Option<PathBuf>`; on a client host `settings_schema`,
  `set_setting` and the mod API read and write that file.
- Server `SETTINGS_*`: `bhmod_core::netsettings::SettingsNet`: `join(host, player, is_admin, features)` ->
  `SETTINGS_SCHEMA` packets for an admin who listed `settings`; `receive_set(host, player, is_admin, set, now)` ->
  the `SETTINGS_RESULT` packet (admin check, rate limit, validation, store); `refresh(host, &is_admin)` -> schema
  updates for admins after any settings change; `leave(player)`.

### `bhmod-voice` (GPL, hosts only)

- `bhmod_voice::server::VoiceServer` (shared as `SharedVoiceServer`, which implements `VoiceHost`): `join(player,
  alias, features)`, `leave(player)`, `receive(now, player, &Message, lag_ms, sink)` for `VOICE_KEY`, `VOICE_CTRL`,
  `VOICE_FRAME`, `receive_udp(now, datagram, &bound, lag_ms, sink) -> Option<peer to bind>`, `udp_port()`, `tick(now, sink)` (talk-stop, mute expiry,
  `VOICE_STATE` deltas), `take_events() -> Vec<VoiceEvent>` (pass to `ModHost::voice_event`). `now` is Unix time in
  ms. Output goes to a `ServerSink` (`reliable(player, packet)`, `unsequenced(player, packet)`, `udp(peer, datagram)`).
- `bhmod_voice::client::VoiceClient` (shared as `SharedVoiceClient`, which implements `VoiceHost`): `new(codec,
  unsequenced)`, `start() -> Option<VOICE_KEY packet>`, `on_message(&Message)` (`VOICE_KEY` reply, `VOICE_STATE`,
  `VOICE_FRAME`), `on_packet(&[u8])` (raw 0xBC), `on_udp(&[u8])`, `on_mic_frame(&[f32; 960 mono])`,
  `poll_send(now) -> Vec<Outgoing>` (`Reliable`, `Unsequenced`, `Udp`), `mix(&mut [f32] stereo)`, `mic_wanted()`,
  `mic_state(MicState)`, `transmitting() -> Option<Mode>` (host-drawn mic indicator), `take_events()`,
  `prefs()` / `load_prefs()` (master, per-player volumes and mutes as text for a file on the device),
  `players()` / `alias(id)` (every player the server listed with its alias: what the loader's bridge answers for
  `online_players` / `player_alias`), `speakers()` (players with a voice session, for a per-player volume list),
  `allowed()`, `default_mode()`, `self_muted()`, `deafened()`, `mic()`.
- Opus: `bhmod_opus_sys::OpusCodec` implements `bhmod_voice::codec::Codec` (in `bhmod-opus-sys`, so the server,
  which relays without decoding, does not build libopus).

### `bhmod-client` (MIT OR Apache-2.0, sans-IO)

The session stays free of GPL code: it routes the new ops and the host feeds them to `VoiceClient`.
`Output::Voice(Message)` (`VOICE_KEY`, `VOICE_STATE`, `VOICE_FRAME`, only when both sides listed `voice`),
`Output::SettingsSchema(SettingsSchema)`, `Output::SettingsResult(SettingsResult)`; `Session::voice_enabled()`,
`Session::settings_set(mod, key, value_cbor) -> Option<packet>`, `Session::voice_ctrl(...)`.

### What each host implements

| Host | Implements |
| --- | --- |
| blockheads-server | `GameBridge::voice` -> `SharedVoiceServer`; `tile_solid`, `world_width`, `platform`, `player_sleeping`, `player_alive`, `player_has_mod`; `ModHost::player_event` on sleep/wake/respawn; route `/mods settings|set` to `ModHost::settings_command`; feeds `VOICE_*` 0xBC packets and the UDP socket to `VoiceServer`, calls `tick` each poll and forwards `take_events` to `ModHost::voice_event`; `SettingsNet` for `SETTINGS_*`; lists `voice` in `SERVER_HELLO.features` only when a loaded mod holds the `voice` permission, `settings` always with mods. |
| PC client (blockheads-pc) | `GameBridge::voice` -> `SharedVoiceClient`; `local_player`, `platform`, `has_touch`; `ModHost::ui_pick` on blockhead clicks; mic capture -> `on_mic_frame`, `poll_send` -> unsequenced ENet / UDP, `mix` -> audio out; `input` + `input_key` (key bindings, rebinding UI, file); new `UiCommand`s (overhead icons on the blockhead render path, buttons, settings screen); `client_settings` path; `Output::Voice`/`Settings*` from the session. |
| Android loader (bhmod-android) | Same as PC with AAudio/OpenSL capture/playback, the draggable PTT button, the runtime `RECORD_AUDIO` prompt (`voice:mic`). |
