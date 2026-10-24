# {{id}}

A Blockheads server mod written in Luau.

```sh
bhmod check .     # validate
bhmod pack .      # build {{id}}-0.1.0.bhmod
```

Copy the `.bhmod` into your server's `mods/` folder.

## Talking to client mods (API 0.3.1)

Client and server mods can call each other's services over the game connection. The call is asynchronous: you give
a callback, which runs later. The provider opts in with `remote = true` (and optionally `remote-methods = [..]`) in
`[provides]` of `mod.toml`; `mod.toml` has a commented example. The caller needs the `channels` permission and
`api = "^0.3.1"`. Limits: timeout 5 s by default (30 s at most), 16 open calls per mod, 32 KiB for arguments and
results, and a rate limit per player. Inside the provider, `caller` says who called. Validate your arguments: a
crash of a remote-callable service counts as a fault of your mod. Full description: `docs/luau-api.md#remote-services-api-031` in the mod-sdk repository
and "Working with other mods" in the website docs.
