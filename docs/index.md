# The mod index tools (`bhmod index`)

The public mod list lives in the `Blockheads-Club/mods` repo: one file per mod, `mods/<first letter>/<id>.toml`
(schema: [`schema/index-entry.schema.json`](../schema/index-entry.schema.json)). Its CI runs these commands; you can
run them yourself before you open a pull request.

| Command | What it does |
| --- | --- |
| `bhmod index validate` | Checks the pull request rules (below). `--base <dir>` is a checkout of the base branch, `--author <login>` the PR author, `--packages <dir>` a folder with the packages as `<id>-<version>.bhmod`, `--luau-compile <bin>` turns on the Luau syntax check, `--report`/`--labels` write the PR comment and labels. |
| `bhmod index downloads` | Prints `<url> <file>` for every package `validate` needs. The tool never downloads anything itself: CI fetches the files and passes the folder with `--packages` (that is also the local test mode). |
| `bhmod index build` | Writes `index.json` ([`schema/index.schema.json`](../schema/index.schema.json)). Draft entries are left out unless `--include-drafts`; `--mirror-base` fills in `mirror` URLs. |
| `bhmod index sign <file>` | Writes `<file>.minisig` with a minisign secret key (`--key <file>` or `--key-env <VAR>`, `--password-env <VAR>` for encrypted keys). |
| `bhmod index verify <file> --pubkey <file>` | Checks `<file>.minisig`. |
| `bhmod index keygen --dir <dir>` | Creates `index.pub` and `index.key` (unencrypted; keep the key secret). |

## Rules checked by `validate`

1. Every entry file parses and matches the schema; the file name and folder match the id.
2. One mod per pull request (`--allow-multiple` for maintainer edits).
3. Published versions are append-only: no edits, removals or reordering. `yanked = true` may be added (never removed);
   `mirror` is set by CI. New versions must be newer than the last one. Mods are not deleted, only yanked.
4. Owners: on an existing mod the PR author must be in its current `owners`; a new mod must list the author and gets
   the `new-mod` label (a maintainer approves it).
5. For every new version: `size` is set, the package's SHA-256 and size match, the package rules pass with the index
   rules on (`bhmod check --index`), and the package's `mod.toml` agrees with the entry (id, version, sides,
   client-required, api, loader, dependencies, permissions).
6. `api` matches a supported `blockheads:api` version (`--api`, default the SDK's); every dependency exists in the
   index with a non-yanked matching version (soft dependencies only warn; only drafts may depend on drafts).
7. Every `.wasm` validates and imports only `blockheads:api/*` and sandboxed WASI (`wasi:cli`, `wasi:io`,
   `wasi:clocks`, `wasi:random`, `wasi:filesystem`); core modules may import from `bh`, `env` and
   `wasi_snapshot_preview1`. Every `.luau` is non-empty UTF-8 and, with `--luau-compile`, compiles.
8. The comment lists the permissions of the newest version and what changed since the previous one.
   `needs-security-review` is added when a new version asks for `native`, when an update adds any permission, or when
   a new mod asks for `net-http`.
