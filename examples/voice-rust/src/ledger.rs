//! Admin mutes and speaker gains, kept in the mod's storage (key `ledger`, canonical CBOR) so they survive restarts.
//! Entries are keyed by player id and remember the alias, so `/voice unmute <alias>` works while the player is away.

use std::collections::BTreeMap;

use blockheads_mod::cbor::Value;

/// One admin mute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mute {
    /// The alias when muted.
    pub alias: String,
    /// Unix time in ms when it ends (`None` = until unmuted).
    pub until_ms: Option<u64>,
}

/// One speaker gain set with `/voice gain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gain {
    /// The alias when set.
    pub alias: String,
    /// Percent, 0..=200.
    pub percent: u16,
}

/// Mutes and gains by player id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    /// Mutes.
    pub mutes: BTreeMap<String, Mute>,
    /// Gains.
    pub gains: BTreeMap<String, Gain>,
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

impl Ledger {
    /// The stored form.
    pub fn to_value(&self) -> Value {
        let mutes = self
            .mutes
            .iter()
            .map(|(id, m)| {
                let until = m.until_ms.map_or(Value::Null, |u| Value::Int(u as i64));
                (id.clone(), Value::Map(vec![("alias".into(), text(&m.alias)), ("until".into(), until)]))
            })
            .collect();
        let gains = self
            .gains
            .iter()
            .map(|(id, g)| {
                let entry = vec![("alias".into(), text(&g.alias)), ("percent".into(), Value::Int(g.percent.into()))];
                (id.clone(), Value::Map(entry))
            })
            .collect();
        Value::Map(vec![("gains".into(), Value::Map(gains)), ("mutes".into(), Value::Map(mutes))])
    }

    /// Reads the stored form; malformed entries are skipped.
    pub fn from_value(v: &Value) -> Ledger {
        let mut l = Ledger::default();
        if let Some(Value::Map(m)) = v.get("mutes") {
            for (id, e) in m {
                let Some(alias) = e.get("alias").and_then(Value::as_str) else { continue };
                let until_ms = e.get("until").and_then(Value::as_i64).and_then(|u| u64::try_from(u).ok());
                l.mutes.insert(id.clone(), Mute { alias: alias.into(), until_ms });
            }
        }
        if let Some(Value::Map(m)) = v.get("gains") {
            for (id, e) in m {
                let alias = e.get("alias").and_then(Value::as_str);
                let percent = e.get("percent").and_then(Value::as_i64).filter(|p| (0..=200).contains(p));
                if let (Some(alias), Some(p)) = (alias, percent) {
                    l.gains.insert(id.clone(), Gain { alias: alias.into(), percent: p as u16 });
                }
            }
        }
        l
    }

    /// Removes mutes that ended before `now_ms`; returns their ids.
    pub fn expire(&mut self, now_ms: u64) -> Vec<String> {
        let gone: Vec<String> = self
            .mutes
            .iter()
            .filter(|(_, m)| m.until_ms.is_some_and(|u| u <= now_ms))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &gone {
            self.mutes.remove(id);
        }
        gone
    }

    /// The id of a muted player by alias (case-insensitive).
    pub fn muted_id_by_alias(&self, alias: &str) -> Option<String> {
        self.mutes.iter().find(|(_, m)| m.alias.eq_ignore_ascii_case(alias)).map(|(id, _)| id.clone())
    }

    /// Sets a gain; 100 removes the entry.
    pub fn set_gain(&mut self, id: &str, alias: &str, percent: u16) {
        if percent == 100 {
            self.gains.remove(id);
        } else {
            self.gains.insert(id.into(), Gain { alias: alias.into(), percent });
        }
    }

    /// The gain of a player: `/voice gain` first, then the config list (by id or alias), else 100.
    pub fn gain_of(&self, id: &str, alias: &str, config: &[(String, u16)]) -> u16 {
        if let Some(g) = self.gains.get(id) {
            return g.percent;
        }
        config.iter().find(|(who, _)| who == id || who.eq_ignore_ascii_case(alias)).map_or(100, |(_, p)| *p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut l = Ledger::default();
        l.mutes.insert("1".into(), Mute { alias: "Bob".into(), until_ms: Some(5000) });
        l.mutes.insert("2".into(), Mute { alias: "Eve".into(), until_ms: None });
        l.set_gain("3", "Loud", 60);
        let back = Ledger::from_value(&Value::decode(&l.to_value().encode()).unwrap());
        assert_eq!(back, l);
    }

    #[test]
    fn malformed_entries() {
        let v = Value::Map(vec![
            (
                "mutes".into(),
                Value::Map(vec![
                    ("1".into(), Value::Map(vec![("until".into(), Value::Int(1))])),
                    ("2".into(), Value::Map(vec![("alias".into(), text("A")), ("until".into(), Value::Int(-1))])),
                ]),
            ),
            (
                "gains".into(),
                Value::Map(vec![
                    ("1".into(), Value::Map(vec![("alias".into(), text("A")), ("percent".into(), Value::Int(500))])),
                    ("2".into(), Value::Map(vec![("percent".into(), Value::Int(5))])),
                ]),
            ),
        ]);
        let l = Ledger::from_value(&v);
        assert_eq!(l.mutes.len(), 1);
        assert_eq!(l.mutes["2"].until_ms, None);
        assert!(l.gains.is_empty());
        assert_eq!(Ledger::from_value(&Value::Null), Ledger::default());
    }

    #[test]
    fn expiry_and_lookup() {
        let mut l = Ledger::default();
        l.mutes.insert("1".into(), Mute { alias: "Bob".into(), until_ms: Some(5000) });
        l.mutes.insert("2".into(), Mute { alias: "Eve".into(), until_ms: None });
        assert!(l.expire(4999).is_empty());
        assert_eq!(l.muted_id_by_alias("BOB").as_deref(), Some("1"));
        assert_eq!(l.expire(5000), ["1"]);
        assert_eq!(l.muted_id_by_alias("bob"), None);
        assert_eq!(l.muted_id_by_alias("eve").as_deref(), Some("2"));
    }

    #[test]
    fn gains() {
        let mut l = Ledger::default();
        let config = vec![("LOUD".to_string(), 50), ("7".to_string(), 150)];
        assert_eq!(l.gain_of("1", "loud", &config), 50);
        assert_eq!(l.gain_of("7", "x", &config), 150);
        assert_eq!(l.gain_of("8", "x", &config), 100);
        l.set_gain("1", "loud", 120);
        assert_eq!(l.gain_of("1", "loud", &config), 120);
        l.set_gain("1", "loud", 100);
        assert!(l.gains.is_empty());
    }
}
