//! Fixed vocabulary of `[content]`: the cosmetic slots and the vanilla items a cosmetic may re-skin.
//!
//! The numbers are the game's `ItemType` values (1.7.x). They are facts about the protocol and the save format,
//! the same ones the Blockheads Server uses.

/// Where a cosmetic is worn or used. `hat`, `shirt`, `pants` and `shoes` re-skin the vanilla item worn in that
/// slot, `hair` re-skins the hair, `weapon` the weapon in hand (declared now, drawn by clients later).
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

const fn b(slot: &'static str, slug: &'static str, item_type: u16) -> BaseItem {
    BaseItem { slug, slot, item_type }
}

/// Every `base` value.
pub const BASE_ITEMS: &[BaseItem] = &[
    b("hat", "linen-cap", 115),
    b("hat", "linen-hat", 117),
    b("hat", "santa-hat", 142),
    b("hat", "tin-foil-hat", 154),
    b("hat", "fruit-hat", 140),
    b("hat", "tin-helmet", 189),
    b("hat", "iron-helmet", 193),
    b("hat", "titanium-helmet", 289),
    b("hat", "carbon-fiber-helmet", 293),
    b("hat", "ice-helmet", 197),
    b("shirt", "linen-shirt", 85),
    b("shirt", "leather-jacket", 122),
    b("shirt", "fur-coat", 126),
    b("shirt", "tin-chest-plate", 188),
    b("shirt", "iron-chest-plate", 192),
    b("shirt", "titanium-chest-plate", 288),
    b("shirt", "carbon-fiber-chest-plate", 292),
    b("shirt", "ice-chest-plate", 196),
    b("pants", "linen-pants", 84),
    b("pants", "leather-pants", 130),
    b("pants", "grass-skirt", 141),
    b("pants", "tin-pants", 187),
    b("pants", "iron-pants", 191),
    b("pants", "titanium-pants", 287),
    b("pants", "carbon-fiber-pants", 291),
    b("pants", "ice-pants", 195),
    b("shoes", "leather-boots", 124),
    b("shoes", "fur-boots", 127),
    b("shoes", "tin-boots", 190),
    b("shoes", "iron-boots", 194),
    b("shoes", "titanium-boots", 290),
    b("shoes", "carbon-fiber-boots", 294),
    b("shoes", "ice-boots", 198),
    b("shoes", "south-pole-boots", 143),
    b("weapon", "bronze-sword", 50),
    b("weapon", "iron-sword", 68),
    b("weapon", "titanium-sword", 286),
];

/// The base item with this `base` value.
pub fn base_item(slug: &str) -> Option<&'static BaseItem> {
    BASE_ITEMS.iter().find(|b| b.slug == slug)
}

/// The base item for a game `ItemType` worn or held in `slot`.
pub fn base_item_for(slot: &str, item_type: u16) -> Option<&'static BaseItem> {
    BASE_ITEMS.iter().find(|b| b.slot == slot && b.item_type == item_type)
}
