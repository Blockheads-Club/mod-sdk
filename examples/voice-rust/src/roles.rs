//! Who may talk in which mode: the role × mode matrix.
//!
//! A talk list holds built-in role names and permission groups. Built-in roles are ranked
//! (`everyone` < `mod` < `admin` < `owner`) and a player matches a role when their own rank is at least as high, so
//! `["mod"]` lets moderators, admins and the owner talk. Any other name is a group of the official permissions mod:
//! the player is in group `g` when the `permissions` service says they have the node `group.<g>` (admins and the owner
//! have every node there).

use crate::config::ServerConfig;
use crate::model::{Mode, ModeSet, Role};

/// The rank of a built-in role name (`None` for a permission group).
pub fn builtin(name: &str) -> Option<Role> {
    match name {
        "everyone" | "player" => Some(Role::Player),
        "mod" | "moderator" => Some(Role::Moderator),
        "admin" => Some(Role::Admin),
        "owner" => Some(Role::Owner),
        _ => None,
    }
}

/// The permission node of group `name`.
pub fn group_node(name: &str) -> String {
    format!("group.{name}")
}

/// True when a player with `role` (in the groups `in_group` says yes to) matches one entry of `list`.
pub fn matches(role: Role, list: &[String], in_group: &dyn Fn(&str) -> bool) -> bool {
    list.iter().any(|name| match builtin(name) {
        Some(r) => role >= r,
        None => in_group(name),
    })
}

/// The modes a player may talk in: switched on by the admin and allowed by the matrix.
pub fn allowed_modes(cfg: &ServerConfig, role: Role, in_group: &dyn Fn(&str) -> bool) -> ModeSet {
    Mode::ALL
        .into_iter()
        .filter(|m| cfg.modes.has(*m) && matches(role, cfg.talk_roles(*m), in_group))
        .fold(ModeSet::EMPTY, ModeSet::with)
}

/// The permission groups named anywhere in the matrix, sorted, without duplicates.
pub fn groups(cfg: &ServerConfig) -> Vec<String> {
    let mut out: Vec<String> = cfg.talk.iter().flatten().filter(|n| builtin(n).is_none()).cloned().collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> bool {
        false
    }

    fn strings(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ranks() {
        let staff = strings(&["mod"]);
        assert!(!matches(Role::Player, &staff, &none));
        assert!(matches(Role::Moderator, &staff, &none));
        assert!(matches(Role::Owner, &staff, &none));
        assert!(matches(Role::Player, &strings(&["everyone"]), &none));
        assert!(matches(Role::Player, &strings(&["player"]), &none));
        assert!(!matches(Role::Admin, &strings(&["owner"]), &none));
        assert!(matches(Role::Admin, &strings(&["moderator"]), &none));
        assert!(!matches(Role::Owner, &[], &none));
        assert_eq!(group_node("vip"), "group.vip");
    }

    #[test]
    fn groups_ask_the_permissions_mod() {
        let list = strings(&["admin", "vip"]);
        assert!(!matches(Role::Player, &list, &none));
        assert!(matches(Role::Player, &list, &|g| g == "vip"));
    }

    #[test]
    fn matrix() {
        let mut cfg = ServerConfig::default();
        assert_eq!(allowed_modes(&cfg, Role::Player, &none), ModeSet::of(&[Mode::Proximity, Mode::Channel]));
        assert_eq!(
            allowed_modes(&cfg, Role::Moderator, &none),
            ModeSet::of(&[Mode::Proximity, Mode::Channel, Mode::Broadcast])
        );
        cfg.modes = cfg.modes.with(Mode::Global);
        cfg.talk[1] = strings(&["vip", "admin", "vip"]);
        cfg.talk[3] = strings(&["staff"]);
        assert_eq!(
            allowed_modes(&cfg, Role::Player, &|g| g == "vip"),
            ModeSet::of(&[Mode::Proximity, Mode::Global, Mode::Channel])
        );
        assert_eq!(allowed_modes(&cfg, Role::Owner, &none).0, 0b0111);
        assert_eq!(groups(&cfg), ["staff", "vip"]);
        assert!(groups(&ServerConfig::default()).is_empty());
    }
}
