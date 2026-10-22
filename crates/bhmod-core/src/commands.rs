//! The command registry: names and aliases of mod commands, owned per mod.

use std::collections::BTreeMap;

use crate::runtime::CommandId;
use crate::types::{ApiError, CommandSpec};

/// One registered command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Id given to the mod.
    pub id: CommandId,
    /// Owning mod (slot index).
    pub owner: usize,
    /// The spec as registered (name and aliases lower-case).
    pub spec: CommandSpec,
}

/// All mod commands.
#[derive(Debug, Default)]
pub struct CommandRegistry {
    cmds: BTreeMap<CommandId, Command>,
    labels: BTreeMap<String, CommandId>,
    next_id: CommandId,
}

/// `[a-z0-9_-]{1,32}`.
pub fn valid_name(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Most commands per mod.
pub const MAX_PER_MOD: usize = 256;

impl CommandRegistry {
    /// Registers a command. `builtin` says whether a name belongs to the game.
    pub fn register(
        &mut self,
        owner: usize,
        spec: CommandSpec,
        builtin: &dyn Fn(&str) -> bool,
    ) -> Result<CommandId, ApiError> {
        let mut labels = vec![spec.name.clone()];
        labels.extend(spec.aliases.iter().cloned());
        for (i, l) in labels.iter().enumerate() {
            if !valid_name(l) {
                return Err(ApiError::InvalidArgument(format!("command name {l:?} must match [a-z0-9_-]{{1,32}}")));
            }
            if builtin(l) || self.labels.contains_key(l) || labels[..i].contains(l) {
                return Err(ApiError::InvalidArgument(format!("command /{l} is already taken")));
            }
        }
        if self.cmds.values().filter(|c| c.owner == owner).count() >= MAX_PER_MOD {
            return Err(ApiError::LimitExceeded(format!("more than {MAX_PER_MOD} commands")));
        }
        self.next_id += 1;
        let id = self.next_id;
        for l in labels {
            self.labels.insert(l, id);
        }
        self.cmds.insert(id, Command { id, owner, spec });
        Ok(id)
    }

    /// Removes one of `owner`'s commands.
    pub fn unregister(&mut self, owner: usize, id: CommandId) -> bool {
        if self.cmds.get(&id).is_none_or(|c| c.owner != owner) {
            return false;
        }
        self.cmds.remove(&id);
        self.labels.retain(|_, v| *v != id);
        true
    }

    /// Removes every command of `owner`.
    pub fn remove_owner(&mut self, owner: usize) {
        let ids: Vec<CommandId> = self.cmds.values().filter(|c| c.owner == owner).map(|c| c.id).collect();
        for id in ids {
            self.unregister(owner, id);
        }
    }

    /// The command a typed label (name or alias, any case) refers to.
    pub fn lookup(&self, label: &str) -> Option<&Command> {
        self.labels.get(&label.to_ascii_lowercase()).and_then(|id| self.cmds.get(id))
    }

    /// Every command, by id.
    pub fn all(&self) -> impl Iterator<Item = &Command> {
        self.cmds.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, aliases: &[&str]) -> CommandSpec {
        CommandSpec { aliases: aliases.iter().map(|s| s.to_string()).collect(), ..CommandSpec::new(name) }
    }

    #[test]
    fn register_lookup_unregister() {
        let mut r = CommandRegistry::default();
        let builtin = |n: &str| n == "help";
        let a = r.register(0, spec("hello", &["hi"]), &builtin).unwrap();
        assert_eq!(r.lookup("HI").unwrap().id, a);
        assert_eq!(r.lookup("hello").unwrap().owner, 0);
        assert!(r.lookup("nope").is_none());
        for bad in [spec("help", &[]), spec("x", &["hi"]), spec("Bad", &[]), spec("y", &["y"]), spec("", &[])] {
            assert!(matches!(r.register(1, bad, &builtin), Err(ApiError::InvalidArgument(_))));
        }
        let b = r.register(1, spec("bye", &[]), &builtin).unwrap();
        assert_eq!(r.all().count(), 2);
        assert!(!r.unregister(1, a));
        assert!(!r.unregister(0, 99));
        assert!(r.unregister(0, a));
        assert!(r.lookup("hi").is_none());
        r.remove_owner(1);
        assert!(r.lookup("bye").is_none());
        assert_eq!(r.all().count(), 0);
        let _ = b;
    }

    #[test]
    fn names_and_limit() {
        assert!(valid_name("a-b_9"));
        assert!(!valid_name(&"a".repeat(33)));
        let mut r = CommandRegistry::default();
        let none = |_: &str| false;
        for i in 0..MAX_PER_MOD {
            r.register(0, spec(&format!("c{i}"), &[]), &none).unwrap();
        }
        assert!(matches!(r.register(0, spec("more", &[]), &none), Err(ApiError::LimitExceeded(_))));
    }
}
