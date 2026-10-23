# Mod packages (`.bhmod`)

A `.bhmod` is a plain zip, normally `<id>-<version>.bhmod`, built with `bhmod pack`. The rules below are checked by
`bhmod check` (crate `bhmod-manifest`); servers, clients and the index CI use the same crate.

```
mod.toml          required (schema: ../schema/mod.schema.json)
LICENSE           required for the public index (also LICENSE.md, LICENSE.txt); README.md, CHANGELOG.md allowed
server/           server entry point and its files
client/           client entry point; native/<abi>/lib<name>.so only with permissions.native = true
wit/              WIT of services this mod provides
assets/           client assets (PNG textures, sounds, lang files)
data/             default config, recipes, loot tables
```

| Rule | Limit |
| --- | --- |
| Paths | relative, `/`-separated, no `\`, `:`, `.`/`..`, empty or hidden (`.x`) components, ≤ 240 bytes, no control characters |
| Duplicates | none, compared case-insensitively |
| Symlinks, encrypted entries | not allowed |
| Compression | stored or deflate only |
| Files | ≤ 4096 |
| Size | ≤ 32 MiB zip, ≤ 128 MiB uncompressed |
| `mod.toml` | ≤ 64 KiB, valid against schema 1 plus the semantic rules |
| Entry points | exist; `.wasm` starts with the WASM magic (component `0d 00 01 00` or core module `01 00 00 00`), `.luau` is non-empty UTF-8, `.jar` is a zip |
| Native ABIs | `arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`; `server/native/` is not supported in schema 1 |
| Textures | every `assets/**.png` has a valid PNG header, edges 1-1024 px; every `[content]` texture exists |
| Services | every `[provides]` WIT file exists |

When `bhmod check` / `bhmod pack` read a **directory**, only `mod.toml`, the root files above and the five
directories are part of the package; everything else (`src/`, `Cargo.toml`, `target/`, dotfiles) is listed as "not
packed" and ignored. Inside a zip, any other file is an error.

`bhmod pack` writes entries in path order, deflated, with the timestamp 1980-01-01 00:00 and mode 0644, so the same
input always gives the same bytes and the same SHA-256.

## Semantic `mod.toml` rules (beyond the JSON Schema)

- `api` must match one of the API versions the SDK serves (`0.1.0`, `0.2.0`, `0.3.0`; see [luau-api.md](luau-api.md#api-versions)); `loader` and dependency versions must be semver requirements.
- Each enabled side needs exactly one entry point inside `<side>/` (not `<side>/native/`); a disabled side must not have one.
- Runtime and extension match: `wasm` → `.wasm`, `lua` → `.luau`, `jvm` → `.jar`. `jvm` needs `permissions.native = true`; the jar's manifest names the entry class (`Bhmod-Main`, else `Main-Class`) and may carry `classes.dex` for Android (see [jvm.md](jvm.md)).
- No self-dependency; an id may not be in both `dependencies` and `soft-dependencies`.
- `permissions.net-http` needs a server side.
- `[content]` needs `sides.client = true` and forces `client-required = true` (explicit `false` is an error). Keys are
  `<id>:<name>`, unique; textures are PNGs under `assets/`. A cosmetic's `slot` is one of `hat`, `shirt`, `pants`,
  `shoes`, `hair`, `weapon`; `base` is a known vanilla item of that slot; `texture-arm` is for `shirt` only
  ([content.md](content.md)).
- `client-required` defaults to `sides.client`.
