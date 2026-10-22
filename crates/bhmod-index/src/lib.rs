//! Validate and build the Blockheads mod index (`Blockheads-Club/mods`).
//!
//! - [`repo::Repo::load`] reads every `mods/<letter>/<id>.toml` entry and checks it against
//!   `index-entry.schema.json`.
//! - [`validate::validate`] runs the pull request rules (one mod per PR, append-only versions, owners, package
//!   download checks, dependencies, WASM and Luau checks, permission diff and review labels).
//! - [`build::build`] produces `index.json` (checked against `index.schema.json`).
//!
//! The index is not signed: it is served over HTTPS, and every package version records its `sha256` and `size`,
//! which CI, clients and servers check. The tool never touches the network: CI downloads packages (see [`validate::downloads`]) and passes the folder
//! with `--packages`, which is also the local test mode.
#![warn(missing_docs)]

pub mod build;
pub mod entry;
pub mod luau;
pub mod repo;
pub mod validate;
pub mod wasm;

pub use bhmod_manifest::Issue;
