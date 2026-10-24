//! The client's content state, shared by the PC port and the Android loader: the packages' declarations joined
//! with the world's registry, who wears what, and the textures prepared for the game. The platform glue asks
//! [`Content::resolve`] what a blockhead should show and [`Content::texture`] for the pixels.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bhmod_manifest::package::{DirSource, Source, ZipSource, MAX_MANIFEST};
use bhmod_manifest::Manifest;
use bhmod_proto::Cosmetics;

use crate::cosmetics::{self, Resolved, Table, Wearing};
use crate::items::{BlockDef, BlockTable, ItemDef, ItemTable};
use crate::registry::{self, Registry, Wire};
use crate::texture::{Gpu, Texture};

/// Which texture of a cosmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// `texture`: the one texture, or a shirt's body.
    Main,
    /// `texture-arm`: a shirt's arms.
    Arm,
}

/// Opens a mod package: a `.bhmod` file or an unpacked folder (developer mods).
pub fn open_package(path: &Path) -> Result<Box<dyn Source>, String> {
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    if path.is_dir() {
        return Ok(Box::new(DirSource::open(path).map_err(fail)?));
    }
    let file = File::open(path).map_err(fail)?;
    let size = file.metadata().map_err(fail)?.len();
    Ok(Box::new(ZipSource::open(file, size).map_err(fail)?))
}

/// Reads and validates the `mod.toml` of a package (a `.bhmod` file or a folder).
pub fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let mut src = open_package(path)?;
    let text = src.read("mod.toml", MAX_MANIFEST).map_err(|e| format!("{}: mod.toml: {e}", path.display()))?;
    Manifest::parse(&String::from_utf8_lossy(&text)).map_err(|issues| {
        let why: Vec<String> = issues.iter().map(ToString::to_string).collect();
        format!("{}: {}", path.display(), why.join("; "))
    })
}

/// Ids for a client that has no server to give them (single-player developer tests): every key the packages
/// declare, allocated in memory the way a fresh server world would.
pub fn local_registry(packages: &[PathBuf]) -> Wire {
    let manifests: Vec<Manifest> = packages.iter().filter_map(|p| read_manifest(p).ok()).collect();
    let want = registry::declared(&manifests);
    let mut reg = Registry::new();
    reg.allocate(&want).expect("a few keys always fit");
    reg.wire(&want)
}

/// A clothing slot of a blockhead, in the order of the game's `clothingItemAtIndex:` (0 shoes, 1 pants, 2 shirt,
/// 3 hat).
pub const CLOTHING: [&str; 4] = ["shoes", "pants", "shirt", "hat"];

/// What one blockhead shows from cosmetics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    /// Per clothing slot (indexed like [`CLOTHING`]): the cosmetic id, and the vanilla item to wear if the slot is
    /// empty.
    pub clothing: [Option<(u64, Option<u16>)>; 4],
    /// The hair cosmetic.
    pub hair: Option<u64>,
}

impl Look {
    /// True when nothing is overridden.
    pub fn is_empty(&self) -> bool {
        self.clothing.iter().all(Option::is_none) && self.hair.is_none()
    }
}

/// The client's cosmetics: table, wearing, texture cache.
#[derive(Default)]
pub struct Content {
    table: Table,
    wearing: Wearing,
    /// Package of each mod id.
    packages: BTreeMap<String, PathBuf>,
    cache: BTreeMap<(u64, Part), Option<Arc<Gpu>>>,
    items: ItemTable,
    blocks: BlockTable,
    /// Icons of items (0) and textures of variant blocks (1), by id.
    extra_cache: BTreeMap<(u8, u64), Option<Arc<Gpu>>>,
    serial: u64,
}

impl Content {
    /// An empty state: no cosmetics declared, nobody wearing anything.
    pub fn new() -> Content {
        Content::default()
    }

    /// Reads the manifests of the packages the server listed and joins their cosmetics with `registry`
    /// (`MOD_LIST.registry`), replacing the table and the texture cache. Who wears what is kept: the server's
    /// `COSMETICS` may arrive before the packages are loaded. Problems (an unreadable package) are returned as
    /// text; that package adds nothing.
    pub fn load(&mut self, registry: &Wire, packages: &[PathBuf]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut manifests = Vec::new();
        let mut paths = BTreeMap::new();
        for path in packages {
            match read_manifest(path) {
                Ok(m) => {
                    paths.insert(m.id.clone(), path.clone());
                    manifests.push(m);
                }
                Err(why) => problems.push(why),
            }
        }
        let table = Table::build(registry, &manifests);
        let items = ItemTable::build(registry, &manifests);
        let blocks = BlockTable::build(registry, &manifests);
        // Keep only the packages that declare content the world has ids for.
        let used: BTreeSet<&str> = table
            .all()
            .map(|c| c.mod_id.as_str())
            .chain(items.all().map(|i| i.mod_id.as_str()))
            .chain(blocks.all().map(|b| b.mod_id.as_str()))
            .collect();
        paths.retain(|id, _| used.contains(id.as_str()));
        self.table = table;
        self.items = items;
        self.blocks = blocks;
        self.packages = paths;
        self.cache.clear();
        self.extra_cache.clear();
        self.serial += 1;
        problems
    }

    /// True when some loaded mod declares a cosmetic the world has an id for.
    pub fn is_active(&self) -> bool {
        !self.table.is_empty()
    }

    /// The declared cosmetics.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The declared items.
    pub fn items(&self) -> &ItemTable {
        &self.items
    }

    /// The declared variant blocks.
    pub fn blocks(&self) -> &BlockTable {
        &self.blocks
    }

    /// The name the game shows for item `id` in `lang`; see [`ItemTable::display_name`].
    pub fn item_name(&self, id: u64, lang: &str) -> Option<String> {
        self.items.display_name(id, lang)
    }

    /// The plural name the game shows for item `id` in `lang`; see [`ItemTable::plural_name`].
    pub fn item_plural(&self, id: u64, lang: &str) -> Option<String> {
        self.items.plural_name(id, lang)
    }

    /// Applies a `COSMETICS` message. Returns the players whose look changed.
    pub fn apply(&mut self, msg: &Cosmetics) -> Vec<String> {
        let changed = self.wearing.apply(msg);
        if !changed.is_empty() {
            self.serial += 1;
        }
        changed
    }

    /// Forgets who wears what (the server connection ended).
    pub fn clear(&mut self) {
        if !self.wearing.is_empty() {
            self.wearing.clear();
            self.serial += 1;
        }
    }

    /// Bumped whenever somebody's look changes; glue re-applies a blockhead when it differs from the last one seen.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// What `player` shows in `slot`; see [`cosmetics::resolve`].
    pub fn resolve(&self, player: &str, slot: &str, worn: Option<u16>) -> Option<Resolved<'_>> {
        cosmetics::resolve(&self.table, &self.wearing, player, slot, worn)
    }

    /// The weapon cosmetic `player` shows on a held sprite item of type `item` (`ItemType`): the id of the cosmetic
    /// whose `base` is that item. Drawn by the sprite renderer hooks of the clients.
    pub fn weapon(&self, player: &str, item: u16) -> Option<u64> {
        self.resolve(player, "weapon", Some(item)).map(|r| r.def.id)
    }

    /// The cosmetic `player` wears in `slot`, whatever item is worn or held (a dev aid; the clients use
    /// [`Content::weapon`] and [`Content::look`]).
    pub fn worn_in(&self, player: &str, slot: &str) -> Option<u64> {
        self.wearing.of(player)?.get(slot).copied()
    }

    /// True when some loaded mod declares a `weapon` cosmetic (the clients then hook the sprite renderer).
    pub fn has_weapons(&self) -> bool {
        self.table.all().any(|c| c.slot == "weapon")
    }

    /// Everything `player`'s blockhead shows from cosmetics, given the `ItemType` worn in each clothing slot
    /// ([`CLOTHING`] order; `None` for an empty slot).
    pub fn look(&self, player: &str, worn: [Option<u16>; 4]) -> Look {
        let mut look = Look::default();
        for (i, slot) in CLOTHING.iter().enumerate() {
            look.clothing[i] = self.resolve(player, slot, worn[i]).map(|r| (r.def.id, r.wear));
        }
        look.hair = self.resolve(player, "hair", None).map(|r| r.def.id);
        look
    }

    /// The pixels of a cosmetic's texture, prepared for the game and cached. `None` when the cosmetic has no such
    /// part or the file cannot be read (the problem is returned once, by the first call that hit it).
    pub fn texture(&mut self, id: u64, part: Part) -> Result<Option<Arc<Gpu>>, String> {
        if let Some(t) = self.cache.get(&(id, part)) {
            return Ok(t.clone());
        }
        let Some(def) = self.table.get(id) else { return Ok(None) };
        let path = match part {
            Part::Main => Some(def.texture.clone()),
            Part::Arm => def.texture_arm.clone(),
        };
        let Some(path) = path else {
            self.cache.insert((id, part), None);
            return Ok(None);
        };
        let loaded = self
            .packages
            .get(&def.mod_id)
            .ok_or_else(|| format!("{}: the package is not loaded", def.mod_id))
            .and_then(|p| open_package(p))
            .and_then(|mut src| Texture::read(src.as_mut(), &path));
        let gpu = loaded.as_ref().ok().map(|t| Arc::new(t.for_gpu()));
        self.cache.insert((id, part), gpu.clone());
        loaded.map(|_| gpu)
    }
}

impl Content {
    /// The icon of item `id`, prepared for the game and cached (`None` for an id that is no item of a loaded mod).
    pub fn item_texture(&mut self, id: u64) -> Result<Option<Arc<Gpu>>, String> {
        let source = self.items.get(id).map(ItemDef::source);
        self.cached_texture((0, id), source)
    }

    /// The icon of item `id` as `size` x `size` premultiplied RGBA, one cell of the game's item atlas (`None` for an
    /// id without an icon or a cell).
    pub fn item_cell_pixels(&mut self, id: u64, size: u32) -> Result<Option<Vec<u8>>, String> {
        let source = self.items.get(id).map(ItemDef::source);
        self.cell_pixels(source, size)
    }

    /// The texture of variant block `id` as `size` x `size` premultiplied RGBA, one cell of the game's tile atlas.
    pub fn block_cell_pixels(&mut self, id: u64, size: u32) -> Result<Option<Vec<u8>>, String> {
        let source = self.blocks.get(id).map(BlockDef::source);
        self.cell_pixels(source, size)
    }

    fn cell_pixels(&self, source: Option<(String, String)>, size: u32) -> Result<Option<Vec<u8>>, String> {
        let Some((mod_id, path)) = source else { return Ok(None) };
        let pkg = self.packages.get(&mod_id).ok_or_else(|| format!("{mod_id}: the package is not loaded"))?;
        let mut src = open_package(pkg)?;
        Ok(Some(Texture::read(src.as_mut(), &path)?.cell(size)))
    }

    /// The texture of variant block `id`, prepared for the game and cached.
    pub fn block_texture(&mut self, id: u64) -> Result<Option<Arc<Gpu>>, String> {
        let source = self.blocks.get(id).map(BlockDef::source);
        self.cached_texture((1, id), source)
    }

    /// `key` is (0 item | 1 block, id); `source` is (mod id, path in its package).
    fn cached_texture(&mut self, key: (u8, u64), source: Option<(String, String)>) -> Result<Option<Arc<Gpu>>, String> {
        if let Some(t) = self.extra_cache.get(&key) {
            return Ok(t.clone());
        }
        let Some((mod_id, path)) = source else { return Ok(None) };
        let loaded = self
            .packages
            .get(&mod_id)
            .ok_or_else(|| format!("{mod_id}: the package is not loaded"))
            .and_then(|p| open_package(p))
            .and_then(|mut src| Texture::read(src.as_mut(), &path));
        let gpu = loaded.as_ref().ok().map(|t| Arc::new(t.for_gpu()));
        self.extra_cache.insert(key, gpu.clone());
        loaded.map(|_| gpu)
    }
}

#[cfg(test)]
mod tests;
