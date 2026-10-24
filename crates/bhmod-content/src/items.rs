//! Items and variant blocks on the client: which declared key has which id (the world's registry), what it is
//! called and which texture it uses.
//!
//! Items are real `ItemType`s above the vanilla ones (registry kind `items`, ids from 1107). They exist only on
//! sessions where every peer runs the mod, so the game's own tables never need to know them: the clients hook the
//! few places that ask for a name or an icon. Variant blocks (registry kind `blocks`) are a vanilla block plus a
//! record in the tile ([`crate::variant`]).

use std::collections::BTreeMap;

use bhmod_manifest::content::base_tile;
use bhmod_manifest::manifest::Localized;
use bhmod_manifest::Manifest;

use crate::registry::{Kind, Wire};

/// The first cell of the game's item atlas (`Items.png`, 32 cells per row) that no vanilla item uses: 344 cells are
/// drawn (the last ones of the 11th row), the 168 after them are free.
pub const ITEM_ATLAS_FIRST_CELL: u32 = 344;
/// How many free cells the item atlas has.
pub const ITEM_ATLAS_CELLS: u32 = 168;
/// The first cell of the game's tile atlas (`TileMap.png`, 32 cells per row) that is free: the last five rows (160
/// cells) are empty.
pub const BLOCK_ATLAS_FIRST_CELL: u32 = 864;
/// How many free cells the tile atlas has.
pub const BLOCK_ATLAS_CELLS: u32 = 160;
/// Cells per row of both atlases.
pub const ATLAS_COLUMNS: u32 = 32;

/// The item atlas cell of item `id` (an `ItemType` from [`crate::Kind::Items`]'s range), if the atlas has room.
pub fn item_cell(id: u64) -> Option<u32> {
    let k = u32::try_from(id.checked_sub(crate::registry::Kind::Items.first_id())?).ok()?;
    (k < ITEM_ATLAS_CELLS).then_some(ITEM_ATLAS_FIRST_CELL + k)
}

/// The tile atlas cell of variant block `id` (from 1), if the atlas has room.
pub fn block_cell(id: u64) -> Option<u32> {
    let k = u32::try_from(id.checked_sub(crate::registry::Kind::Blocks.first_id())?).ok()?;
    (k < BLOCK_ATLAS_CELLS).then_some(BLOCK_ATLAS_FIRST_CELL + k)
}

/// An item a loaded mod declares and the world has an id for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDef {
    /// The world's id, an `ItemType` (`registry.items[key]`).
    pub id: u64,
    /// `<mod-id>:<name>`.
    pub key: String,
    /// The mod that declares it.
    pub mod_id: String,
    /// Icon path inside the package (`assets/...png`).
    pub texture: String,
    /// Display name per language.
    pub name: Localized,
    /// Plural display name per language (may be empty).
    pub plural: Localized,
}

impl ItemDef {
    /// The mod and the path of the icon inside its package.
    pub fn source(&self) -> (String, String) {
        (self.mod_id.clone(), self.texture.clone())
    }
}

/// A variant block a loaded mod declares and the world has an id for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDef {
    /// The world's id (`registry.blocks[key]`), stored in the tile.
    pub id: u64,
    /// `<mod-id>:<name>`.
    pub key: String,
    /// The mod that declares it.
    pub mod_id: String,
    /// Texture path inside the package.
    pub texture: String,
    /// The `Tile.typeIndex` of the vanilla block it is a variant of; `None` without a (known) `base`.
    pub base: Option<u8>,
    /// Display name per language.
    pub name: Localized,
}

impl BlockDef {
    /// The mod and the path of the texture inside its package.
    pub fn source(&self) -> (String, String) {
        (self.mod_id.clone(), self.texture.clone())
    }
}

/// The text to show in `lang` (`en`, `tr`, ...): that language, else English, else any, else `None`.
pub fn pick_name<'a>(name: &'a Localized, lang: &str) -> Option<&'a str> {
    name.get(lang).or_else(|| name.get("en")).or_else(|| name.values().next()).map(String::as_str)
}

/// The items of the loaded mods by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemTable {
    by_id: BTreeMap<u64, ItemDef>,
}

impl ItemTable {
    /// Joins the manifests' items with the `items` part of the server's registry; a declaration without an id is
    /// left out.
    pub fn build<'a>(registry: &Wire, mods: impl IntoIterator<Item = &'a Manifest>) -> ItemTable {
        let ids = registry.get(Kind::Items.name());
        let mut by_id = BTreeMap::new();
        for m in mods {
            for i in &m.content.items {
                let Some(id) = ids.and_then(|ids| ids.get(&i.key)).copied() else { continue };
                by_id.insert(
                    id,
                    ItemDef {
                        id,
                        key: i.key.clone(),
                        mod_id: m.id.clone(),
                        texture: i.texture.clone(),
                        name: i.name.clone(),
                        plural: i.plural.clone(),
                    },
                );
            }
        }
        ItemTable { by_id }
    }

    /// The item with this id.
    pub fn get(&self, id: u64) -> Option<&ItemDef> {
        self.by_id.get(&id)
    }

    /// All items, by id.
    pub fn all(&self) -> impl Iterator<Item = &ItemDef> {
        self.by_id.values()
    }

    /// True when no item is declared.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// What the game shows for item `id` in `lang`: the declared name in capitals like the game's own, else the
    /// key's name part (`decor:brass-lamp` shows `BRASS LAMP`). `None` for an id that is not a loaded mod's item.
    pub fn display_name(&self, id: u64, lang: &str) -> Option<String> {
        let def = self.by_id.get(&id)?;
        let text = pick_name(&def.name, lang).map_or_else(
            || def.key.split_once(':').map_or(def.key.as_str(), |(_, n)| n).replace(['-', '_'], " "),
            str::to_string,
        );
        Some(text.to_uppercase())
    }

    /// What the game shows where it asks for the plural of item `id` in `lang`: the declared `plural` in capitals;
    /// when the mod has none for `lang` but has a singular name for it, that singular (the mod's own words beat
    /// another language's plural); else the English plural, else the singular of [`ItemTable::display_name`].
    pub fn plural_name(&self, id: u64, lang: &str) -> Option<String> {
        let def = self.by_id.get(&id)?;
        let own =
            def.plural.get(lang).or_else(|| if def.name.contains_key(lang) { None } else { def.plural.get("en") });
        own.map(|t| t.to_uppercase()).or_else(|| self.display_name(id, lang))
    }
}

/// The variant blocks of the loaded mods by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockTable {
    by_id: BTreeMap<u64, BlockDef>,
}

impl BlockTable {
    /// Joins the manifests' blocks with the `blocks` part of the server's registry.
    pub fn build<'a>(registry: &Wire, mods: impl IntoIterator<Item = &'a Manifest>) -> BlockTable {
        let ids = registry.get(Kind::Blocks.name());
        let mut by_id = BTreeMap::new();
        for m in mods {
            for b in &m.content.blocks {
                let Some(id) = ids.and_then(|ids| ids.get(&b.key)).copied() else { continue };
                by_id.insert(
                    id,
                    BlockDef {
                        id,
                        key: b.key.clone(),
                        mod_id: m.id.clone(),
                        texture: b.texture.clone(),
                        base: b.base.as_deref().and_then(base_tile).map(|t| t.tile_type),
                        name: b.name.clone(),
                    },
                );
            }
        }
        BlockTable { by_id }
    }

    /// The block with this id.
    pub fn get(&self, id: u64) -> Option<&BlockDef> {
        self.by_id.get(&id)
    }

    /// All blocks, by id.
    pub fn all(&self) -> impl Iterator<Item = &BlockDef> {
        self.by_id.values()
    }

    /// True when no block is declared.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[cfg(test)]
mod tests;
