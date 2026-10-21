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

//! Placeholder. In milestone M1 this crate becomes the shared mod host: package loading, permissions, event bus
//! (priority, cancellation), commands, scheduler, storage, budgets and the runtimes (Luau first, WASM in M2).

/// The `blockheads:api` version the host will implement.
pub const API_VERSION: &str = "0.1.0";

#[cfg(test)]
mod tests {
    #[test]
    fn api_version() {
        assert_eq!(super::API_VERSION, "0.1.0");
    }
}
