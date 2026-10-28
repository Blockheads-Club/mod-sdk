//! Mod settings at run time (API 0.3.3): the values of every mod's `[settings]` live in `mods.toml`
//! (`[<mod-id>.settings]`), are validated against the manifest, change through the admin (`/mod set`, the host app,
//! the in-game admin screen) or the mod itself (`settings.set`), and every change fires the custom event
//! `<mod-id>:settings-changed` with the payload `{"keys": [...]}`.
//!
//! This module has the helpers; the state lives in [`crate::ModHost`] (`settings_schema`, `settings_values`,
//! `set_setting`, `reload_settings`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value as Json};

use crate::config::FILE_NAME;
use crate::types::ApiError;
use crate::value::Value;

/// How often `poll` looks at `mods.toml` for edits.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// A hash of the bytes of `mods.toml` in `dir` (a missing file hashes like an empty one).
pub fn stamp(dir: &Path) -> u64 {
    let bytes = std::fs::read(dir.join(FILE_NAME)).unwrap_or_default();
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// A setting value as a [`Value`] (what mods get through `settings.get` / `all`).
pub fn to_value(j: &Json) -> Value {
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or_default()),
        },
        Json::String(s) => Value::Text(s.clone()),
        Json::Array(a) => Value::List(a.iter().map(to_value).collect()),
        Json::Object(o) => Value::Map(o.iter().map(|(k, v)| (k.clone(), to_value(v))).collect()),
    }
}

/// A [`Value`] from a mod as JSON for validation (bytes and non-finite floats have no setting type).
pub fn from_value(v: &Value) -> Result<Json, ApiError> {
    Ok(match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => Json::from(*i),
        Value::Float(f) => Json::Number(
            serde_json::Number::from_f64(*f)
                .ok_or_else(|| ApiError::InvalidArgument("a setting cannot be NaN or infinite".into()))?,
        ),
        Value::Text(t) => Json::String(t.clone()),
        Value::Bytes(_) => return Err(ApiError::InvalidArgument("a setting cannot hold bytes".into())),
        Value::List(l) => Json::Array(l.iter().map(from_value).collect::<Result<_, _>>()?),
        Value::Map(m) => {
            Json::Object(m.iter().map(|(k, v)| Ok((k.clone(), from_value(v)?))).collect::<Result<Map<_, _>, _>>()?)
        }
    })
}

/// The payload of `<mod-id>:settings-changed`.
pub fn changed_payload(keys: &[String]) -> Vec<u8> {
    Value::Map(vec![("keys".into(), Value::List(keys.iter().map(|k| Value::text(k)).collect()))]).encode()
}

/// One value for a chat line: JSON, cut to `max` bytes.
pub fn show(v: &Json, max: usize) -> String {
    let s = v.to_string();
    if s.len() <= max {
        return s;
    }
    format!("{}...", crate::host::truncate(&s, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn conversions() {
        let j = json!({ "a": [1, 2.5, "x", true, null], "b": {} });
        let v = to_value(&j);
        assert_eq!(from_value(&v).unwrap(), j);
        assert!(from_value(&Value::Bytes(vec![1])).is_err());
        assert!(from_value(&Value::List(vec![Value::Float(f64::NAN)])).is_err());
        assert!(from_value(&Value::Map(vec![("k".into(), Value::Float(f64::INFINITY))])).is_err());
        let p = Value::decode(&changed_payload(&["a".into()])).unwrap();
        assert_eq!(p.get("keys"), Some(&Value::List(vec![Value::text("a")])));
        assert_eq!(show(&json!("abcdef"), 4), "\"abc...");
        assert_eq!(show(&json!(1), 4), "1");
    }

    #[test]
    fn stamps() {
        let d = tempfile::tempdir().unwrap();
        let empty = stamp(d.path());
        std::fs::write(d.path().join(FILE_NAME), "").unwrap();
        assert_eq!(stamp(d.path()), empty);
        std::fs::write(d.path().join(FILE_NAME), "[a]\n").unwrap();
        assert_ne!(stamp(d.path()), empty);
    }
}
