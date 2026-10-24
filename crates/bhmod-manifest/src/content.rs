//! Fixed vocabulary of `[content]`: the cosmetic slots and the vanilla items a cosmetic may re-skin.
//!
//! The numbers are the game's `ItemType` values (1.7.x). They are facts about the protocol and the save format,
//! the same ones the Blockheads Server uses.

/// Where a cosmetic is worn or used. `hat`, `shirt`, `pants` and `shoes` re-skin the vanilla item worn in that
/// slot, `hair` re-skins the hair, `weapon` the weapon in hand (declared, not drawn by clients).
pub const SLOTS: [&str; 6] = ["hat", "shirt", "pants", "shoes", "hair", "weapon"];

/// A vanilla item a cosmetic can be restricted to (`base` in `[[content.cosmetics]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseItem {
    /// The `base` value.
    pub slug: &'static str,
    /// The slot it belongs to.
    pub slot: &'static str,
    /// The game's `ItemType`.
    pub item_type: u16,
}

macro_rules! b {
    ($slot:literal, $slug:literal, $item_type:literal) => {
        BaseItem { slug: $slug, slot: $slot, item_type: $item_type }
    };
}

/// Every `base` value.
pub const BASE_ITEMS: &[BaseItem] = &[
    b!("hat", "linen-cap", 115),
    b!("hat", "linen-hat", 117),
    b!("hat", "santa-hat", 142),
    b!("hat", "tin-foil-hat", 154),
    b!("hat", "fruit-hat", 140),
    b!("hat", "tin-helmet", 189),
    b!("hat", "iron-helmet", 193),
    b!("hat", "titanium-helmet", 289),
    b!("hat", "carbon-fiber-helmet", 293),
    b!("hat", "ice-helmet", 197),
    b!("shirt", "linen-shirt", 85),
    b!("shirt", "leather-jacket", 122),
    b!("shirt", "fur-coat", 126),
    b!("shirt", "tin-chest-plate", 188),
    b!("shirt", "iron-chest-plate", 192),
    b!("shirt", "titanium-chest-plate", 288),
    b!("shirt", "carbon-fiber-chest-plate", 292),
    b!("shirt", "ice-chest-plate", 196),
    b!("pants", "linen-pants", 84),
    b!("pants", "leather-pants", 130),
    b!("pants", "grass-skirt", 141),
    b!("pants", "tin-pants", 187),
    b!("pants", "iron-pants", 191),
    b!("pants", "titanium-pants", 287),
    b!("pants", "carbon-fiber-pants", 291),
    b!("pants", "ice-pants", 195),
    b!("shoes", "leather-boots", 124),
    b!("shoes", "fur-boots", 127),
    b!("shoes", "tin-boots", 190),
    b!("shoes", "iron-boots", 194),
    b!("shoes", "titanium-boots", 290),
    b!("shoes", "carbon-fiber-boots", 294),
    b!("shoes", "ice-boots", 198),
    b!("shoes", "south-pole-boots", 143),
    // Held tools and weapons: every item the game lets a blockhead hold in hand to work or fight with.
    b!("weapon", "flint-axe", 6),
    b!("weapon", "flint-spear", 7),
    b!("weapon", "flint-pickaxe", 8),
    b!("weapon", "flint-spade", 16),
    b!("weapon", "flint-machete", 25),
    b!("weapon", "stone-axe", 33),
    b!("weapon", "stone-pickaxe", 34),
    b!("weapon", "copper-spear", 39),
    b!("weapon", "tin-spade", 40),
    b!("weapon", "copper-bow", 42),
    b!("weapon", "bronze-pickaxe", 43),
    b!("weapon", "bronze-machete", 49),
    b!("weapon", "bronze-sword", 50),
    b!("weapon", "stone-spade", 64),
    b!("weapon", "iron-pickaxe", 66),
    b!("weapon", "iron-machete", 67),
    b!("weapon", "iron-sword", 68),
    b!("weapon", "iron-axe", 70),
    b!("weapon", "gold-spade", 89),
    b!("weapon", "gold-pickaxe", 90),
    b!("weapon", "steel-pickaxe", 93),
    b!("weapon", "amethyst-pickaxe", 94),
    b!("weapon", "sapphire-pickaxe", 95),
    b!("weapon", "emerald-pickaxe", 96),
    b!("weapon", "ruby-pickaxe", 97),
    b!("weapon", "diamond-pickaxe", 98),
    b!("weapon", "crowbar", 209),
    b!("weapon", "fishing-rod", 156),
    b!("weapon", "poison-bow", 257),
    b!("weapon", "titanium-pickaxe", 285),
    b!("weapon", "titanium-sword", 286),
];

/// The base item with this `base` value.
pub fn base_item(slug: &str) -> Option<&'static BaseItem> {
    BASE_ITEMS.iter().find(|b| b.slug == slug)
}

/// The base item for a game `ItemType` worn or held in `slot`.
pub fn base_item_for(slot: &str, item_type: u16) -> Option<&'static BaseItem> {
    BASE_ITEMS.iter().find(|b| b.slot == slot && b.item_type == item_type)
}

/// A vanilla block a variant block can be based on (`base` in `[[content.blocks]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseTile {
    /// The `base` value.
    pub slug: &'static str,
    /// The game's `TileType` (`Tile.typeIndex`).
    pub tile_type: u8,
    /// The `ItemType` of the block item mining the base block gives (what a variant without `drops` gives).
    pub item_type: u16,
}

macro_rules! t {
    ($slug:literal, $tile:literal, $item:literal) => {
        BaseTile { slug: $slug, tile_type: $tile, item_type: $item }
    };
}

/// Every `base` value of a block: whole solid building blocks without special behaviour.
pub const BASE_TILES: &[BaseTile] = &[
    t!("wood", 9, 0x419),
    t!("cobblestone", 10, 0x400),
    t!("red-bricks", 11, 0x402),
    t!("limestone-block", 13, 0x404),
    t!("marble-block", 15, 0x406),
    t!("sandstone-block", 18, 0x40C),
    t!("red-marble-block", 20, 0x40E),
    t!("glass", 24, 0x412),
    t!("gold-block", 26, 0x415),
    t!("lapis-block", 30, 0x41E),
    t!("basalt-block", 52, 0x427),
    t!("copper-block", 53, 0x42A),
    t!("tin-block", 54, 0x42B),
    t!("bronze-block", 55, 0x42C),
    t!("iron-block", 56, 0x42D),
    t!("steel-block", 57, 0x42E),
    t!("black-glass", 59, 0x434),
    t!("plaster", 76, 0x44F),
];

/// The vanilla block with this `base` value.
pub fn base_tile(slug: &str) -> Option<&'static BaseTile> {
    BASE_TILES.iter().find(|b| b.slug == slug)
}

/// The vanilla block of a `Tile.typeIndex`, if it can be a variant's base.
pub fn base_tile_of(tile_type: u8) -> Option<&'static BaseTile> {
    BASE_TILES.iter().find(|b| b.tile_type == tile_type)
}
