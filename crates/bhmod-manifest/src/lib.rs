//! Parse and validate Blockheads mod manifests (`mod.toml`) and `.bhmod` packages.
//!
//! - [`Manifest::parse`] reads and validates `mod.toml` (schema 1).
//! - [`package::check`] applies the package rules to a source directory ([`package::DirSource`]) or a zip
//!   ([`package::ZipSource`]).
//! - [`pack::write_zip`] builds a deterministic `.bhmod`.
#![warn(missing_docs)]

use std::fmt;

pub mod manifest;
pub mod pack;
pub mod package;
pub mod rules;

pub use manifest::Manifest;

/// The `blockheads:api` version this SDK describes.
pub const API_VERSION: &str = "0.1.0";

/// The `mod.toml` schema version this crate reads.
pub const SCHEMA_VERSION: u32 = 1;

/// One validation problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Where: a manifest key (`entry.server`) or a package path.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl Issue {
    /// Creates an issue.
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Issue {
        Issue { path: path.into(), message: message.into() }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}
