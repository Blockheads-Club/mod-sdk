//! The code wasmtime generates from `wit/blockheads-api` (world `server-mod`).
#![allow(missing_docs)]

// One line on purpose: generated code is attributed to the lines of the invocation, and llvm-cov counts lines of
// generated functions that never run (unused WIT exports such as `call-service`) as missed.
#[rustfmt::skip]
wasmtime::component::bindgen!({ path: "../../wit/blockheads-api", world: "server-mod", with: { "blockheads:api/players.player": PlayerRes }, imports: { default: trappable } });

/// The host side of a `player` resource: the player id.
pub struct PlayerRes(pub String);
