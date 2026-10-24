# Hot reload and time accounting

`bhmod-core` can restart a single mod from its package on disk without leaving the world, and it keeps a per-mod
account of the time each kind of call took. Both work the same on the server, on the PC client and in the Android
loader because they live in the shared host (`ModHost::reload`, `ModHost::timings`).

## Hot reload

```rust
let results = host.reload(Some("shop"));   // one mod, changed or not
let results = host.reload(None);           // every mod whose package changed, new packages, vanished packages
```

What a reload of one mod does, in order:

1. The package is read again (zip or unpacked folder, `mods.toml` too) and a new instance is created. If this fails
   (bad manifest, instance error) nothing else happens and the old version keeps running.
2. `on-disable` of the old version runs, then everything it registered is removed: event handlers, commands, scheduled
   tasks, services, custom-event handlers, channels, open remote calls, widgets and dialogs. Its storage is flushed.
3. The new version starts as at boot: storage opens again (**the data is kept**), `on-enable` runs, then `world-loaded`
   is delivered to that mod alone. Players who are already online are not announced again: read `players.online()`
   in `on-enable` if you track them.
4. Strike counters, the time budget state and the time accounting of that mod start from zero.

The result is one `ReloadResult` per mod: `Reloaded(old, new)`, `Unchanged` (only for a reload of everything),
`Added(version)`, `Removed(why)` or `Failed(why)`. A new version whose `on-enable` fails leaves the mod **disabled**
(reported as `Failed`); fix it and reload again.

Things to know:

- **Stale service handles.** A proxy from `bh.services.get(...)` that another mod got *before* the provider was
  reloaded is stale: calling it raises `stale service handle (the providing mod was reloaded); call services.get
  again`. Calls by name (`bh.services.call`) and proxies fetched after the reload work. While the provider is being
  reloaded its services are gone for a moment, so a call then gives `not-found`.
- **Dependents keep running.** Mods that depend on the reloaded mod are not restarted; fix the stale proxies in them or
  reload them too.
- **Content** (cosmetics, items, blocks) and **native libraries** are not reloaded. The ids of content live in the world;
  change them with a restart (server) or a new session (client).
- **JVM mods** get a fresh class loader, so static state is reset. Threads a JVM mod started itself are outside the
  host's control: stop them in `on-disable`.
- A reload must be called by the host program between mod calls (never from inside a guest call).

### Where to trigger it

| Where | How |
| --- | --- |
| Server | `/mods reload [id]` (admins and the console). `BLOCKHEADS_MODS_WATCH=1` starts a watcher that reloads changed mods by itself (about two checks per second, a change is applied once the files stopped changing). |
| PC client | F5 in developer mode, or the launcher's live reload of folder mods: the session folder is built again and only the mods that changed restart. |
| Android loader | `Loader::reload(only)` rebuilds the run directory from the server's packages and the enabled installed mods, then restarts what changed. |
| Your own host | `ModHost::reload`; `bhmod_core::watch::Watcher` is the polling watcher the server uses (`changed(now)` once per poll). |

## Time accounting

`ModHost::timings(only)` returns, per mod, a `ModTimings`:

| Field | Meaning |
| --- | --- |
| `kinds[]` | one entry per kind of call that happened: `event`, `command`, `task`, `service`, `channel`, `lifecycle` (`on-enable`/`on-disable`). Each has `calls`, `total`, `avg()`, `max`, `soft_hits` (calls over the soft budget), `timeouts` (stopped at the hard deadline) and `errors` (traps, panics, errors returned on purpose). |
| `strikes` | strikes inside the strike window right now (3 within 60 s disable the mod) |
| `strikes_total` | faults since the accounting started (a trap or a timeout) |
| `window` | how far back the numbers reach (the strike window, 60 s by default) |

The numbers are kept in one-second buckets, so memory stays small however busy a mod is. A call's time includes the
time of the mods it called (a service call, a custom event it fired). `ModHost::reset_timings(only)` clears the
account (it does not forgive strikes). `bhmod_core::timings::format_lines` formats the text the server prints.

On the server: `/mods timings [id]` and `/mods timings reset [id]`. On the PC client: press **T** in the Mods panel
(F9) to see the same numbers, **R** (or the Reset button) to clear them.

## Server commands

```text
/mods                       list the mods and their state
/mods reload [id]           reload the mods that changed on disk (or one mod)
/mods timings [id]          time per kind of call, budget hits, strikes
/mods timings reset [id]    clear the time accounting
```

Only admins and the console may use `/mods`. A mod cannot register a command with that name.
