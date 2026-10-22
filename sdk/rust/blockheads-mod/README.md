# blockheads-mod

Rust SDK for [Blockheads](https://github.com/Blockheads-Club/mod-sdk) mods (`MIT OR Apache-2.0`).

It generates the `blockheads:api` bindings with `wit-bindgen` and adds closure-based events, commands and tasks, a
`Command` builder and a `Reply` type. Build mods for `wasm32-wasip2`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
blockheads-mod = { git = "https://github.com/Blockheads-Club/mod-sdk" }
```

See [`docs/wasm.md`](../../../docs/wasm.md) and the examples `hello-rust`, `chat-filter-rust` and `scheduler-rust`.
