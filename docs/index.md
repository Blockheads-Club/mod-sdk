# The mod index tools (`bhmod index`)

The public mod list lives in the `Blockheads-Club/mods` repo: one file per mod, `mods/<first letter>/<id>.toml`
(schema: [`schema/index-entry.schema.json`](../schema/index-entry.schema.json)). Its CI runs these commands; you can
run them yourself before you open a pull request.

| Command | What it does |
| --- | --- |
| `bhmod index validate` | Checks the pull request rules (below). `--base <dir>` is a checkout of the base branch, `--author <login>` the PR author, `--packages <dir>` a folder with the packages as `<id>-<version>.bhmod`, `--luau-compile <bin>` turns on the Luau syntax check, `--report`/`--labels` write the PR comment and labels. |
| `bhmod index downloads` | Prints `<url> <file>` for every package `validate` needs. The tool never downloads anything itself: CI fetches the files and passes the folder with `--packages` (that is also the local test mode). |
| `bhmod index build` | Writes `index.json` ([`schema/index.schema.json`](../schema/index.schema.json)). Draft entries are left out unless `--include-drafts`; `--mirror-base` fills in `mirror` URLs. |

## Rules checked by `validate`

1. Every entry file parses and matches the schema; the file name and folder match the id.
2. One mod per pull request (`--allow-multiple` for maintainer edits).
3. Published versions are append-only: no edits, removals or reordering. `yanked = true` may be added (never removed);
   `mirror` is set by CI. New versions must be newer than the last one. Mods are not deleted, only yanked.
4. Owners: on an existing mod the PR author must be in its current `owners`; a new mod must list the author and gets
   the `new-mod` and `needs-security-review` labels (a maintainer reviews its code and approves it).
5. For every new version: `size` is set, the package's SHA-256 and size match, the package rules pass with the index
   rules on (`bhmod check --index`), and the package's `mod.toml` agrees with the entry (id, version, sides,
   client-required, api, loader, dependencies, permissions).
6. `api` matches a supported `blockheads:api` version (`--api`, repeatable; default: every version the SDK serves,
   see [`luau-api.md`](luau-api.md#api-versions)); every dependency exists in the index with a non-yanked matching
   version (soft dependencies only warn; only drafts may depend on drafts).
7. Every `.wasm` validates and imports only `blockheads:api/*` and sandboxed WASI (`wasi:cli`, `wasi:io`,
   `wasi:clocks`, `wasi:random`, `wasi:filesystem`); core modules may import from `bh`, `env` and
   `wasi_snapshot_preview1`. Every `.luau` is non-empty UTF-8 and, with `--luau-compile`, compiles.
8. The comment lists the permissions of the newest version and what changed since the previous one.
   `needs-security-review` is added for every new mod (the first submission of an id), when a new version asks for
   `native`, and when an update adds any permission compared with the last published version.

## Publishing

On every push to `dev` (the release branch), the mods repo's `publish.yml` mirrors new packages as release assets,
runs `bhmod index build --mirror-base …` and uploads `index.json` to the fixed release `index`. The website copies it
at build time and serves it at `https://blockheads.club/mods/index.json`. The index is not signed: it is fetched over
HTTPS, and every version's `sha256` and `size` are checked by CI, the game, the Patcher and the server before a package
is used.
