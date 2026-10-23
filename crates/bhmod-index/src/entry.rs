//! The index entry model (`mods/<letter>/<id>.toml`) and JSON Schema checks.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bhmod_manifest::manifest::Permissions;
use jsonschema::{Registry, Validator};
use serde::Deserialize;
use serde_json::Value;

use crate::Issue;

const BASE: &str = "https://blockheads-club.github.io/mod-sdk/schema/";
const MOD_SCHEMA: &str = include_str!("../../../schema/mod.schema.json");
const ENTRY_SCHEMA: &str = include_str!("../../../schema/index-entry.schema.json");
const INDEX_SCHEMA: &str = include_str!("../../../schema/index.schema.json");

/// Which schema to check against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schema {
    /// `index-entry.schema.json`.
    Entry,
    /// `index.schema.json`.
    Index,
}

fn validators() -> &'static (Validator, Validator) {
    static V: OnceLock<(Validator, Validator)> = OnceLock::new();
    V.get_or_init(|| {
        let parse = |s: &str| serde_json::from_str::<Value>(s).expect("bundled schema is JSON");
        let registry = Registry::new()
            .add(format!("{BASE}mod.schema.json"), parse(MOD_SCHEMA))
            .and_then(|r| r.add(format!("{BASE}index-entry.schema.json"), parse(ENTRY_SCHEMA)))
            .and_then(|r| r.add(format!("{BASE}index.schema.json"), parse(INDEX_SCHEMA)))
            .and_then(|r| r.prepare())
            .expect("bundled schemas register");
        let build = |s: &str| jsonschema::options().with_registry(&registry).build(&parse(s)).expect("schema builds");
        (build(ENTRY_SCHEMA), build(INDEX_SCHEMA))
    })
}

/// Schema errors of `value`, as `"<json pointer>: <message>"`.
pub fn schema_errors(schema: Schema, value: &Value) -> Vec<String> {
    let (entry, index) = validators();
    let v = if schema == Schema::Entry { entry } else { index };
    v.iter_errors(value).map(|e| format!("{}: {e}", e.instance_path())).collect()
}

/// TOML to JSON; dates become their TOML text (`2026-10-21`), as the schemas expect.
pub fn toml_to_json(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => Value::from(f),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect()),
    }
}

/// A mod's index entry.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Entry {
    /// Mod id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Short description.
    #[serde(default)]
    pub description: Option<String>,
    /// GitHub user names allowed to submit new versions.
    pub owners: Vec<String>,
    /// Source repository.
    pub repo: String,
    /// Search tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Translations of `name` and `description` by language code; English is the main fields.
    #[serde(default)]
    pub i18n: BTreeMap<String, Localized>,
    /// Placeholder entry, never published.
    #[serde(default)]
    pub draft: bool,
    /// Every published version, oldest first.
    pub versions: Vec<VersionEntry>,
}

/// A translated name and description; a missing field falls back to English.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localized {
    /// Translated display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Translated description.
    #[serde(default)]
    pub description: Option<String>,
}

/// One published version.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct VersionEntry {
    /// Semver.
    pub version: String,
    /// Where the author publishes the package.
    pub url: String,
    /// Copy kept by the index.
    #[serde(default)]
    pub mirror: Option<String>,
    /// SHA-256 of the package, lowercase hex.
    pub sha256: String,
    /// Package size in bytes.
    #[serde(default)]
    pub size: Option<u64>,
    /// Sides (`server`, `client`).
    pub sides: Vec<String>,
    /// Whether players need the client half.
    #[serde(default)]
    pub client_required: Option<bool>,
    /// `blockheads:api` requirement.
    pub api: String,
    /// Loader requirement.
    #[serde(default)]
    pub loader: Option<String>,
    /// Required mods.
    #[serde(default)]
    pub depends: BTreeMap<String, String>,
    /// Optional mods.
    #[serde(default)]
    pub soft_depends: BTreeMap<String, String>,
    /// Permissions.
    pub permissions: Permissions,
    /// Publication date, `YYYY-MM-DD`.
    pub published: String,
    /// Withdrawn.
    #[serde(default)]
    pub yanked: bool,
}

impl Entry {
    /// Parses entry TOML: TOML syntax, then the JSON Schema, then the typed model.
    /// Returns the JSON form (used for building `index.json`) and the typed entry.
    pub fn parse(path: &str, text: &str) -> Result<(Value, Entry), Vec<Issue>> {
        let toml: toml::Value = toml::from_str(text).map_err(|e| vec![Issue::new(path, e.message())])?;
        let json = toml_to_json(toml);
        let errors = schema_errors(Schema::Entry, &json);
        if !errors.is_empty() {
            return Err(errors.into_iter().map(|e| Issue::new(path, e)).collect());
        }
        let entry = Entry::deserialize(&json).expect("schema-valid entry deserializes");
        Ok((json, entry))
    }

    /// The version entry with this version string.
    pub fn version(&self, v: &str) -> Option<&VersionEntry> {
        self.versions.iter().find(|x| x.version == v)
    }
}

/// A permission set as sorted, flat labels (`world:write`, `players:kick`, `net-http:host`, `channels`, `native`).
pub fn permission_labels(p: &Permissions) -> Vec<String> {
    let mut v = Vec::new();
    match p.world {
        bhmod_manifest::manifest::WorldAccess::None => {}
        bhmod_manifest::manifest::WorldAccess::Read => v.push("world:read".to_string()),
        bhmod_manifest::manifest::WorldAccess::Write => v.push("world:write".to_string()),
    }
    let mut players: Vec<_> = p.players.clone();
    players.sort();
    v.extend(players.iter().map(|x| format!("players:{}", format!("{x:?}").to_lowercase())));
    let mut hosts = p.net_http.clone();
    hosts.sort();
    v.extend(hosts.iter().map(|h| format!("net-http:{h}")));
    if p.channels {
        v.push("channels".into());
    }
    if p.native {
        v.push("native".into());
    }
    v
}
