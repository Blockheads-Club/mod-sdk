# 0xBC mod protocol, version 1

Status: **spec v0 (milestone M0)**. Implemented by `crates/bhmod-proto`. Byte-exact vectors: [`../vectors/`](../vectors/).
Server side: blockheads-server (M5); the PC loader lands in M6, the Android loader in M7. Every implementation must
pass the vectors. Section 10 collects the points the server implementation settled.

The protocol lets a modded Blockheads client and a modded server find each other, list the server's mods, move the
mod packages from the server to the client and then exchange mod channel traffic. It is added on top of the vanilla
1.7.x protocol without changing a single vanilla byte.

## 1. Rules

1. **A vanilla client never receives a new byte.** Everything new is sent only to a peer that sent `HELLO`.
2. **A modded client works against vanilla / official servers** unchanged (it falls back after a timeout).
3. **A server with no mods sends exactly the 1.7.1 bytes.** The 0xBC handler exists only when a mod host is
   installed.
4. One new packet type byte, `0xBC`, followed by a sub-opcode. ENet channels are **not** used to multiplex (the
   server sends on a channel chosen per peer and ignores the receive channel).
5. All 0xBC packets are sent **reliable**, like the vanilla packets.

## 2. Packet layout

```
+------+--------+---------------------------+
| 0xBC | op: u8 | body                      |
+------+--------+---------------------------+
```

- `body` is a **canonical CBOR map** (section 3) for every op except `FILE_CHUNK` (0x05) and `CHANNEL` (0x10), which
  use a fixed little-endian header plus raw bytes.
- A CBOR body is at most 1 MiB. Receivers drop larger packets.
- Unknown ops are ignored (not an error for the connection). Unknown **keys** inside a known message are ignored, so
  minor additions do not need a new `proto`.
- A malformed packet is dropped; a peer that keeps sending malformed packets may be disconnected.

## 3. CBOR profile

Bodies use a small subset of CBOR (RFC 8949): unsigned integers, byte strings, text strings, arrays, maps, `false`,
`true`, `null`. No negative integers, tags, floats, `undefined` or indefinite lengths.

Encoding is **core deterministic** (RFC 8949 §4.2.1): shortest integer/length form, definite lengths, map keys
sorted by the bytewise order of their encoded form, no duplicate keys. Receivers **reject** anything else (so a
decoded message re-encodes to the same bytes). Maximum nesting depth: 16.

Conventions: map keys are camelCase text; hashes are 32-byte byte strings; optional fields are **omitted** when
absent (never `null`).

## 4. Detection

```
client                                   modded server                  vanilla / official server
  |-- ENet connect --------------------->|                                 |
  |<-- 0x23 initial contact -------------|  (unchanged bytes)              |
  |-- 0xBC 00 HELLO -------------------->|                                 |  dropped by the pre-approval gate
  |<-- 0xBC 01 SERVER_HELLO -------------|                                 |  (no reply)
```

- The modded client sends `HELLO` immediately after it receives the server's first packet (0x23), before its 0x1f.
- It waits **1500 ms** for `SERVER_HELLO`. No reply: the server is vanilla; the client continues exactly like a
  vanilla client (sends its normal 0x1f without `bhcModLoader`) and never sends 0xBC on this connection again.
- Server: the pre-approval gate (only 0x1f passes for unapproved peers) additionally accepts 0xBC from unapproved
  peers **only when a mod host is installed**. Without mods the gate is unchanged.
- If `HELLO.proto` differs from the server's `proto`, the server sends `REJECT_TEXT` ("update your mod loader") and
  rejects with 0x26 reason 3.
- An optional hint key in the 0x23 plist (to skip the 1.5 s wait) is **not** part of version 1: vanilla tolerance of
  unknown 0x23 keys is unverified on 1.7.4-1.7.6 Android and iOS.

## 5. Messages

| Op | Dir | Name | Body |
| --- | --- | --- | --- |
| `00` | C->S | `HELLO` | `{proto, loader, platform, api, runtimes, cached}` |
| `01` | S->C | `SERVER_HELLO` | `{proto, modsHash, clientRequired, count}` |
| `02` | C->S | `LIST_REQUEST` | `{}` |
| `03` | S->C | `MOD_LIST` | `{mods, channels, registry}` |
| `04` | C->S | `FILE_REQUEST` | `{sha256, offset}` |
| `05` | S->C | `FILE_CHUNK` | raw: `sha256[32] offset:u64le total:u64le data[1..=32768]` |
| `06` | S->C | `FILE_ERROR` | `{sha256, code}` |
| `07` | C->S | `READY` | `{acceptedHash, loaded}` |
| `08` | S->C | `REJECT_TEXT` | `{reasonCode, text, url?}` |
| `10` | both | `CHANNEL` | raw: `channelId:u16le payload[0..=32768]` |
| `11` | both | `CHANNEL_REGISTER` | `{name, id}` |
| `12` | S->C | `COSMETICS` | `{full, players}` |
| `13` | both | `SERVICE_CALL` | `{id, mod, service, req, method, args}` |
| `14` | both | `SERVICE_RESULT` | `{id, ok, value}` or `{id, ok, code, message}` |

Ops `09`, `0a`-`0f` and `15`-`ff` are reserved.

### 00 HELLO (client -> server)

| Key | Type | Meaning |
| --- | --- | --- |
| `proto` | uint | Protocol version, `1`. |
| `loader` | text | Mod loader version (semver), e.g. `0.3.0`. |
| `platform` | text | `android-armv7`, `android-arm64`, `pc-linux-x86_64`, `pc-windows-x86_64`, `pc-macos-arm64`, ... |
| `api` | text | Highest `blockheads:api` version the loader implements, e.g. `0.1.0`. |
| `runtimes` | array of text | Available runtimes: `wasm`, `lua`, `jvm`, `native`. |
| `cached` | array of bytes(32) | SHA-256 of packages already in the client cache (may be empty; servers may use it to skip transfers). |

### 01 SERVER_HELLO (server -> client)

| Key | Type | Meaning |
| --- | --- | --- |
| `proto` | uint | `1`. |
| `modsHash` | bytes(32) | See section 7. |
| `clientRequired` | bool | True if a client that does not run the listed client mods is rejected. |
| `count` | uint | Number of entries the coming `MOD_LIST` will hold. |

A server whose mods are all server-only still answers (with `clientRequired = false`) so the client may show the
list; the client may then skip `LIST_REQUEST` and join as usual (it still adds `bhcModLoader` to 0x1f).

### 02 LIST_REQUEST (client -> server)

Empty map (`a0`). The full packet is `bc 02 a0`.

### 03 MOD_LIST (server -> client)

| Key | Type | Meaning |
| --- | --- | --- |
| `mods` | array of mod entries | Every mod the client should or may run, sorted by `id`. Server-only mods may be listed for information with `sides = ["server"]`. |
| `channels` | array of `{name: text, id: uint ≤ 65535}` | Channels open now (`<mod-id>:<name>` → wire id). |
| `registry` | map text → (map text → uint) | Content id map, see [Content registry](#content-registry): kind (`blocks`, `items`, `cosmetics`) → content key → numeric id for this world. Empty map when no listed mod declares content. |

Mod entry:

| Key | Type | Meaning |
| --- | --- | --- |
| `id`, `version` | text | From `mod.toml`. |
| `name`, `description` | map lang → text | From `mod.toml` (description may be an empty map). |
| `sides` | array of text | `server`, `client`. |
| `clientRequired` | bool | Whether this mod's client half is required to join. |
| `sha256` | bytes(32) | SHA-256 of the `.bhmod`. |
| `size` | uint | Size of the `.bhmod` in bytes. |
| `permissions` | map | `{world: text, players: [text], netHttp: [text], channels: bool, native: bool}` |
| `indexListed` | bool | The public index lists this id, version **and** sha256. |
| `sourceUrl` | text, optional | A URL the client may download from instead (the hash must match). |

The client shows the accept dialog between `MOD_LIST` and `READY`, using the game's own alert and never blocking the
game thread. The dialog lists names, permissions, "not in the public index" for `indexListed = false` and the red
warning for `native = true`. **A client refuses a pushed mod with `native = true` unless `indexListed = true` (same
sha256) and the player confirmed the red warning.**

### 04 FILE_REQUEST / 05 FILE_CHUNK / 06 FILE_ERROR

`FILE_REQUEST {sha256: bytes(32), offset: uint}` asks for the package with that hash starting at `offset`.

`FILE_CHUNK` is raw: `bc 05`, then `sha256` (32 bytes), `offset` (u64 LE), `total` (u64 LE, file size), then
`data`. Rules a receiver checks:

- `data` is 1..=32768 bytes; every chunk except the last of a file is exactly 32768 bytes;
- `offset` is a multiple of 32768; `offset + len(data) <= total`.

Flow and flow control:

1. The client requests only hashes not in its cache (cache keyed by SHA-256, shared across servers), one file at a
   time, in `MOD_LIST` order.
2. On `FILE_REQUEST{offset}` the server streams chunks from `offset` on, with at most **8 chunks** beyond the last
   acknowledged offset in flight.
3. The client acknowledges by sending `FILE_REQUEST` again with `offset` = number of bytes it holds contiguously
   (it should acknowledge at least every 4 chunks). The window slides to that offset.
4. When the client holds the whole file it sends a final `FILE_REQUEST` with `offset = total`. The server sends
   nothing in reply and the transfer is complete.
5. **Resume**: after a disconnect, or a partial file in the cache, the client starts with
   `FILE_REQUEST{offset = bytes it already has, rounded down to a multiple of 32768}`.
6. The client verifies the SHA-256 of the complete file, then runs the package rules (`bhmod-manifest`) before
   loading it. A mismatch deletes the file and fails the join.

`FILE_ERROR {sha256, code}`: `1` NOT_FOUND (no such hash), `2` BAD_OFFSET (past the end or not chunk-aligned),
`3` BUSY (rate limited; retry after a short delay), `4` INTERNAL.

Limits (server-configurable): a package ≤ 32 MiB, all client mods of a server ≤ 128 MiB, per-peer rate limit.

### 07 READY (client -> server)

`{acceptedHash: bytes(32), loaded: [{id: text, version: text}]}`: the player accepted, every required file is
present and verified, and the listed mods are loaded. `acceptedHash` is the `modsHash` from `SERVER_HELLO`.

`loaded` is also how a player's choice among **optional** client mods (`clientRequired = false`) reaches the server:
it holds every required client mod plus exactly the optional ones the player switched on; a declined optional mod is
absent. Declining never changes `acceptedHash` (section 7). A server may use the list to skip `CHANNEL` traffic for
mods a player does not run; a `READY` that was never sent (all mods server-only) means "no client mods".

### 08 REJECT_TEXT (server -> client)

`{reasonCode: uint ≤ 255, text: text, url?: text}`, sent only to modded clients, immediately followed by the vanilla
rejection `0x26 <reasonCode>`. The client shows `text` (and the link) instead of the vanilla message.

### 10 CHANNEL (both directions, after approval)

Raw: `bc 10`, `channelId` (u16 LE), payload (0..=32768 bytes). The server routes a client's payload to the mod that
registered the channel, with that player as sender; channel ids come from `MOD_LIST.channels` or
`CHANNEL_REGISTER`. Unknown channel ids are dropped.

### 11 CHANNEL_REGISTER (both directions)

`{name: text, id: uint ≤ 65535}`: a channel opened after `MOD_LIST` (hot reload, late registration).

The server owns the id space: it assigns an id the first time a channel name is used, never reuses or renumbers it
while it runs, and sends `CHANNEL_REGISTER` to a client in the game before the first `CHANNEL` on a new id (and for
channels opened later, on its next network poll). A `CHANNEL_REGISTER` from a client is ignored in version 1.

### 12 COSMETICS (server -> client)

`{full: bool, players: {<playerID>: {<slot>: <cosmetic id>}}}`: which cosmetics players wear. Sent only to modded
clients in the game, and only by servers whose mods declare cosmetics.

- `full = true` replaces everything the client knew. The server sends one right after approval (it is empty when
  nobody wears anything), before the client's world starts drawing other players.
- `full = false` is a delta: the set of every listed player is replaced by the one in the message, and a player with
  an empty map wears nothing. Players not listed are unchanged.
- The key is the vanilla `playerID` string, the same id the client sees as `clientID` of that player's blockheads
  (all of a player's blockheads show the same cosmetics).
- `slot` is one of `hat`, `shirt`, `pants`, `shoes`, `hair`, `weapon`; the id is a value of
  `registry.cosmetics`. A client ignores ids it does not know and slots it does not draw.
- A server resends a delta when a mod equips or takes off a cosmetic, and `{full: false, players: {id: {}}}` when the
  player leaves. Cosmetics are not saved by the server: mods re-equip on `player-join` from their own storage.

### 13 SERVICE_CALL / 14 SERVICE_RESULT (both directions, after approval)

Cross-side service calls (API 0.3.1, milestone M10): a client mod calls a service a server mod provides, and a server
mod one a client mod provides. Both ops are CBOR maps, sent reliably, only between a modded client that is in the game
and the server. The services are the ones of the mod host (`name@semver`, canonical CBOR values), see
`docs/luau-api.md#remote-services-api-031`.

`SERVICE_CALL`:

| Key | Type | Meaning |
| --- | --- | --- |
| `id` | uint ≤ 4294967295 | Chosen by the caller, unique among its open calls (`0` is never used). The answer carries it back. Ids of the two directions are independent. |
| `mod` | text | The calling mod (`[a-z0-9_-]{2,40}`), for the answering side's logs and for `services.caller`. |
| `service` | text | Service name. |
| `req` | text | Semver requirement on the service version (`^1.1`). |
| `method` | text | Method name (≤ 64 bytes). |
| `args` | bytes | Canonical CBOR **array** of the arguments, at most 32768 bytes. |

`SERVICE_RESULT`:

| Key | Type | Meaning |
| --- | --- | --- |
| `id` | uint | The call it answers. |
| `ok` | bool | `true`: the call returned. |
| `value` | bytes | With `ok = true`: the canonical CBOR result, at most 32768 bytes. |
| `code` | uint ≤ 255 | With `ok = false`: `1` NOT_FOUND (no provider of that name), `2` VERSION_MISMATCH, `3` NOT_PERMITTED (the service or method is not exposed to the other side), `4` BUSY (rate limited, too many calls, mods not running), `5` PROVIDER_ERROR (the provider returned an error on purpose), `6` TRAP (the provider crashed or ran out of time), `7` BAD_VALUE (arguments or result are not valid values), `8` TOO_LARGE, `9` INTERNAL. An unknown code reads as INTERNAL. |
| `message` | text | With `ok = false`: detail text, at most 256 bytes; for PROVIDER_ERROR the provider's own text without any traceback. |

Rules:

- The answering side runs the service under the **providing mod's** time budget, on its main thread, and answers with
  exactly one `SERVICE_RESULT` per accepted `SERVICE_CALL` (also a failure). It never answers a call it could not
  decode (malformed packets are dropped), so a caller always needs its own timeout.
- Only services the provider declared `remote = true` in its `mod.toml` `[provides]` (optionally only the methods in
  `remote-methods`) can be reached; everything else answers NOT_PERMITTED. Exposure is the provider's decision alone.
- The answering side limits each peer (reference server: a burst of 20 calls, then 10 per second per player, and 200 / 100 per second for
  all players together); more gets BUSY.
- The caller times out locally (default 5 s, at most 30 s; the call then fails with `timeout`, a late answer is
  dropped) and fails open calls with `disconnected` when the peer leaves. The ids, the timeout and the `unavailable`
  failure (no modded peer) are not on the wire.
- An older peer that does not know the ops ignores them (section 9), so the caller sees a timeout. Vanilla clients
  never receive a byte; a server without mods does not handle the ops.

## 6. Join after acceptance

After `READY` the client sends its normal 0x1f player-information plist with **one extra key**:

```xml
<key>bhcModLoader</key>
<dict>
	<key>acceptedHash</key>
	<string>cde01a3cfbdf76fb3bc63bfacf9f9a8be3f119f4bd3d9d627774508fc9226f4a</string>
	<key>proto</key>
	<integer>1</integer>
</dict>
```

`acceptedHash` is the `modsHash` as 64 lowercase hex characters (plist `<string>`, not `<data>`). Full example:
[`../vectors/plist/bhcModLoader.xml`](../vectors/plist/bhcModLoader.xml). A modded client that found a modded server
whose mods are all server-only also sends the key (`acceptedHash` = the server's `modsHash`).

Server handshake check (a new step between the vanilla checks and approval):

```
if mod_host.client_required():
    if 0x1f has no bhcModLoader                   -> reject 0x26 3 (CLIENT_TOO_OLD)            # vanilla client
    elif bhcModLoader.proto != 1                  -> REJECT_TEXT + reject 0x26 3
    elif bhcModLoader.acceptedHash != modsHash    -> REJECT_TEXT + reject 0x26 3               # stale / declined
```

The server **strips `bhcModLoader`** before it stores player info, so saved player plists stay vanilla. Then the
`player-pre-login` event runs, then approval as usual.

### Vanilla clients

- Server without client-required mods: vanilla clients join normally and never see 0xBC.
- Server with client-required mods: vanilla clients get `0x26 0x03` (CLIENT_TOO_OLD, full packet `26 03`); the
  vanilla client shows its own "update your game" text. Server list metadata and the website explain that the mod
  loader is installed with Patcher. A readable "limbo join" message is a later, device-tested option (M11).

## 7. modsHash

SHA-256 over the concatenation of the lines

```
<id>@<version>:<sha256 as 64 lowercase hex>\n
```

for every `MOD_LIST` entry with `clientRequired = true`, sorted bytewise, UTF-8. Server-only mods and optional
client mods (`clientRequired = false`, which the player may decline) are **not** included, so declining an optional
mod never changes the hash. `SERVER_HELLO.clientRequired` is true exactly when this set is not empty. The empty set
gives `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. Vectors: `modsHash` in
[`../vectors/vectors.json`](../vectors/vectors.json).

## Content registry

Mods with `[content]` give their blocks, items and cosmetics a string key `<mod-id>:<name>`. Packets carry numbers,
so the server gives every key an id the first time its world sees it, and sends the map in `MOD_LIST.registry`:

```
registry = { "cosmetics": { "hats:crown": 1, "hats:ember-hair": 2 }, "items": { "hats:crown-token": 1107 } }
```

- Kinds are `blocks`, `items`, `cosmetics`. A kind without a key is left out; the map is empty without content.
- Ids are per world and per kind. `0` is never an id. Ranges: `cosmetics` and `blocks` start at 1, `items` at 1107
  (above the game's own item types). All ids fit 16 bits.
- The server keeps the map in `<world>/mods/registry.toml` (`schema = 1`, one table per kind of `"key" = id`). It
  gives new keys, sorted, the lowest free id of their kind. It never changes or reuses an id, not even after the
  mod was removed (the key stays in the file), so a mod that comes back gets its ids back.
- The list holds only the keys of the mods listed in this `MOD_LIST`. A client matches the keys with the `[content]`
  of the packages it loaded; a key without a declaration, and a declaration without a key, are ignored.
- The registry is not part of `modsHash`: the ids are decided by the server and delivered with the list, and a
  change of the declarations changes the mods' hashes anyway.

## 8. Test vectors

`vectors/*.hex` hold one full packet each (lowercase hex + newline); `vectors/invalid/*.hex` must be rejected with
the error class named in `vectors.json` (`not-mod-packet`, `truncated`, `unknown-op`, `cbor`, `missing`, `type`,
`too-large`, `not-map`, `invalid`). `vectors.json` also shows every CBOR body as JSON (byte strings as
`{"$bytes": "<hex>"}`), the `modsHash` cases and the vanilla rejection packet.

An implementation passes when, for every valid vector, decoding gives the fields in `vectors.json` and encoding those
fields gives the exact bytes; and every invalid vector is rejected. The Rust reference test is
`crates/bhmod-proto/tests/vectors.rs` (`BHMOD_BLESS=1` regenerates the files after an intended change). The vectors
were cross-checked with Python `cbor2` (`canonical=True`).

Example, `02-list-request.hex`: `bc02a0`. `10-channel.hex` (channel 1, payload `hello`): `bc10010068656c6c6f`.

## 9. Versioning

`proto` is bumped only for incompatible changes (`COSMETICS`, `SERVICE_CALL` and `SERVICE_RESULT` are compatible
additions: an older client ignores the unknown op). New keys and new ops are compatible additions (ignored by older
peers). The API version (`HELLO.api`, `mod.toml` `api`) is independent of `proto`.

## 10. Server behaviour (blockheads-server, M5)

Clarifications that hold for the reference server; clients must not depend on more than the rules above.

- Before `HELLO` every 0xBC op is ignored; a `HELLO` from a player already in the game is ignored. Only waiting peers
  (between ENet connect and approval) may start the protocol; `CHANNEL` is accepted only from players in the game.
- `MOD_LIST.mods` holds every enabled package with a client half whose server half (if any) loaded, plus the loaded
  server-only mods for information (`sides = ["server"]`, never downloadable: `FILE_ERROR` NOT_FOUND). `sides` is
  written in the order `server`, `client`. Unpacked mod folders are packed with the deterministic `.bhmod` writer of
  `bhmod pack`, so their hash is stable.
- A server never lists a client mod with `permissions.native = true` (it cannot push native code); such a mod is
  only reachable through the index (M10). A server sets `indexListed` only when it checked the index itself (the M5
  server does not and always sends `false`); a client must check its own copy of the index either way.
- Downloads: the window is 8 chunks beyond the last acknowledged offset per client. A `FILE_REQUEST` below the last
  acknowledged offset restarts the transfer there (resume on the same connection). A client that asks for another
  file drops its current transfer. When too many clients download at once (8 on the reference server), new
  transfers get `FILE_ERROR` BUSY; the final acknowledgement (`offset = total`) or `READY` frees the slot.
- `REJECT_TEXT` precedes the 0x26 rejection of a modded client (one that sent `HELLO`) for a protocol mismatch, a
  missing or stale `bhcModLoader`, and a refusal by a server mod (`player-pre-login`; the reason byte is the mod's).
  Peers that never sent `HELLO` get only the vanilla 0x26.
- `SERVICE_CALL` and `SERVICE_RESULT` are accepted only from players in the game that sent `HELLO` (the same rule as
  `CHANNEL`). A server mod calling a player without the mod loader (or not in the game) fails locally with
  `unavailable`; nothing is sent. The `mod` of an incoming call must be a mod in the player's `READY.loaded` (a peer that
  never sent `READY` is not restricted, as for channels), otherwise the answer is NOT_PERMITTED and no token is used;
  beyond that the provider's `remote = true` is the gate.
- A server mod sending on a channel to a player without the mod loader (or not in the game) is a silent no-op;
  `all-players` reaches every modded player in the game.
