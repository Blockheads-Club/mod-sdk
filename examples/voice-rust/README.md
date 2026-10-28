# voice: voice chat for The Blockheads (official mod)

Talk to players nearby, to your channel, to everyone, or (staff) broadcast to the whole server. The host programs
(server, PC port, Android loader) do the audio: microphone, Opus, encryption, relay and mixing. This mod only decides
**policy**: who may talk in which mode, who hears whom and how loud, channels, mutes, icons and the player's controls.
No API can read audio, and the server only relays sealed frames, so nothing can be recorded.

Needs a host with `blockheads:api@0.3.4` voice support. One WASM component (Rust, `wasm32-wasip2`) serves both sides.
License: GPL-3.0-or-later.

## For players

| Action | PC (rebindable) | Touch screen (Android) |
| --- | --- | --- |
| Talk nearby | hold `V` | hold the round mic button |
| Talk to your channel | hold `B` | long-press the mic button, pick a mode |
| Talk to everyone (if allowed) | hold `G` | same |
| Admin broadcast (staff) | hold `J` | same |
| Volumes, local mutes | `F8` or `/voicemenu` (`/vm`) | the headphones button |
| One player's volume or mute | click their blockhead | tap their blockhead |
| Mute your microphone / deafen | `F9` / `F10` | the voice settings |

- The microphone stays **off** until you say yes in the first dialog (setting `mic`: ask / on / off).
- `/voicevol <player> <0-200|mute|unmute>` and `/voicevol master <0-200>` change volumes on your device only; the
  server never learns them. The volume dialog does the same.
- Your settings (talk mode, push-to-talk / voice activation / toggle, threshold, master volume, button size and
  opacity) are in the loader's settings screen and hold on every server.
- Icons above blockheads: speaking, no voice (crossed headphones), self mute, deafened, admin mute; the server's admin
  decides which ones show.

## For admins

Settings are in `mods.toml` under `[voice.settings]`, the host app's Settings tab, or `/mods set voice <key> <value>`.
Defaults:

| Key | Default | Meaning |
| --- | --- | --- |
| `mode_proximity` / `mode_global` / `mode_channel` / `mode_broadcast` | on / off / on / on | Talk modes |
| `activation_ptt` / `_vad` / `_toggle` | on | Allowed activations |
| `talk_proximity` / `talk_global` / `talk_channel` / `talk_broadcast` | everyone / admin / everyone / admin, mod | Who may talk (role x mode matrix) |
| `proximity_radius`, `proximity_curve` | 32, `log` | Hearing radius in blocks, falloff (`linear`, `log`) |
| `proximity_pan`, `proximity_muffle`, `proximity_muffle_per_block` | on, on, 0.15 | Stereo direction, muffling through solid blocks |
| `dead_can_talk`, `sleeping_can_talk` | off, off | Whether dead / sleeping players talk nearby |
| `max_bitrate`, `max_speakers`, `max_frames_per_sec` | 24000, 8, 60 | Relay caps |
| `udp_port` | 0 (= game port + 1, 15152 by default) | The voice UDP port; see below |
| `channels`, `channels_player_created`, `channels_max_per_player` | none, on, 1 | Fixed channels (name, optional password); player channels |
| `broadcast_overrides_deafen` | off | Broadcast reaches deafened players |
| `icon_speaking` ... `icon_admin_mute` | on | Overhead icon visibility |
| `speaker_gain` | none | Per-player speaker volume (`{ player, percent }`) |

### Open the voice UDP port

Voice audio does **not** travel inside the game connection: it uses one extra **UDP** port that the server opens,
by default the game port + 1 (**15152** for the usual 15151; `udp_port` changes it). Control messages (keys, states,
mutes, settings) stay in the game connection. For players outside your network to hear each other:

- allow the port (UDP, incoming) in the server's firewall (`ufw allow 15152/udp`, Windows Defender Firewall, ...);
- forward it on your router (UDP 15152 -> the server machine), next to the game's own port;
- on a hosted server, open it in the provider's panel.

Without it the game still works, but voice chat stays off: clients get no voice port and the mod tells the player
"Voice chat is off: this server's voice UDP port is not open".

**Who may talk.** A list holds `owner`, `admin`, `mod`, `everyone` (a role includes the higher ones: `["mod"]` lets
moderators, admins and the owner talk) and **permission groups**: any other name `g` matches players who have the node
`group.<g>` in the official Permissions mod (`perms`, `/perm grant <player> group.<g>`). Groups are re-read every 5 s.

**Players without the mod** (vanilla clients) may join by default; they neither hear nor talk and others see a
"no voice" icon above them. Whether they may join at all is the host's `client-required` override for this mod
(`[voice] client-required = true` in `mods.toml`, or the toggle in the host app), not a setting of the mod.

Commands (`/voice help`):

- `/voice channel list | join <name> [password] | leave | create <name> [password]` (everyone; admins skip passwords
  and may create channels when players may not).
- `/voice mute <player> [30s|10m|2h|1d]` and `/voice unmute <player>` (moderators). Mutes are stored and come back
  after a restart; `unmute` works for players who are away.
- `/voice gain <player> <0-200>` (admins): speaker volume for everyone, stored; `100` goes back to the
  `speaker_gain` setting.

Other mods (clans, teams) can put players into channels through the `voice` service
([`wit/voice.wit`](wit/voice.wit)): `channel-of`, `join-channel`, `leave-channel`.

## How it works

- **Server**: every world step (5 Hz) the mod builds, for each speaker, the list of listeners with mode, gain, pan and
  muffle (`voice.set-routes`): proximity by distance (with `terrain.solid-count` for muffling), global and broadcast
  to everyone, channel to the channel's members. It tells each client what it may use (`voice.set-allowed`) and
  applies caps, the voice UDP port, icons, mutes and speaker gains.
  Proximity distance and pan go the short way round the world (`terrain.world-width`); dead (`player.alive`) and
  sleeping (`player.sleeping`) speakers follow the admin's rules, re-routed at once on `player:sleep|wake|respawn`.
- **Client**: binds the push-to-talk keys, shows the on-screen button on touch screens (`ui.has-touch`), offers the
  modes the server allows (`voice.allowed`, `voice:allowed`) in the long-press picker, opens the volume dialogs
  (also for a tapped or clicked blockhead, `ui-pick`), draws overhead icons from the `voice:*` events (never above the
  local player) and keeps the default-mode setting in step with `voice:mode`.

## Building and testing

```sh
./build.sh                                   # server/mod.wasm and client/mod.wasm (needs the wasm32-wasip2 target)
cargo test                                   # the policy modules on the host
cargo llvm-cov --ignore-filename-regex 'sdk/rust' --fail-under-lines 100
```

The policy (`src/{routing,roles,channels,command,config,ledger,payload,model}.rs`) has no host calls and is tested
on the host with 100 % line coverage; `src/{glue,server,client}.rs` are the WASM-only glue. The whole mod also runs in
the real mod host in `crates/bhmod-wasm/tests/voice_mod.rs` (after `scripts/build-wasm.sh`).

`index/v/voice.toml` is the draft entry for the public mod index (`Blockheads-Club/mods`); its `sha256` and `size`
must be refreshed from `bhmod pack` of the released package.

## Known limits

- Hosts that do not implement `player.alive` report every player as not alive, so proximity voice there needs
  `dead_can_talk = true` until the host tracks it.
