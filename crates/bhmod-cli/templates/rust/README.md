# {{id}}

A Blockheads server mod written in Rust, compiled to a WASM component.

```sh
rustup target add wasm32-wasip2   # once
./build.sh                         # builds server/mod.wasm
bhmod check .                      # validate
bhmod pack .                       # build {{id}}-0.1.0.bhmod
```

`bindings/blockheads-api/` holds the `blockheads:api` WIT the bindings are generated from; `src/`, `Cargo.toml` and
`target/` are not packed. Copy the `.bhmod` into your server's `mods/` folder.
