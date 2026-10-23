//! The per-world content registry: string keys to numeric ids.
//!
//! Mods declare content by key (`<mod-id>:<name>`). A world gives every key a number the first time it sees it,
//! from the free range of its kind, and never changes or reuses it afterwards, not even when the mod is removed:
//! the sidecar keeps the key so the same id comes back with the mod. Allocation sorts the new keys of a kind, so it
//! does not depend on the order mods were loaded in.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::Path;

use bhmod_manifest::Manifest;

/// The sidecar format version.
pub const SCHEMA: u32 = 1;

/// A kind of content. The registry has one id space per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Variant blocks. The id is a `u16` stored with the tile (M8 part 2).
    Blocks,
    /// Items: real `ItemType`s above the vanilla ones (M8 part 3).
    Items,
    /// Cosmetics: appearance overrides. Their ids never become item types.
    Cosmetics,
}

impl Kind {
    /// All kinds in registry order.
    pub const ALL: [Kind; 3] = [Kind::Blocks, Kind::Items, Kind::Cosmetics];

    /// The registry key of the kind (`blocks`, `items`, `cosmetics`), as in `MOD_LIST.registry` and the sidecar.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Blocks => "blocks",
            Kind::Items => "items",
            Kind::Cosmetics => "cosmetics",
        }
    }

    /// The kind with this registry key.
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// First id handed out. Items start above the highest vanilla `ItemType` (1106 in 1.7.x); ids below are the
    /// game's. `0` is never an id.
    pub fn first_id(self) -> u64 {
        match self {
            Kind::Items => 1107,
            Kind::Blocks | Kind::Cosmetics => 1,
        }
    }

    /// Last id (all ids fit a `u16`: item types and variant ids are 16 bits wide).
    pub fn last_id(self) -> u64 {
        u64::from(u16::MAX)
    }
}

/// What can go wrong with the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The sidecar is not valid TOML or does not follow the format.
    Format(String),
    /// A kind has no free id left.
    Exhausted(Kind),
    /// Reading or writing the sidecar failed.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Format(why) => write!(f, "registry.toml: {why}"),
            Error::Exhausted(k) => write!(f, "no free {} id left", k.name()),
            Error::Io(why) => write!(f, "registry.toml: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// A content key: `<mod-id>:<name>`.
pub fn is_key(key: &str) -> bool {
    let ok = |s: &str, max: usize, min: usize| {
        (min..=max).contains(&s.len())
            && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    key.split_once(':').is_some_and(|(m, n)| ok(m, 40, 2) && ok(n, 40, 1))
}

/// Every key the manifests declare, per kind.
pub fn declared<'a>(manifests: impl IntoIterator<Item = &'a Manifest>) -> BTreeMap<Kind, BTreeSet<String>> {
    let mut out: BTreeMap<Kind, BTreeSet<String>> = BTreeMap::new();
    for m in manifests {
        let c = &m.content;
        out.entry(Kind::Blocks).or_default().extend(c.blocks.iter().map(|b| b.key.clone()));
        out.entry(Kind::Items).or_default().extend(c.items.iter().map(|i| i.key.clone()));
        out.entry(Kind::Cosmetics).or_default().extend(c.cosmetics.iter().map(|k| k.key.clone()));
    }
    out.retain(|_, keys| !keys.is_empty());
    out
}

/// The `MOD_LIST.registry` map: kind name to key to id.
pub type Wire = BTreeMap<String, BTreeMap<String, u64>>;

/// Keys and ids of a world.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    ids: BTreeMap<Kind, BTreeMap<String, u64>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Parses the sidecar text.
    pub fn parse(text: &str) -> Result<Registry, Error> {
        let doc: toml::Table = toml::from_str(text).map_err(|e| Error::Format(e.message().to_string()))?;
        let mut reg = Registry::new();
        for (name, value) in doc {
            if name == "schema" {
                if value.as_integer() != Some(i64::from(SCHEMA)) {
                    return Err(Error::Format(format!("unsupported schema {value}")));
                }
                continue;
            }
            let kind = Kind::from_name(&name).ok_or_else(|| Error::Format(format!("unknown kind `{name}`")))?;
            let toml::Value::Table(entries) = value else {
                return Err(Error::Format(format!("`{name}` must be a table")));
            };
            let mut used = BTreeSet::new();
            for (key, id) in entries {
                let id = id
                    .as_integer()
                    .and_then(|n| u64::try_from(n).ok())
                    .filter(|n| (kind.first_id()..=kind.last_id()).contains(n))
                    .ok_or_else(|| Error::Format(format!("id of `{key}` is not in the {name} range")))?;
                if !is_key(&key) {
                    return Err(Error::Format(format!("`{key}` is not a content key")));
                }
                if !used.insert(id) {
                    return Err(Error::Format(format!("{name} id {id} is used twice")));
                }
                reg.ids.entry(kind).or_default().insert(key, id);
            }
        }
        Ok(reg)
    }

    /// The sidecar text (`schema = 1`, then one table per non-empty kind, keys sorted).
    pub fn to_toml(&self) -> String {
        let mut out = format!(
            "# Content ids of this world. Written by the server; do not edit or delete keys.\nschema = {SCHEMA}\n"
        );
        for (kind, keys) in self.ids.iter().filter(|(_, k)| !k.is_empty()) {
            out.push_str(&format!("\n[{}]\n", kind.name()));
            for (key, id) in keys {
                out.push_str(&format!("\"{key}\" = {id}\n"));
            }
        }
        out
    }

    /// The id of a key.
    pub fn id(&self, kind: Kind, key: &str) -> Option<u64> {
        self.ids.get(&kind)?.get(key).copied()
    }

    /// The key of an id.
    pub fn key(&self, kind: Kind, id: u64) -> Option<&str> {
        self.ids.get(&kind)?.iter().find(|(_, i)| **i == id).map(|(k, _)| k.as_str())
    }

    /// Number of keys, retired ones included.
    pub fn len(&self) -> usize {
        self.ids.values().map(BTreeMap::len).sum()
    }

    /// True when no key has an id.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Gives every key without an id the lowest free id of its kind. Returns true when something was added.
    /// Nothing changes when a kind runs out of ids.
    pub fn allocate(&mut self, wanted: &BTreeMap<Kind, BTreeSet<String>>) -> Result<bool, Error> {
        let mut next = self.clone();
        let mut changed = false;
        for (kind, keys) in wanted {
            let map = next.ids.entry(*kind).or_default();
            let mut used: BTreeSet<u64> = map.values().copied().collect();
            let mut candidate = kind.first_id();
            let missing: Vec<&String> = keys.iter().filter(|k| !map.contains_key(*k)).collect();
            for key in missing {
                while used.contains(&candidate) {
                    candidate += 1;
                }
                if candidate > kind.last_id() {
                    return Err(Error::Exhausted(*kind));
                }
                map.insert(key.clone(), candidate);
                used.insert(candidate);
                changed = true;
            }
        }
        *self = next;
        Ok(changed)
    }

    /// The `MOD_LIST.registry` map for the keys that are in `wanted` (the loaded mods' content): retired keys stay
    /// in the sidecar but are not sent. Kinds without keys are left out.
    pub fn wire(&self, wanted: &BTreeMap<Kind, BTreeSet<String>>) -> Wire {
        let mut out = Wire::new();
        for (kind, keys) in wanted {
            let sent: BTreeMap<String, u64> =
                keys.iter().filter_map(|k| self.id(*kind, k).map(|id| (k.clone(), id))).collect();
            if !sent.is_empty() {
                out.insert(kind.name().to_string(), sent);
            }
        }
        out
    }

    /// Reads the sidecar. A missing file is an empty registry.
    pub fn read(path: &Path) -> Result<Registry, Error> {
        match std::fs::read_to_string(path) {
            Ok(text) => Registry::parse(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Registry::new()),
            Err(e) => Err(Error::Io(e.to_string())),
        }
    }

    /// Writes the sidecar (to a temporary file next to it, then renamed over it).
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let io = |e: io::Error| Error::Io(e.to_string());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }
}

#[cfg(test)]
mod tests;
