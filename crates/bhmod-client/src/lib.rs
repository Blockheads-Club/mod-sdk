//! The client side of the Blockheads Club 0xBC mod protocol (`docs/protocol-0xbc.md`), shared by the Android loader
//! (M7) and usable by the PC loader: a sans-IO join [`session`], the package [`cache`], package checks
//! ([`verify`]) and the player's own [`installed`] mods. No network, threads or UI here: the host feeds packets and
//! the clock in and acts on the returned [`session::Output`]s.
#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod cache;
pub mod installed;
pub mod session;
pub mod verify;

#[cfg(test)]
mod testutil;

pub use cache::PackageCache;
pub use session::{Options, Output, Session};
pub use verify::{Caps, Refusal};
