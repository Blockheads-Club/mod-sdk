//! The code wasmtime generates from the frozen `wit/compat/blockheads-api-0.2.0` (`blockheads:api@0.2.0`).
#![allow(missing_docs)]

pub use super::PlayerRes;

// One line on purpose, see `wit.rs`.
#[rustfmt::skip]
wasmtime::component::bindgen!({ path: "../../wit/compat/blockheads-api-0.2.0", world: "server-mod", with: { "blockheads:api/players.player": PlayerRes }, imports: { default: trappable } });
