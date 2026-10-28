//! `mods.toml`: the admin's per-mod settings, next to the mods in the mods directory.
//!
//! ```toml
//! [hello-lua]
//! grant = ["players:chat"]
//!
//! [noisy-mod]
//! enabled = false
//!
//! [voice]
//! client-required = false   # players without the mod may join (ignored for mods with [content])
//!
//! [voice.settings]          # values of the mod's `[settings]` (API 0.3.3); missing keys use the default
//! radius = 48
//! ```
//!
//! A mod that asks for a permission the admin has not granted is not enabled. Permission names:
//! `world:read`, `world:write` (also covers `world:read`), `players:<chat|teleport|kick|ban|inventory|health>`,
//! `net-http:<host>`, `channels`, `native`.

use std::collections::BTreeMap;
use std::path::Path;

use bhmod_manifest::manifest::{Manifest, Permissions, PlayerPermission, WorldAccess};
use serde::Deserialize;
use serde_json::{Map, Value as Json};

/// The file name inside the mods directory.
pub const FILE_NAME: &str = "mods.toml";

/// Settings of one mod.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModConfig {
    /// False keeps the mod installed but never loads it.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Granted permissions.
    #[serde(default)]
    pub grant: Vec<String>,
    /// The admin's override of `sides.client-required` (see [`client_required`]).
    #[serde(default, rename = "client-required")]
    pub client_required: Option<bool>,
    /// Stored values of the mod's `[settings]` (API 0.3.3), as written.
    #[serde(default)]
    pub settings: toml::Table,
}

fn yes() -> bool {
    true
}

impl Default for ModConfig {
    fn default() -> Self {
        ModConfig { enabled: true, grant: Vec::new(), client_required: None, settings: toml::Table::new() }
    }
}

impl ModConfig {
    /// The stored settings as JSON. A value TOML has but JSON has not (a date) becomes a text, which no setting
    /// type accepts, so it falls back to the default with a warning.
    pub fn settings_json(&self) -> Map<String, Json> {
        self.settings
            .iter()
            .map(|(k, v)| {
                let j = bhmod_manifest::settings::toml_to_json(v).unwrap_or_else(|| Json::String(v.to_string()));
                (k.clone(), j)
            })
            .collect()
    }
}

/// Whether players must run the client half of `m`: `[content]` always needs it and a mod without a client half
/// never does; otherwise the admin's `client-required` wins over the manifest.
pub fn client_required(m: &Manifest, c: &ModConfig) -> bool {
    if !m.content.is_empty() {
        return true;
    }
    m.sides.client && c.client_required.unwrap_or_else(|| m.client_required())
}

/// JSON to a TOML value for `mods.toml` (`null` has no TOML form and gives `None`).
pub fn json_to_toml(v: &Json) -> Option<toml_edit::Value> {
    Some(match v {
        Json::Null => return None,
        Json::Bool(b) => (*b).into(),
        Json::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64()?.into(),
        },
        Json::String(s) => s.as_str().into(),
        Json::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for x in a {
                arr.push(json_to_toml(x)?);
            }
            toml_edit::Value::Array(arr)
        }
        Json::Object(o) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, x) in o {
                t.insert(k, json_to_toml(x)?);
            }
            toml_edit::Value::InlineTable(t)
        }
    })
}

/// JSON to a [`toml::Value`] (`null` gives `None`).
pub fn json_to_toml_value(v: &Json) -> Option<toml::Value> {
    Some(match v {
        Json::Null => return None,
        Json::Bool(b) => toml::Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64()?),
        },
        Json::String(s) => toml::Value::String(s.clone()),
        Json::Array(a) => toml::Value::Array(a.iter().map(json_to_toml_value).collect::<Option<_>>()?),
        Json::Object(o) => {
            toml::Value::Table(o.iter().map(|(k, x)| Some((k.clone(), json_to_toml_value(x)?))).collect::<Option<_>>()?)
        }
    })
}

/// Writes one stored setting of mod `id` into `mods.toml` in `dir` (`None` removes it, so the default applies),
/// keeping everything else in the file (comments, other mods, order). An invalid file is not touched.
pub fn write_setting(dir: &Path, id: &str, key: &str, value: Option<&Json>) -> Result<(), String> {
    let path = dir.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|e| format!("{} is invalid, not changing it: {e}", path.display()))?;
    let value = match value {
        Some(v) => Some(json_to_toml(v).ok_or_else(|| format!("{key}: a null value cannot be stored"))?),
        None => None,
    };
    let table = doc.entry(id).or_insert_with(toml_edit::table).as_table_like_mut();
    let Some(table) = table else { return Err(format!("{}: `{id}` is not a table", path.display())) };
    match value {
        Some(v) => {
            let settings = table.entry("settings").or_insert_with(toml_edit::table).as_table_like_mut();
            let Some(settings) = settings else {
                return Err(format!("{}: `{id}.settings` is not a table", path.display()));
            };
            settings.insert(key, toml_edit::Item::Value(v));
        }
        None => {
            if let Some(settings) = table.get_mut("settings").and_then(|s| s.as_table_like_mut()) {
                settings.remove(key);
                if settings.is_empty() {
                    table.remove("settings");
                }
            }
        }
    }
    if table.is_empty() {
        doc.remove(id);
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The whole file: mod id -> settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModsConfig {
    /// Per-mod settings.
    pub mods: BTreeMap<String, ModConfig>,
}

impl ModsConfig {
    /// Parses `mods.toml` text.
    pub fn parse(text: &str) -> Result<ModsConfig, String> {
        let mods: BTreeMap<String, ModConfig> = toml::from_str(text).map_err(|e| e.to_string())?;
        Ok(ModsConfig { mods })
    }

    /// Settings of one mod (defaults when absent).
    pub fn get(&self, id: &str) -> ModConfig {
        self.mods.get(id).cloned().unwrap_or_default()
    }
}

fn player_permission_name(p: PlayerPermission) -> &'static str {
    match p {
        PlayerPermission::Chat => "chat",
        PlayerPermission::Teleport => "teleport",
        PlayerPermission::Kick => "kick",
        PlayerPermission::Ban => "ban",
        PlayerPermission::Inventory => "inventory",
        PlayerPermission::Health => "health",
    }
}

/// The permission names a manifest asks for.
pub fn required_permissions(p: &Permissions) -> Vec<String> {
    let mut out = Vec::new();
    match p.world {
        WorldAccess::None => {}
        WorldAccess::Read => out.push("world:read".to_string()),
        WorldAccess::Write => out.push("world:write".to_string()),
    }
    out.extend(p.players.iter().map(|x| format!("players:{}", player_permission_name(*x))));
    out.extend(p.net_http.iter().map(|h| format!("net-http:{h}")));
    if p.channels {
        out.push("channels".to_string());
    }
    if p.native {
        out.push("native".to_string());
    }
    out
}

/// Required permissions that `grant` does not cover.
pub fn missing_permissions(required: &[String], grant: &[String]) -> Vec<String> {
    required
        .iter()
        .filter(|r| !(grant.contains(r) || (r.as_str() == "world:read" && grant.iter().any(|g| g == "world:write"))))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_defaults() {
        let c = ModsConfig::parse("[a]\ngrant = [\"players:chat\"]\n[b]\nenabled = false\n").unwrap();
        assert_eq!(c.get("a"), ModConfig { grant: vec!["players:chat".into()], ..ModConfig::default() });
        assert!(!c.get("b").enabled);
        assert_eq!(c.get("zzz"), ModConfig::default());
        assert!(ModsConfig::parse("[a]\nbogus = 1\n").is_err());
        assert!(ModsConfig::parse("not toml =").is_err());
        assert_eq!(ModsConfig::parse("").unwrap(), ModsConfig::default());
    }

    const MANIFEST: &str = "schema = 1\nid = \"mod1\"\nversion = \"1.0.0\"\nname = { en = \"M\" }\nauthors = [\"a\"]\n\
license = \"MIT\"\napi = \"^0.3\"\n";

    fn manifest(extra: &str) -> Manifest {
        Manifest::parse(&format!("{MANIFEST}{extra}")).unwrap()
    }

    #[test]
    fn client_required_override() {
        let both = "[sides]\nserver = true\nclient = true\n[entry]\nserver = { runtime = \"lua\", path = \"server/a.luau\" }\n\
client = { runtime = \"lua\", path = \"client/a.luau\" }\n";
        let m = manifest(both);
        let off = ModConfig { client_required: Some(false), ..ModConfig::default() };
        assert!(client_required(&m, &ModConfig::default()));
        assert!(!client_required(&m, &off));
        let opt = manifest(&both.replace("[entry]", "client-required = false\n[entry]"));
        assert!(!client_required(&opt, &ModConfig::default()));
        let on = ModConfig { client_required: Some(true), ..ModConfig::default() };
        assert!(client_required(&opt, &on));
        let server =
            manifest("[sides]\nserver = true\n[entry]\nserver = { runtime = \"lua\", path = \"server/a.luau\" }\n");
        assert!(!client_required(&server, &on), "no client half");
        let content = manifest(&format!(
            "{both}[content]\ncosmetics = [{{ key = \"mod1:hat\", slot = \"hat\", texture = \"assets/h.png\" }}]\n"
        ));
        assert!(client_required(&content, &off), "content always needs the client");
    }

    #[test]
    fn stored_settings() {
        let c = ModsConfig::parse(
            "[m]\nclient-required = false\n[m.settings]\na = 1\nb = { c = [true] }\nd = 1979-05-27\n",
        )
        .unwrap()
        .get("m");
        assert_eq!(c.client_required, Some(false));
        let j = c.settings_json();
        assert_eq!(j["a"], serde_json::json!(1));
        assert_eq!(j["b"], serde_json::json!({ "c": [true] }));
        assert_eq!(j["d"], serde_json::json!("1979-05-27"));
    }

    #[test]
    fn json_to_toml_forms() {
        let j = serde_json::json!({ "a": [1, 2.5, "x", true], "b": { "c": false } });
        let v = json_to_toml(&j).unwrap();
        assert_eq!(v.to_string(), "{ a = [1, 2.5, \"x\", true], b = { c = false } }");
        assert!(json_to_toml(&serde_json::json!([null])).is_none());
        assert!(json_to_toml(&serde_json::json!({ "a": null })).is_none());
        let t = json_to_toml_value(&j).unwrap();
        assert_eq!(t["a"].as_array().unwrap().len(), 4);
        assert_eq!(t["b"]["c"].as_bool(), Some(false));
        assert!(json_to_toml_value(&serde_json::json!([null])).is_none());
    }

    #[test]
    fn write_settings_keeps_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        let read = || std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        write_setting(dir, "m", "a", Some(&serde_json::json!(5))).unwrap();
        assert_eq!(read(), "[m]\n\n[m.settings]\na = 5\n");
        std::fs::write(
            dir.join(FILE_NAME),
            "# admin notes\n[m]\ngrant = [\"channels\"] # keep\n\n[m.settings]\na = 5\n",
        )
        .unwrap();
        write_setting(dir, "m", "b", Some(&serde_json::json!(["x"]))).unwrap();
        assert!(read().starts_with("# admin notes\n[m]\ngrant = [\"channels\"] # keep\n"), "{}", read());
        assert_eq!(ModsConfig::parse(&read()).unwrap().get("m").settings.len(), 2);
        write_setting(dir, "m", "a", None).unwrap();
        write_setting(dir, "m", "b", None).unwrap();
        assert_eq!(read(), "# admin notes\n[m]\ngrant = [\"channels\"] # keep\n");
        write_setting(dir, "m", "gone", None).unwrap();
        // An entry that only held settings disappears with them.
        write_setting(dir, "n", "a", Some(&serde_json::json!(true))).unwrap();
        write_setting(dir, "n", "a", None).unwrap();
        assert!(!read().contains("[n"));
        // Inline tables are edited in place.
        std::fs::write(dir.join(FILE_NAME), "m = { settings = { a = 1 } }\n").unwrap();
        write_setting(dir, "m", "a", Some(&serde_json::json!(2))).unwrap();
        assert_eq!(ModsConfig::parse(&read()).unwrap().get("m").settings["a"].as_integer(), Some(2));
        // Problems leave the file alone.
        for bad in ["m = 1\n", "[m]\nsettings = 1\n", "not toml ="] {
            std::fs::write(dir.join(FILE_NAME), bad).unwrap();
            assert!(write_setting(dir, "m", "a", Some(&serde_json::json!(1))).is_err(), "{bad}");
            assert_eq!(read(), bad);
        }
        std::fs::write(dir.join(FILE_NAME), "").unwrap();
        assert!(write_setting(dir, "m", "a", Some(&serde_json::json!(null))).unwrap_err().contains("null"));
        assert!(write_setting(&dir.join("missing/dir"), "m", "a", Some(&serde_json::json!(1)))
            .unwrap_err()
            .contains("cannot write"));
        std::fs::create_dir(dir.join("isdir")).unwrap();
        std::fs::create_dir(dir.join("isdir").join(FILE_NAME)).unwrap();
        assert!(write_setting(&dir.join("isdir"), "m", "a", None).unwrap_err().contains("cannot read"));
    }

    #[test]
    fn permissions() {
        let all = Permissions {
            world: WorldAccess::Read,
            players: vec![
                PlayerPermission::Chat,
                PlayerPermission::Teleport,
                PlayerPermission::Kick,
                PlayerPermission::Ban,
                PlayerPermission::Inventory,
                PlayerPermission::Health,
            ],
            net_http: vec!["api.example.com".into()],
            channels: true,
            native: true,
        };
        let req = required_permissions(&all);
        assert_eq!(
            req,
            vec![
                "world:read",
                "players:chat",
                "players:teleport",
                "players:kick",
                "players:ban",
                "players:inventory",
                "players:health",
                "net-http:api.example.com",
                "channels",
                "native"
            ]
        );
        let write = Permissions { world: WorldAccess::Write, ..Permissions::default() };
        assert_eq!(required_permissions(&write), vec!["world:write"]);
        assert!(required_permissions(&Permissions::default()).is_empty());

        let grant = vec!["world:write".to_string(), "players:chat".to_string()];
        assert!(missing_permissions(&["world:read".into(), "players:chat".into()], &grant).is_empty());
        assert_eq!(missing_permissions(&["players:kick".into()], &grant), vec!["players:kick"]);
        assert_eq!(missing_permissions(&["world:read".into()], &[]), vec!["world:read"]);
    }
}
