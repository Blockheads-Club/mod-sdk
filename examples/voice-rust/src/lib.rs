//! The official `voice` mod (voice chat): one WASM component for both sides.
//!
//! - **Server** (`src/server.rs`): talk modes, the role × mode matrix (with permission groups of the `perms` mod), routes
//!   at 5 Hz (proximity falloff, pan, muffling; global; channels; broadcast), channels, `/voice` admin commands,
//!   persisted mutes and gains, caps, transport, icon visibility.
//! - **Client** (`src/client.rs`): push-to-talk keys per mode, the Android push-to-talk button with a long-press mode
//!   picker, overhead icons, the microphone opt-in, the volume dialog and `/voicevol`.
//!
//! The host does audio, Opus, encryption and relaying; this mod only decides policy (API `blockheads:api@0.3.4`).
//! The policy itself lives in plain modules ([`routing`], [`roles`], [`channels`], [`command`], [`config`],
//! [`ledger`], [`payload`], [`model`]) that build and test on the host; only the glue is WASM-only.

pub mod channels;
pub mod command;
pub mod config;
pub mod ledger;
pub mod model;
pub mod payload;
pub mod roles;
pub mod routing;

#[cfg(target_arch = "wasm32")]
mod client;
#[cfg(target_arch = "wasm32")]
mod glue;
#[cfg(target_arch = "wasm32")]
mod server;
