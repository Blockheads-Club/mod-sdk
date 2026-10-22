//! The code wasmtime generates from `wit/blockheads-api` (`blockheads:api@0.2.0`, world `server-mod`) and from the
//! frozen `wit/compat/blockheads-api-0.1.0` ([`v01`]), so components built against either version load.
#![allow(missing_docs)]

// One line on purpose: generated code is attributed to the lines of the invocation, and llvm-cov counts lines of
// generated functions that never run (unused WIT exports and helpers) as missed.
#[rustfmt::skip]
wasmtime::component::bindgen!({ path: "../../wit/blockheads-api", world: "server-mod", with: { "blockheads:api/players.player": PlayerRes }, imports: { default: trappable } });

#[path = "wit01.rs"]
pub mod v01;

/// The host side of a `player` resource: the player id.
pub struct PlayerRes(pub String);
