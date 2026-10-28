# Mod settings (API 0.3.3)

A mod declares the settings its admin can change in `mod.toml`. The host stores the values in `mods.toml`, checks
them against the declaration, gives them to the mod (`bh.settings`, the WIT `settings` interface) and tells the mod
when they change. The same declaration drives every editor: `/mods set` in chat or on the console, the Settings tab of
the Blockheads Server app and (later) the in-game admin screen.

## Declaring settings

One table per setting, in the order the forms show them:

```toml
api = "^0.3.3"

[settings.radius]
type = "int"
default = 32
min = 4
max = 128
step = 4
label = { en = "Hearing radius (blocks)", tr = "Duyma yarıçapı (blok)" }
help = "How far proximity voice carries."
group = "proximity"

[settings.curve]
type = "enum"
values = ["linear", "log"]
default = "log"
label = "Falloff"
restart = true

[settings.talk]
type = "matrix"
rows = ["proximity", "global"]
cols = ["admin", "everyone"]
cell = "bool"
default = { proximity = { admin = true, everyone = true }, global = { admin = true, everyone = false } }
label = "Who may talk"

[settings.channels]
type = "list"
default = [{ name = "main" }]
max_items = 16
label = "Channels"
item = { name = { type = "string", default = "", max_length = 32, label = "Name" }, password = { type = "string", default = "", label = "Password" } }
```

| Key | For | Meaning |
| --- | --- | --- |
| `type` | all | `bool`, `int`, `float`, `enum`, `string`, `roles`, `matrix`, `list` |
| `default` | all | required; must be a valid value |
| `label` | all | required: an English text or `{ en = "...", tr = "...", ru = "...", kk = "..." }` (`en` required, 1-128 characters) |
| `help` | all | optional, same form (1-1024 characters) |
| `group` | top level | optional group key (`[a-z0-9_]{1,48}`); forms put a group's settings together |
| `scope` | top level | `server` (default): set by the server admin; `client`: each player's own setting, stored on the client |
| `restart` | top level | `true`: the mod reads it only at start; editors say so. Default `false` |
| `min`, `max`, `step` | `int`, `float` | bounds (integers for `int`); `step` is a hint for sliders and is not enforced |
| `values` | `enum` | 1-64 distinct texts |
| `max_length` | `string` | most characters, 1-65536, default 256 |
| `rows`, `cols`, `cell` | `matrix` | row and column keys; `cell` is `bool` (default) or `roles` |
| `item`, `max_items` | `list` | the fields of one record (any type but `list`, no `group`/`scope`/`restart`); at most `max_items` records (1-1000, default 100) |

Keys of settings, groups, rows, columns and item fields match `[a-z0-9_]{1,48}`. A mod has at most 128 settings.

### Values

| Type | Value |
| --- | --- |
| `bool` | `true` / `false` |
| `int` | an integer within ±2^53 |
| `float` | a number (an integer is taken as a float) |
| `enum` | one of `values` |
| `string` | a text of at most `max_length` characters |
| `roles` | a list of distinct role names (`[a-z0-9_-]{1,40}`): `owner`, `admin`, `mod`, `everyone` or a group of a permissions mod |
| `matrix` | `{ row = { col = cell } }` with every row and every column |
| `list` | a list of records; a record may leave out fields (they get their default) but has no unknown ones |

## Storing values: `mods.toml`

```toml
[voice]
grant = ["channels"]
client-required = false     # players without the mod may join (admin override; not for mods with [content])

[voice.settings]
radius = 48
talk = { proximity = { admin = true, everyone = true }, global = { admin = true, everyone = true } }
```

- A setting that is not in the file has its default. An invalid value or an unknown key is ignored with a warning in
  the log; the default applies.
- Edits of `mods.toml` while the game runs are picked up within a second. A file that does not parse changes nothing
  (an error is logged and the old values stay). `enabled`, `grant` and `client-required` still need `/mods reload`
  or a restart.
- Changes made through the host (`/mods set`, the host app, `bh.settings.set`) are written back to `mods.toml`, touching
  only that one key: comments, the order and the other mods stay. A value set back to its default is removed.

## Changing settings

- **Chat or console** (admins and the console only):
  - `/mods settings <mod>` lists the values.
  - `/mods set <mod> <key> <value>`: `bool` takes `true/false/on/off/yes/no/1/0`, `int` and `float` a number, `enum`
    and `string` the rest of the line, `roles` `owner, admin` or a JSON list, `matrix` and `list` JSON
    (`/mods set voice talk {"proximity": {"admin": true, "everyone": false}, "global": {"admin": true, "everyone": false}}`).
- **The mod itself**: `bh.settings.set(key, value)` (WIT `settings.set`), its own keys only.
  These are subcommands of the server's own `/mods` command (next to `/mods reload|list|timings`); the vanilla
  `/mod <player>` (add a moderator) stays the game's. A host program routes them with
  `ModHost::settings_command(is_admin, &args)` (`None` for other subcommands).
- **Host programs**: `ModHost::set_setting(id, key, &json)` (or `set_setting_cbor` with a canonical CBOR value).
- **In-game admin screen** (API 0.3.4): 0xBC `SETTINGS_SCHEMA` / `SETTINGS_SET` / `SETTINGS_RESULT`, served by
  `bhmod_core::netsettings::SettingsNet` on the server. Client-scope settings (`scope = "client"`) are stored on the
  player's device instead (see [voice.md](voice.md)).

Every change that alters a value fires the custom event `<mod-id>:settings-changed` with the payload
`{"keys": [...]}`; other mods can subscribe to it too.

```lua
local function apply()
    radius = bh.settings.get("radius")
end

function mod.on_enable()
    apply()
    bh.events.on_custom("voice:settings-changed", function(ev) apply() end)
end
```

## Reading settings

| Language | Calls |
| --- | --- |
| Luau | `bh.settings.get(key)`, `bh.settings.all()`, `bh.settings.set(key, value)` ([luau-api.md](luau-api.md)) |
| WIT / WASM | `settings.get(key) -> option<value>`, `settings.all() -> value`, `settings.set(key, value)`; Rust SDK `blockheads_mod::settings` |
| Java | `api.settings().get(key)`, `.all()`, `.set(key, value)` |

## The canonical JSON (host app, 0xBC)

`bhmod_manifest::settings::schema_json` / `ModHost::settings_schema(id)` give

```json
{
  "fields": [
    { "key": "radius", "type": "int", "default": 32, "label": { "en": "Hearing radius (blocks)", "tr": "..." },
      "help": { "en": "..." }, "group": "proximity", "scope": "server", "restart": false, "min": 4, "max": 128, "step": 4 },
    { "key": "channels", "type": "list", "default": [{ "name": "main", "password": "" }], "label": { "en": "Channels" },
      "scope": "server", "restart": false, "max_items": 16,
      "item": [
        { "key": "name", "type": "string", "default": "", "label": { "en": "Name" }, "max_length": 32 },
        { "key": "password", "type": "string", "default": "", "label": { "en": "Password" }, "max_length": 256 }
      ] }
  ],
  "values": { "radius": 48, "channels": [{ "name": "main", "password": "" }] }
}
```

- `fields` is in declaration order; `values` has every key.
- `help`, `group`, `min`, `max`, `step` appear when declared; `values` (enum), `max_length` (string), `rows`/`cols`/
  `cell` (matrix) and `item`/`max_items` (list) always appear for their type, with defaults filled in.
- `item` is a list of field objects in declaration order, shaped like top-level fields without `group`, `scope` and
  `restart`.
- 0xBC `SETTINGS_SCHEMA` and `SETTINGS_SET` carry the schema and values as canonical CBOR byte strings:
  `ModHost::settings_schema_cbor`, `bhmod_core::settings::{to_cbor, from_cbor}`.

### Errors

A rejected value gives `SettingsError`, as JSON (`SettingsError::to_json`):

```json
{ "code": "setting", "detail": "min", "key": "radius", "message": "radius: must be at least 4", "expected": "4", "got": "1" }
```

`code` is `settingKey` (no such setting) or `setting`; `detail` is the rule: `type`, `number`, `integer`, `min`,
`max`, `choice`, `length`, `items`, `unknown` or `role`. `key` is the path of the bad part (`talk.global.admin`,
`channels[2].name`).
