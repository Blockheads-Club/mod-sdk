//! Key bindings of client mods (`blockheads:api@0.3.4` `input`): validation of action ids and key names, the
//! bookkeeping of what each mod bound, and the [`InputCommand`]s a host program receives through
//! [`crate::GameBridge::input`]. Key presses come back through [`crate::ModHost::input_event`] as the custom event
//! `<mod-id>:input {action, pressed}`.

use std::collections::{BTreeMap, BTreeSet};

use crate::types::ApiError;

/// Most actions one mod may bind.
pub const MAX_ACTIONS: usize = 16;

/// A validated request to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputCommand {
    /// Declare (or re-declare) an action with its default key.
    Bind {
        /// The action id.
        action: String,
        /// The default key name.
        default_key: String,
    },
    /// Remove an action.
    Unbind {
        /// The action id.
        action: String,
    },
    /// The mod is gone: remove all its actions.
    ClearMod,
}

const NAMED: &[&str] = &[
    "space",
    "tab",
    "enter",
    "backspace",
    "insert",
    "delete",
    "home",
    "end",
    "page-up",
    "page-down",
    "up",
    "down",
    "left",
    "right",
    "left-shift",
    "right-shift",
    "left-ctrl",
    "right-ctrl",
    "left-alt",
    "right-alt",
    "grave",
    "minus",
    "equals",
    "left-bracket",
    "right-bracket",
    "backslash",
    "semicolon",
    "apostrophe",
    "comma",
    "period",
    "slash",
    "escape",
    "mouse-middle",
    "mouse-4",
    "mouse-5",
];

/// True for a key name of the `input` interface (`a`-`z`, `0`-`9`, `f1`-`f12`, `keypad-0`-`keypad-9` and the named
/// keys).
pub fn is_key(name: &str) -> bool {
    let b = name.as_bytes();
    if b.len() == 1 {
        return b[0].is_ascii_lowercase() || b[0].is_ascii_digit();
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=12).contains(&n) && !name.starts_with("f0");
    }
    if let Some(d) = name.strip_prefix("keypad-") {
        return d.len() == 1 && d.as_bytes()[0].is_ascii_digit();
    }
    NAMED.contains(&name)
}

/// What each mod (by slot index) bound: action -> default key.
#[derive(Debug, Default)]
pub struct InputState {
    actions: BTreeMap<usize, BTreeMap<String, String>>,
}

impl InputState {
    /// Checks a `bind` of `owner`.
    pub fn check_bind(&self, owner: usize, action: &str, key: &str) -> Result<(), ApiError> {
        crate::ui::check_id(action).map_err(|_| {
            ApiError::InvalidArgument(format!("action must be 1 to {} characters of a-z 0-9 _ . -", crate::ui::MAX_ID))
        })?;
        if !is_key(key) {
            return Err(ApiError::InvalidArgument(format!("unknown key name {key:?}")));
        }
        let mine = self.actions.get(&owner);
        if mine.is_none_or(|m| !m.contains_key(action)) && mine.map_or(0, BTreeMap::len) >= MAX_ACTIONS {
            return Err(ApiError::LimitExceeded("key bindings per mod".into()));
        }
        Ok(())
    }

    /// Records a binding.
    pub fn bound(&mut self, owner: usize, action: &str, key: &str) {
        self.actions.entry(owner).or_default().insert(action.to_string(), key.to_string());
    }

    /// Forgets a binding; false when there was none.
    pub fn unbound(&mut self, owner: usize, action: &str) -> bool {
        self.actions.get_mut(&owner).is_some_and(|m| m.remove(action).is_some())
    }

    /// The default key of a binding.
    pub fn default_key(&self, owner: usize, action: &str) -> Option<&str> {
        self.actions.get(&owner)?.get(action).map(String::as_str)
    }

    /// The mod's actions.
    pub fn actions(&self, owner: usize) -> BTreeSet<String> {
        self.actions.get(&owner).map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }

    /// The mod is gone; true when it had bindings.
    pub fn clear_owner(&mut self, owner: usize) -> bool {
        self.actions.remove(&owner).is_some_and(|m| !m.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        for k in ["a", "z", "0", "f1", "f12", "keypad-7", "space", "left-shift", "mouse-5", "escape"] {
            assert!(is_key(k), "{k}");
        }
        for k in ["", "A", "f0", "f13", "f01", "fx", "keypad-", "keypad-10", "keypad-a", "ctrl", "!"] {
            assert!(!is_key(k), "{k}");
        }
    }

    #[test]
    fn bindings() {
        let mut s = InputState::default();
        assert!(matches!(s.check_bind(1, "BAD", "v"), Err(ApiError::InvalidArgument(_))));
        assert!(matches!(s.check_bind(1, "ptt", "hyper"), Err(ApiError::InvalidArgument(_))));
        for i in 0..MAX_ACTIONS {
            s.check_bind(1, &format!("a{i}"), "v").unwrap();
            s.bound(1, &format!("a{i}"), "v");
        }
        s.check_bind(1, "a0", "b").unwrap();
        assert!(matches!(s.check_bind(1, "new", "b"), Err(ApiError::LimitExceeded(_))));
        s.check_bind(2, "new", "b").unwrap();
        assert_eq!(s.default_key(1, "a3"), Some("v"));
        assert_eq!(s.default_key(9, "a3"), None);
        assert_eq!(s.actions(1).len(), MAX_ACTIONS);
        assert!(s.actions(9).is_empty());
        assert!(s.unbound(1, "a0"));
        assert!(!s.unbound(1, "a0"));
        assert!(!s.unbound(9, "a0"));
        assert!(s.clear_owner(1));
        assert!(!s.clear_owner(1));
        s.bound(3, "x", "v");
        s.unbound(3, "x");
        assert!(!s.clear_owner(3));
    }
}
