//! Parse and validate Blockheads mod manifests (`mod.toml`) and `.bhmod` packages.
//!
//! - [`Manifest::parse`] reads and validates `mod.toml` (schema 1).
//! - [`package::check`] applies the package rules to a source directory ([`package::DirSource`]) or a zip
//!   ([`package::ZipSource`]).
//! - [`pack::write_zip`] builds a deterministic `.bhmod`.
#![warn(missing_docs)]

use std::fmt;

pub mod content;
pub mod manifest;
pub mod pack;
pub mod package;
pub mod rules;

pub use manifest::Manifest;

/// The `blockheads:api` version this SDK describes.
pub const API_VERSION: &str = "0.3.0";

/// Every `blockheads:api` version a host of this SDK serves (oldest first).
///
/// Compatibility rule: inside 0.x a new minor only adds (events, functions, fields of new records); nothing is
/// removed or changed. A host that implements 0.N therefore also serves every earlier 0.M, and a mod's `api`
/// requirement is accepted when it matches any of these versions. `api = "^0.1"` (semver: `>=0.1.0, <0.2.0`) keeps
/// working on a 0.2 host; a mod that uses 0.2 additions declares `api = "^0.2"`. From 1.0 on, plain semver applies.
pub const SUPPORTED_API_VERSIONS: &[&str] = &["0.1.0", "0.2.0", "0.3.0"];

/// True when the requirement `req` (from `mod.toml` `api`) matches a version in [`SUPPORTED_API_VERSIONS`].
pub fn api_supported(req: &semver::VersionReq) -> bool {
    SUPPORTED_API_VERSIONS.iter().any(|v| req.matches(&semver::Version::parse(v).expect("constant")))
}

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
