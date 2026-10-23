//! Cosmetics on the client: which declared cosmetic has which id (the world's registry), who wears what (the
//! server's `COSMETICS` messages), and which texture a blockhead shows in a slot.
//!
//! A cosmetic re-skins the vanilla item worn (or held) in its slot, or the hair. It never adds an item to the
//! inventory and never reaches vanilla players, who keep seeing the plain vanilla look.

use std::collections::BTreeMap;

use bhmod_manifest::content::base_item;
use bhmod_manifest::Manifest;
use bhmod_proto::Cosmetics;

use crate::registry::Kind;

/// A cosmetic a loaded mod declares and the world has an id for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CosmeticDef {
    /// The world's id (`registry.cosmetics[key]`).
    pub id: u64,
    /// `<mod-id>:<name>`.
    pub key: String,
    /// The mod that declares it.
    pub mod_id: String,
    /// One of [`bhmod_manifest::content::SLOTS`].
    pub slot: String,
    /// Texture path inside the package (`assets/...png`).
    pub texture: String,
    /// Arm texture of a shirt.
    pub texture_arm: Option<String>,
    /// The game `ItemType` this cosmetic re-skins, if it is restricted to one vanilla item.
    pub base: Option<u16>,
}

/// The cosmetics of the loaded mods by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    by_id: BTreeMap<u64, CosmeticDef>,
}

impl Table {
    /// Joins the manifests' declarations with the `cosmetics` part of the server's registry. A declaration the
    /// registry has no id for is left out.
    pub fn build<'a>(
        registry: &BTreeMap<String, BTreeMap<String, u64>>,
        mods: impl IntoIterator<Item = &'a Manifest>,
    ) -> Table {
        let ids = registry.get(Kind::Cosmetics.name());
        let mut by_id = BTreeMap::new();
        for m in mods {
            for c in &m.content.cosmetics {
                let Some(id) = ids.and_then(|ids| ids.get(&c.key)).copied() else { continue };
                by_id.insert(
                    id,
                    CosmeticDef {
                        id,
                        key: c.key.clone(),
                        mod_id: m.id.clone(),
                        slot: c.slot.clone(),
                        texture: c.texture.clone(),
                        texture_arm: c.texture_arm.clone(),
                        base: c.base.as_deref().and_then(base_item).map(|b| b.item_type),
                    },
                );
            }
        }
        Table { by_id }
    }

    /// The cosmetic with this id.
    pub fn get(&self, id: u64) -> Option<&CosmeticDef> {
        self.by_id.get(&id)
    }

    /// All cosmetics, by id.
    pub fn all(&self) -> impl Iterator<Item = &CosmeticDef> {
        self.by_id.values()
    }

    /// True when no cosmetic is declared.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Who wears which cosmetic, as the server last said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wearing {
    players: BTreeMap<String, BTreeMap<String, u64>>,
}

impl Wearing {
    /// Applies a `COSMETICS` message. Returns the players whose set changed (the blockheads to refresh).
    pub fn apply(&mut self, msg: &Cosmetics) -> Vec<String> {
        let mut changed = Vec::new();
        if msg.full {
            let gone: Vec<String> = self.players.keys().filter(|p| !msg.players.contains_key(*p)).cloned().collect();
            for p in gone {
                self.players.remove(&p);
                changed.push(p);
            }
        }
        for (player, slots) in &msg.players {
            let before = self.players.get(player);
            if slots.is_empty() {
                if self.players.remove(player).is_some() {
                    changed.push(player.clone());
                }
            } else if before != Some(slots) {
                self.players.insert(player.clone(), slots.clone());
                changed.push(player.clone());
            }
        }
        changed.sort();
        changed.dedup();
        changed
    }

    /// Forgets everything (the player left the server).
    pub fn clear(&mut self) {
        self.players.clear();
    }

    /// What `player` wears in each slot.
    pub fn of(&self, player: &str) -> Option<&BTreeMap<String, u64>> {
        self.players.get(player)
    }

    /// True when nobody wears anything.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }
}

/// The vanilla item a blockhead wears for a cosmetic that has no `base` while the slot is empty: plain items with
/// no armour value.
pub const DEFAULT_WEAR: [(&str, u16); 4] = [("hat", 115), ("shirt", 85), ("pants", 84), ("shoes", 124)];

/// What a blockhead shows in a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved<'a> {
    /// The cosmetic to draw.
    pub def: &'a CosmeticDef,
    /// `Some(item type)`: the slot is empty, so the blockhead wears this vanilla item (the cosmetic's base, or the
    /// slot's [`DEFAULT_WEAR`]) just to have a shape to draw it on. `None`: the item worn is used.
    pub wear: Option<u16>,
}

/// The cosmetic `player` shows in `slot`, given the vanilla item worn there (`worn`, an `ItemType`, or `None` when
/// the slot is empty).
///
/// - `hair` ignores `worn`; `weapon` shows only on its base weapon.
/// - A cosmetic restricted to a base item shows while that item is worn, and on an empty slot (the blockhead then
///   wears the base). With another item worn it stays hidden.
/// - A cosmetic without a base shows on any worn vanilla item, and on an empty slot (the blockhead then wears the
///   slot's [`DEFAULT_WEAR`]).
pub fn resolve<'a>(
    table: &'a Table,
    wearing: &Wearing,
    player: &str,
    slot: &str,
    worn: Option<u16>,
) -> Option<Resolved<'a>> {
    let def = table.get(*wearing.of(player)?.get(slot)?)?;
    if def.slot != slot {
        return None;
    }
    let default = DEFAULT_WEAR.iter().find(|(s, _)| *s == slot).map(|(_, t)| *t);
    match (slot, def.base, worn) {
        ("hair", _, _) => Some(Resolved { def, wear: None }),
        ("weapon", Some(base), Some(item)) => (base == item).then_some(Resolved { def, wear: None }),
        ("weapon", ..) => None,
        (_, base, None) => Some(Resolved { def, wear: Some(base.or(default)?) }),
        (_, Some(base), Some(item)) => (base == item).then_some(Resolved { def, wear: None }),
        (_, None, Some(_)) => Some(Resolved { def, wear: None }),
    }
}

#[cfg(test)]
mod tests;
