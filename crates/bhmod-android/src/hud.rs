//! What the mods' API 0.3.4 calls leave for the loader (they run inside a mod call, where the host cannot be
//! entered again): the overhead icons, the key bindings and the settings screens a mod asked to open. Shared between
//! the [`crate::loader`] bridge and the loader.

use std::collections::{BTreeMap, BTreeSet};

use bhmod_core::input::InputCommand;

use crate::device::DeviceStore;
use crate::overhead::Board;

/// The shared HUD state.
#[derive(Debug, Default)]
pub struct Hud {
    /// Overhead icons.
    pub board: Board,
    /// Declared key bindings: mod -> action -> default key.
    bindings: BTreeMap<String, BTreeMap<String, String>>,
    /// Keys held down now (a second down without an up is a repeat).
    pressed: BTreeSet<String>,
    /// Settings screens mods asked to open (`ui.open-settings`), oldest first.
    pub open_settings: Vec<String>,
}

impl Hud {
    /// A mod's `input` call (already validated by the host).
    pub fn input(&mut self, mod_id: &str, cmd: &InputCommand) {
        match cmd {
            InputCommand::Bind { action, default_key } => {
                self.bindings.entry(mod_id.to_string()).or_default().insert(action.clone(), default_key.clone());
            }
            InputCommand::Unbind { action } => {
                if let Some(m) = self.bindings.get_mut(mod_id) {
                    m.remove(action);
                }
            }
            InputCommand::ClearMod => {
                self.bindings.remove(mod_id);
            }
        }
    }

    /// The actions of `mod_id` with their default keys.
    pub fn actions(&self, mod_id: &str) -> Vec<(String, String)> {
        self.bindings.get(mod_id).map(|m| m.iter().map(|(a, k)| (a.clone(), k.clone())).collect()).unwrap_or_default()
    }

    /// Mods with key bindings.
    pub fn binding_mods(&self) -> Vec<String> {
        self.bindings.iter().filter(|(_, m)| !m.is_empty()).map(|(id, _)| id.clone()).collect()
    }

    /// The key an action is bound to now: the player's choice on this device, else the default.
    pub fn key_of(device: &DeviceStore, mod_id: &str, action: &str, default_key: &str) -> Option<String> {
        match device.binding(mod_id, action) {
            Some(choice) => choice,
            None => Some(default_key.to_string()),
        }
    }

    /// A hardware key went down or up: the `(mod, action)`s bound to it. A repeated down or an up of a key that was
    /// not down gives nothing.
    pub fn key(&mut self, device: &DeviceStore, name: &str, down: bool) -> Vec<(String, String)> {
        let fresh = if down { self.pressed.insert(name.to_string()) } else { self.pressed.remove(name) };
        if !fresh {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for (m, actions) in &self.bindings {
            for (a, default) in actions {
                if Self::key_of(device, m, a, default).as_deref() == Some(name) {
                    hits.push((m.clone(), a.clone()));
                }
            }
        }
        hits
    }

    /// Forgets everything (the mods stopped).
    pub fn clear(&mut self) {
        self.board.clear_all();
        self.bindings.clear();
        self.pressed.clear();
        self.open_settings.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(a: &str, k: &str) -> InputCommand {
        InputCommand::Bind { action: a.into(), default_key: k.into() }
    }

    #[test]
    fn bindings_keys_and_rebinding() {
        let t = tempfile::tempdir().unwrap();
        let dev = DeviceStore::new(t.path());
        let mut h = Hud::default();
        h.input("voice", &bind("talk", "v"));
        h.input("voice", &bind("global", "g"));
        h.input("other", &bind("x", "v"));
        assert_eq!(h.actions("voice").len(), 2);
        assert!(h.actions("none").is_empty());
        assert_eq!(h.binding_mods(), vec!["other", "voice"]);
        let mut hits = h.key(&dev, "v", true);
        hits.sort();
        assert_eq!(hits, vec![("other".into(), "x".into()), ("voice".into(), "talk".into())]);
        assert!(h.key(&dev, "v", true).is_empty(), "repeat");
        assert_eq!(h.key(&dev, "v", false).len(), 2);
        assert!(h.key(&dev, "v", false).is_empty(), "up without down");
        dev.set_binding("voice", "talk", Some(Some("b"))).unwrap();
        dev.set_binding("voice", "global", Some(None)).unwrap();
        assert_eq!(h.key(&dev, "b", true), vec![("voice".into(), "talk".into())]);
        assert!(h.key(&dev, "g", true).is_empty());
        assert_eq!(Hud::key_of(&dev, "voice", "global", "g"), None);
        h.input("voice", &InputCommand::Unbind { action: "talk".into() });
        h.input("nobody", &InputCommand::Unbind { action: "talk".into() });
        assert_eq!(h.actions("voice").len(), 1);
        h.input("other", &InputCommand::ClearMod);
        h.input("voice", &InputCommand::Unbind { action: "global".into() });
        assert!(h.binding_mods().is_empty());
        h.board.set("voice", "p", "speaking");
        h.open_settings.push("voice".into());
        h.clear();
        assert!(!h.board.active() && h.open_settings.is_empty());
    }
}
