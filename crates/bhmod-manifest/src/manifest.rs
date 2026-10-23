//! The `mod.toml` data model and its semantic validation.

use std::collections::{BTreeMap, BTreeSet};

use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::rules::{check_path, is_host, is_lang, is_mod_id, is_slug};
use crate::{Issue, SCHEMA_VERSION, SUPPORTED_API_VERSIONS};

/// A localized text: language code -> text. `en` is required.
pub type Localized = BTreeMap<String, String>;

/// A parsed and validated `mod.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Manifest {
    /// Manifest format version; must be 1.
    pub schema: u32,
    /// Mod id, `[a-z0-9_-]{2,40}`.
    pub id: String,
    /// Mod version (semver).
    pub version: String,
    /// Display name per language.
    pub name: Localized,
    /// Description per language.
    #[serde(default)]
    pub description: Localized,
    /// Author names or handles.
    pub authors: Vec<String>,
    /// The mod's licence (SPDX expression recommended). Mod authors choose freely.
    pub license: String,
    /// Project page.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Supported `blockheads:api` versions (semver requirement).
    pub api: String,
    /// Supported mod loader / host versions (semver requirement).
    #[serde(default)]
    pub loader: Option<String>,
    /// Where the mod runs.
    pub sides: Sides,
    /// Entry points per side.
    #[serde(default)]
    pub entry: Entries,
    /// Required mods.
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
    /// Optional mods that only influence load order.
    #[serde(default)]
    pub soft_dependencies: BTreeMap<String, Dependency>,
    /// Permissions the admin or player must grant.
    #[serde(default)]
    pub permissions: Permissions,
    /// Services this mod provides.
    #[serde(default)]
    pub provides: Provides,
    /// New blocks, items and cosmetics.
    #[serde(default)]
    pub content: Content,
}

/// `[sides]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Sides {
    /// Has a server half.
    #[serde(default)]
    pub server: bool,
    /// Has a client half.
    #[serde(default)]
    pub client: bool,
    /// Whether players must run the client half to join. Defaults to `client`.
    #[serde(default)]
    pub client_required: Option<bool>,
}

/// `[entry]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entries {
    /// Server entry point.
    pub server: Option<Entry>,
    /// Client entry point.
    pub client: Option<Entry>,
}

/// One entry point.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Which runtime loads it.
    pub runtime: Runtime,
    /// Path inside the package.
    pub path: String,
}

/// A mod runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Runtime {
    /// A WASM component (`.wasm`).
    Wasm,
    /// Luau source (`.luau`).
    Lua,
    /// A JVM jar (`.jar`); unsandboxed, needs `native`.
    Jvm,
}

impl Runtime {
    /// The file extension the entry path must have.
    pub fn extension(self) -> &'static str {
        match self {
            Runtime::Wasm => ".wasm",
            Runtime::Lua => ".luau",
            Runtime::Jvm => ".jar",
        }
    }
}

/// A dependency: either a bare version requirement or a table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    /// `economy = "^1.1"`.
    Req(String),
    /// `economy = { version = "^1.1", side = "server" }`.
    Detailed {
        /// Version requirement.
        version: String,
        /// Side on which the dependency is needed.
        #[serde(default)]
        side: DepSide,
    },
}

impl Dependency {
    /// The version requirement text.
    pub fn version(&self) -> &str {
        match self {
            Dependency::Req(v) | Dependency::Detailed { version: v, .. } => v,
        }
    }
}

/// The side a dependency is needed on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DepSide {
    /// Server only.
    Server,
    /// Client only.
    Client,
    /// Both sides (default).
    #[default]
    Both,
}

/// `[permissions]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Permissions {
    /// World access.
    #[serde(default)]
    pub world: WorldAccess,
    /// Player permissions.
    #[serde(default)]
    pub players: Vec<PlayerPermission>,
    /// Outbound HTTP host allowlist (server side only).
    #[serde(default)]
    pub net_http: Vec<String>,
    /// May open channels.
    #[serde(default)]
    pub channels: bool,
    /// Native hooks or unsandboxed code (JVM, `.so`). Shown with a red warning.
    #[serde(default)]
    pub native: bool,
}

/// World access level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldAccess {
    /// No world access.
    #[default]
    None,
    /// Read tiles and objects.
    Read,
    /// Change tiles and objects.
    Write,
}

/// Player permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlayerPermission {
    /// Broadcast chat.
    Chat,
    /// Move players.
    Teleport,
    /// Kick players.
    Kick,
    /// Ban players.
    Ban,
    /// Read and change inventories.
    Inventory,
    /// Change health and food.
    Health,
}

/// `[provides]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provides {
    /// Services.
    #[serde(default)]
    pub services: Vec<ServiceDecl>,
}

/// One provided service.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDecl {
    /// Service name, `[a-z0-9_-]{2,40}`.
    pub name: String,
    /// Service version (semver).
    pub version: String,
    /// WIT file under `wit/`.
    pub wit: String,
}

/// `[content]`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    /// New blocks.
    #[serde(default)]
    pub blocks: Vec<BlockDecl>,
    /// New items.
    #[serde(default)]
    pub items: Vec<ItemDecl>,
    /// New cosmetics.
    #[serde(default)]
    pub cosmetics: Vec<CosmeticDecl>,
}

impl Content {
    /// True if no content is declared.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() && self.items.is_empty() && self.cosmetics.is_empty()
    }

    /// The key of every declaration.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        let blocks = self.blocks.iter().map(|b| b.key.as_str());
        let items = self.items.iter().map(|i| i.key.as_str());
        blocks.chain(items).chain(self.cosmetics.iter().map(|c| c.key.as_str()))
    }

    /// `(key, texture)` of every declaration (a cosmetic's `texture-arm` counts too).
    pub fn textures(&self) -> Vec<(&str, &str)> {
        let blocks = self.blocks.iter().map(|b| (b.key.as_str(), b.texture.as_str()));
        let items = self.items.iter().map(|i| (i.key.as_str(), i.texture.as_str()));
        let cosmetics = self.cosmetics.iter().flat_map(|c| {
            std::iter::once((c.key.as_str(), c.texture.as_str()))
                .chain(c.texture_arm.as_deref().map(|a| (c.key.as_str(), a)))
        });
        blocks.chain(items).chain(cosmetics).collect()
    }
}

/// A new block.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDecl {
    /// `<mod-id>:<name>`.
    pub key: String,
    /// PNG under `assets/`.
    pub texture: String,
    /// Mining hardness (>= 0).
    #[serde(default)]
    pub hardness: Option<f64>,
}

/// A new item.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDecl {
    /// `<mod-id>:<name>`.
    pub key: String,
    /// PNG under `assets/`.
    pub texture: String,
    /// Display name per language.
    #[serde(default)]
    pub name: Localized,
}

/// A new cosmetic.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct CosmeticDecl {
    /// `<mod-id>:<name>`.
    pub key: String,
    /// Where it is worn: one of [`crate::content::SLOTS`].
    pub slot: String,
    /// PNG under `assets/`. For `shirt` the body texture, for the other slots the one texture, laid out like the
    /// vanilla texture of the item it re-skins.
    pub texture: String,
    /// `shirt` only: PNG for the arms (the body is `texture`). Without it the vanilla arms stay.
    #[serde(default)]
    pub texture_arm: Option<String>,
    /// The vanilla item (`linen-cap`, `iron-sword`, ... see [`crate::content::BASE_ITEMS`]) this cosmetic re-skins:
    /// it shows only while that item is worn or held. Without it, it shows on every vanilla item of the slot (and
    /// always for `hair`, which has no base).
    #[serde(default)]
    pub base: Option<String>,
    /// Display name per language.
    #[serde(default)]
    pub name: Localized,
}

impl Manifest {
    /// Parses and validates `mod.toml` text. Returns every problem found.
    pub fn parse(text: &str) -> Result<Manifest, Vec<Issue>> {
        let m: Manifest = toml::from_str(text).map_err(|e| vec![Issue::new("mod.toml", e.message())])?;
        let issues = m.validate();
        if issues.is_empty() {
            Ok(m)
        } else {
            Err(issues)
        }
    }

    /// The parsed mod version. Only call on a validated manifest.
    pub fn semver(&self) -> Version {
        Version::parse(&self.version).expect("validated manifest")
    }

    /// Whether players must run the client half: `[content]` forces true, otherwise the explicit value,
    /// otherwise `sides.client`.
    pub fn client_required(&self) -> bool {
        !self.content.is_empty() || self.sides.client_required.unwrap_or(self.sides.client)
    }

    /// Semantic checks beyond the TOML shape.
    pub fn validate(&self) -> Vec<Issue> {
        let mut v = Vec::new();
        if self.schema != SCHEMA_VERSION {
            v.push(Issue::new("schema", format!("unsupported schema {}, expected {SCHEMA_VERSION}", self.schema)));
        }
        if !is_mod_id(&self.id) {
            v.push(Issue::new("id", "must match [a-z0-9_-]{2,40}"));
        }
        if Version::parse(&self.version).is_err() {
            v.push(Issue::new("version", "not a semver version (e.g. 1.0.0)"));
        }
        check_localized(&mut v, "name", &self.name, true, 64);
        check_localized(&mut v, "description", &self.description, false, 1024);
        if self.authors.is_empty() || self.authors.iter().any(|a| a.trim().is_empty()) {
            v.push(Issue::new("authors", "list at least one non-empty author"));
        }
        if self.license.trim().is_empty() || self.license.len() > 128 {
            v.push(Issue::new("license", "must be a non-empty licence name or SPDX expression (max 128 bytes)"));
        }
        if let Some(h) = &self.homepage {
            if !(h.starts_with("https://") || h.starts_with("http://")) {
                v.push(Issue::new("homepage", "must be an http(s) URL"));
            }
        }
        match VersionReq::parse(&self.api) {
            Err(_) => v.push(Issue::new("api", "not a semver requirement (e.g. ^0.1)")),
            Ok(req) if !crate::api_supported(&req) => v.push(Issue::new(
                "api",
                format!("matches none of the API versions this SDK supports ({})", SUPPORTED_API_VERSIONS.join(", ")),
            )),
            Ok(_) => {}
        }
        if self.loader.as_deref().is_some_and(|l| VersionReq::parse(l).is_err()) {
            v.push(Issue::new("loader", "not a semver requirement"));
        }
        self.validate_sides(&mut v);
        self.validate_deps(&mut v);
        self.validate_permissions(&mut v);
        self.validate_provides(&mut v);
        self.validate_content(&mut v);
        v
    }

    fn validate_sides(&self, v: &mut Vec<Issue>) {
        if !self.sides.server && !self.sides.client {
            v.push(Issue::new("sides", "enable at least one of server, client"));
        }
        for (side, enabled, entry) in
            [("server", self.sides.server, &self.entry.server), ("client", self.sides.client, &self.entry.client)]
        {
            let key = format!("entry.{side}");
            match (enabled, entry) {
                (true, None) => v.push(Issue::new(&key, format!("missing entry point for side {side}"))),
                (false, Some(_)) => v.push(Issue::new(&key, format!("entry point given but sides.{side} is false"))),
                (_, Some(e)) => {
                    if let Err(msg) = check_path(&e.path) {
                        v.push(Issue::new(&key, msg));
                    } else if !e.path.starts_with(&format!("{side}/")) || e.path.starts_with(&format!("{side}/native/"))
                    {
                        v.push(Issue::new(&key, format!("path must be inside {side}/ (and not native/)")));
                    }
                    if !e.path.ends_with(e.runtime.extension()) {
                        v.push(Issue::new(&key, format!("runtime needs a {} file", e.runtime.extension())));
                    }
                    if e.runtime == Runtime::Jvm && !self.permissions.native {
                        v.push(Issue::new(&key, "the jvm runtime is unsandboxed and needs permissions.native = true"));
                    }
                }
                (false, None) => {}
            }
        }
    }

    fn validate_deps(&self, v: &mut Vec<Issue>) {
        for (table, deps) in [("dependencies", &self.dependencies), ("soft-dependencies", &self.soft_dependencies)] {
            for (id, dep) in deps {
                let key = format!("{table}.{id}");
                if !is_mod_id(id) {
                    v.push(Issue::new(&key, "not a valid mod id"));
                } else if id == &self.id {
                    v.push(Issue::new(&key, "a mod cannot depend on itself"));
                }
                if VersionReq::parse(dep.version()).is_err() {
                    v.push(Issue::new(&key, "not a semver requirement"));
                }
            }
        }
        for id in self.dependencies.keys().filter(|id| self.soft_dependencies.contains_key(*id)) {
            v.push(Issue::new(format!("soft-dependencies.{id}"), "also listed in dependencies"));
        }
    }

    fn validate_permissions(&self, v: &mut Vec<Issue>) {
        let p = &self.permissions;
        for host in p.net_http.iter().filter(|h| !is_host(h)) {
            v.push(Issue::new("permissions.net-http", format!("`{host}` is not a lowercase host name")));
        }
        if !p.net_http.is_empty() && !self.sides.server {
            v.push(Issue::new("permissions.net-http", "outbound HTTP is only available to server mods"));
        }
        let unique: BTreeSet<_> = p.players.iter().collect();
        if unique.len() != p.players.len() {
            v.push(Issue::new("permissions.players", "duplicate entries"));
        }
    }

    fn validate_provides(&self, v: &mut Vec<Issue>) {
        let mut seen = BTreeSet::new();
        for (i, s) in self.provides.services.iter().enumerate() {
            let key = format!("provides.services[{i}]");
            if !is_mod_id(&s.name) {
                v.push(Issue::new(&key, "name must match [a-z0-9_-]{2,40}"));
            }
            if !seen.insert(&s.name) {
                v.push(Issue::new(&key, "duplicate service name"));
            }
            if Version::parse(&s.version).is_err() {
                v.push(Issue::new(&key, "version is not semver"));
            }
            if check_path(&s.wit).is_err() || !s.wit.starts_with("wit/") || !s.wit.ends_with(".wit") {
                v.push(Issue::new(&key, "wit must be a .wit file under wit/"));
            }
        }
    }

    fn validate_content(&self, v: &mut Vec<Issue>) {
        if self.content.is_empty() {
            return;
        }
        if !self.sides.client || self.sides.client_required == Some(false) {
            v.push(Issue::new(
                "content",
                "content needs a client half: set sides.client = true and do not set client-required = false",
            ));
        }
        let mut seen = BTreeSet::new();
        for key in self.content.keys() {
            let name = key.strip_prefix(&self.id).and_then(|r| r.strip_prefix(':'));
            if !name.is_some_and(|n| is_slug(n, 1, 40)) {
                v.push(Issue::new(
                    "content",
                    format!("key `{key}` must be `{}:<name>` with name [a-z0-9_-]{{1,40}}", self.id),
                ));
            }
            if !seen.insert(key) {
                v.push(Issue::new("content", format!("duplicate key `{key}`")));
            }
        }
        for (key, texture) in self.content.textures() {
            if check_path(texture).is_err() || !texture.starts_with("assets/") || !texture.ends_with(".png") {
                v.push(Issue::new("content", format!("texture of `{key}` must be a .png under assets/")));
            }
        }
        for b in &self.content.blocks {
            if b.hardness.is_some_and(|h| !h.is_finite() || h < 0.0) {
                v.push(Issue::new("content", format!("hardness of `{}` must be a finite number >= 0", b.key)));
            }
        }
        for i in &self.content.items {
            check_localized(v, &format!("content `{}` name", i.key), &i.name, false, 64);
        }
        for c in &self.content.cosmetics {
            self.validate_cosmetic(v, c);
        }
    }
}

impl Manifest {
    fn validate_cosmetic(&self, v: &mut Vec<Issue>, c: &CosmeticDecl) {
        let at = format!("content `{}`", c.key);
        if !crate::content::SLOTS.contains(&c.slot.as_str()) {
            v.push(Issue::new(&at, format!("slot must be one of {}", crate::content::SLOTS.join(", "))));
        }
        check_localized(v, &format!("{at} name"), &c.name, false, 64);
        if let Some(arm) = &c.texture_arm {
            if c.slot != "shirt" {
                v.push(Issue::new(&at, "texture-arm is only for shirt cosmetics"));
            }
            if check_path(arm).is_err() || !arm.starts_with("assets/") || !arm.ends_with(".png") {
                v.push(Issue::new(&at, "texture-arm must be a .png under assets/"));
            }
        }
        if let Some(base) = &c.base {
            match crate::content::base_item(base) {
                None => v.push(Issue::new(&at, format!("base `{base}` is not a known vanilla item"))),
                Some(b) if b.slot != c.slot => {
                    v.push(Issue::new(&at, format!("base `{base}` belongs to the {} slot, not {}", b.slot, c.slot)))
                }
                Some(_) => {}
            }
        } else if c.slot == "weapon" {
            v.push(Issue::new(&at, "a weapon cosmetic needs a base (the weapon it re-skins)"));
        }
    }
}

fn check_localized(v: &mut Vec<Issue>, key: &str, map: &Localized, required: bool, max: usize) {
    if (required || !map.is_empty()) && !map.contains_key("en") {
        v.push(Issue::new(key, "needs an `en` text"));
    }
    for (lang, text) in map {
        if !is_lang(lang) {
            v.push(Issue::new(key, format!("`{lang}` is not a language code")));
        }
        if text.trim().is_empty() || text.chars().count() > max {
            v.push(Issue::new(key, format!("`{lang}` text must be 1-{max} characters")));
        }
    }
}
