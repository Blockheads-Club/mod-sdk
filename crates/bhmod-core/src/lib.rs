// bhmod-core: shared mod host core for the Blockheads Club server and clients.
// Copyright (C) 2026 Blockheads Club contributors
//
// This program is free software: you can redistribute it and/or modify it under the terms of the GNU General
// Public License as published by the Free Software Foundation, either version 3 of the License, or (at your
// option) any later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the
// implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
// more details.
//
// You should have received a copy of the GNU General Public License along with this program. If not, see
// <https://www.gnu.org/licenses/>.
//
// Additional permission under GNU GPL version 3 section 7: the "Blockheads Club Mod API Exception, version 1.0"
// in LICENSING.md at the root of the mod-sdk repository applies to this program.

//! The shared mod host of the Blockheads Club server and clients.
//!
//! - [`ModHost`] discovers packages in a mods directory ([`discover`]), applies `mods.toml` ([`config`]), orders
//!   them by dependencies ([`resolve`]) and runs them: event bus with Spigot priorities and cancellation ([`bus`]),
//!   commands ([`commands`]), scheduler ([`scheduler`]), per-world storage ([`storage`]) and time budgets
//!   ([`budget`]); cross-mod interop through services ([`services`]), custom events, channels ([`channels`]) and
//!   host handles ([`handles`]), with values as canonical CBOR ([`value`]).
//! - A host program implements [`GameBridge`] and calls the `ModHost` at its hook points.
//! - Languages plug in through [`Runtime`] / [`Instance`] / [`HostApi`] ([`runtime`]); Luau ships in `lua`.
#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod bridge;
pub mod budget;
pub mod bus;
pub mod channels;
pub mod commands;
pub mod config;
pub mod discover;
pub mod handles;
pub mod host;
#[cfg(feature = "luau")]
pub mod lua;
pub mod resolve;
pub mod runtime;
pub mod scheduler;
pub mod services;
pub mod storage;
pub mod types;
pub mod ui;
pub mod value;

pub use bridge::GameBridge;
pub use host::{HostOptions, ModHost, ModInfo, ModState};
pub use runtime::{CallError, HostApi, Instance, Runtime};

/// The `blockheads:api` version this host implements.
pub const API_VERSION: &str = "0.3.0";

/// The runtimes compiled into this build.
pub fn default_runtimes() -> Vec<Box<dyn Runtime>> {
    vec![
        #[cfg(feature = "luau")]
        Box::new(lua::LuauRuntime),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_version_matches_manifest_crate() {
        assert_eq!(super::API_VERSION, bhmod_manifest::API_VERSION);
        assert_eq!(super::default_runtimes().len(), 1);
    }
}
