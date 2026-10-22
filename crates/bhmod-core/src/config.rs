//! `mods.toml`: the admin's per-mod settings, next to the mods in the mods directory.
//!
//! ```toml
//! [hello-lua]
//! grant = ["players:chat"]
//!
//! [noisy-mod]
//! enabled = false
//! ```
//!
//! A mod that asks for a permission the admin has not granted is not enabled. Permission names:
//! `world:read`, `world:write` (also covers `world:read`), `players:<chat|teleport|kick|ban|inventory|health>`,
//! `net-http:<host>`, `channels`, `native`.

use std::collections::BTreeMap;

use bhmod_manifest::manifest::{Permissions, PlayerPermission, WorldAccess};
use serde::Deserialize;

/// The file name inside the mods directory.
pub const FILE_NAME: &str = "mods.toml";

/// Settings of one mod.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModConfig {
    /// False keeps the mod installed but never loads it.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Granted permissions.
    #[serde(default)]
    pub grant: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Default for ModConfig {
    fn default() -> Self {
        ModConfig { enabled: true, grant: Vec::new() }
    }
}

/// The whole file: mod id -> settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        assert_eq!(c.get("a"), ModConfig { enabled: true, grant: vec!["players:chat".into()] });
        assert!(!c.get("b").enabled);
        assert_eq!(c.get("zzz"), ModConfig::default());
        assert!(ModsConfig::parse("[a]\nbogus = 1\n").is_err());
        assert!(ModsConfig::parse("not toml =").is_err());
        assert_eq!(ModsConfig::parse("").unwrap(), ModsConfig::default());
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
