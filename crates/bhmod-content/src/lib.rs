//! Content of the Blockheads mod system (milestone M8): what mods declare in `[content]`, the numeric ids a world
//! gives it, and how clients show cosmetics.
//!
//! - [`registry`]: per-world id allocation from free ranges and the `<world>/mods/registry.toml` sidecar. The
//!   server owns it; the result is sent in `MOD_LIST.registry`.
//! - [`cosmetics`]: the client's table of cosmetics (registry + manifests), the `COSMETICS` state, and the rule
//!   that decides which cosmetic a given blockhead shows in a slot.
//! - [`items`]: the items and variant blocks of the loaded mods, their names and textures.
//! - [`variant`]: the record a variant block keeps in the tile's spare bytes.
//! - [`texture`]: PNG decoding to RGBA for the clients' game hooks.
//! - [`client`]: the clients' state: packages, registry, who wears what, prepared textures.
//!
//! Nothing here touches the game, the network or a clock; the server and both clients share it, so they agree on
//! ids and on which texture is drawn.
#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod client;
pub mod cosmetics;
pub mod items;
pub mod registry;
pub mod texture;
pub mod variant;

pub use client::{Content, Look, Part, CLOTHING};
pub use cosmetics::{resolve, CosmeticDef, Resolved, Table, Wearing};
pub use items::{block_cell, item_cell, BlockDef, BlockTable, ItemDef, ItemTable};
pub use registry::{Kind, Registry};
pub use texture::{Gpu, Texture, PIXEL_FORMAT};
