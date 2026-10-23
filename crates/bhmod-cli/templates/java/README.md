# {{id}}

A Blockheads server mod written in Java (any JVM language works: Kotlin, Scala, ...).

JVM code is not sandboxed. The mod declares `permissions.native = true`, the server owner must grant `native` in
`mods.toml`, and the host must be started with its JVM support switched on.

```sh
mkdir libs && cp /path/to/blockheads-mod-sdk.jar libs/    # from the mod-sdk release
./build.sh               # or: gradle jar      -> server/mod.jar
bhmod check .            # validate
bhmod pack .             # build {{id}}-0.1.0.bhmod
```

Copy the `.bhmod` into your server's `mods/` folder and grant the permissions it asks for in `mods.toml`.
The API reference is the generated Javadoc of the SDK; the guide is `docs/jvm.md` in the mod-sdk repository.
