//! The server half of 0xBC `SETTINGS_*` (API 0.3.4): schemas to admins who listed `settings`, sets with rights,
//! rate limit and validation, refreshes after changes from any source.

mod common;

use bhmod_core::netsettings::{schema, SettingsNet, RATE_LIMIT, RATE_WINDOW_MS};
use bhmod_core::types::Side;
use bhmod_core::{HostOptions, ModHost};
use bhmod_proto::{settings_error, Message, SettingsSet};
use common::*;
use serde_json::json;

const SETTINGS: &str =
    "[settings.radius]\ntype = \"int\"\ndefault = 32\nmin = 4\nmax = 64\nlabel = { en = \"Radius\" }\n\
[settings.volume]\ntype = \"int\"\ndefault = 100\nmin = 0\nmax = 200\nscope = \"client\"\nlabel = { en = \"Volume\" }\n\
[settings.only]\ntype = \"bool\"\ndefault = false\nlabel = { en = \"Only\" }\n";

fn host() -> (tempfile::TempDir, ModHost) {
    let t = tempfile::tempdir().unwrap();
    lua_mod(t.path(), "cfg", SETTINGS, "return {}\n");
    lua_mod(
        t.path(),
        "cli",
        "[settings.v]\ntype = \"int\"\ndefault = 1\nscope = \"client\"\nlabel = { en = \"V\" }\n",
        "return {}\n",
    );
    lua_mod(t.path(), "none", "", "return {}\n");
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(t.path(), Side::Server), b.clone());
    h.enable("W", "w");
    assert_eq!(h.mods().len(), 3, "{}", b.log_text());
    (t, h)
}

fn features() -> Vec<String> {
    vec!["settings".into()]
}

fn result(pkt: &[u8]) -> Result<(), (u8, Option<String>)> {
    match Message::decode(pkt).unwrap() {
        Message::SettingsResult(r) => r.outcome.map_err(|f| (f.code, f.message)),
        other => panic!("{other:?}"),
    }
}

fn set(mod_id: &str, key: &str, v: serde_json::Value) -> SettingsSet {
    SettingsSet { mod_id: mod_id.into(), key: key.into(), value: bhmod_core::settings::to_cbor(&v) }
}

#[test]
fn schemas_sets_and_refreshes() {
    let (_t, mut h) = host();
    let s = schema(&h, "cfg").unwrap();
    let form = bhmod_core::settings::from_cbor(&s.schema).unwrap();
    assert_eq!(form["fields"].as_array().unwrap().len(), 2, "server-scope fields only");
    assert_eq!(bhmod_core::settings::from_cbor(&s.values).unwrap(), json!({"radius": 32, "only": false}));
    assert!(schema(&h, "cli").is_none() && schema(&h, "none").is_none());

    let mut n = SettingsNet::default();
    assert!(n.join(&h, "plain", true, &[]).is_empty(), "did not list settings");
    assert!(n.join(&h, "player", false, &features()).is_empty());
    let pkts = n.join(&h, "admin", true, &features());
    assert_eq!(pkts.len(), 1);
    assert!(matches!(Message::decode(&pkts[0]).unwrap(), Message::SettingsSchema(s) if s.mod_id == "cfg"));

    assert_eq!(
        result(&n.receive_set(&mut h, "player", false, &set("cfg", "radius", json!(8)), 0)),
        Err((settings_error::NOT_PERMITTED, None))
    );
    assert_eq!(
        result(&n.receive_set(&mut h, "admin", true, &set("ghost", "radius", json!(8)), 0)),
        Err((settings_error::UNKNOWN_MOD, None))
    );
    assert_eq!(
        result(&n.receive_set(&mut h, "admin", true, &set("cli", "v", json!(8)), 0)),
        Err((settings_error::UNKNOWN_MOD, None))
    );
    assert_eq!(
        result(&n.receive_set(&mut h, "admin", true, &set("cfg", "volume", json!(8)), 0)),
        Err((settings_error::UNKNOWN_KEY, None))
    );
    assert_eq!(
        result(&n.receive_set(&mut h, "admin", true, &set("cfg", "nope", json!(8)), 0)),
        Err((settings_error::UNKNOWN_KEY, None))
    );
    let bad = result(&n.receive_set(&mut h, "admin", true, &set("cfg", "radius", json!(1)), 0));
    assert!(matches!(bad, Err((settings_error::INVALID_VALUE, Some(m))) if m.contains("at least")));
    assert_eq!(result(&n.receive_set(&mut h, "admin", true, &set("cfg", "radius", json!(8)), 0)), Ok(()));
    assert_eq!(h.settings_values("cfg").unwrap()["radius"], json!(8));

    // the change goes to the admins; `player` became an admin and gets everything
    let out = n.refresh(&mut h, &|p| p == "admin" || p == "player");
    assert_eq!(out.iter().filter(|(p, _)| p == "admin").count(), 1);
    assert_eq!(out.iter().filter(|(p, _)| p == "player").count(), 1);
    assert!(n.refresh(&mut h, &|p| p == "admin").is_empty(), "nothing changed since");
    // an admin who lost the right gets nothing; one who gets it back gets everything again
    h.set_setting("cfg", "only", &json!(true)).unwrap();
    let out = n.refresh(&mut h, &|_| false);
    assert!(out.is_empty());
    assert_eq!(n.refresh(&mut h, &|p| p == "admin").len(), 1);

    // the rate limit
    let w = 2 * RATE_WINDOW_MS;
    for i in 0..RATE_LIMIT {
        let r = result(&n.receive_set(&mut h, "admin", true, &set("cfg", "radius", json!(8 + i)), w));
        assert_eq!(r, Ok(()), "{i}");
    }
    assert_eq!(
        result(&n.receive_set(&mut h, "admin", true, &set("cfg", "radius", json!(9)), w + 1)),
        Err((settings_error::BUSY, None))
    );
    assert_eq!(
        result(&n.receive_set(&mut h, "admin", true, &set("cfg", "radius", json!(9)), w + RATE_WINDOW_MS)),
        Ok(())
    );

    n.leave("admin");
    n.leave("admin");
    assert!(n.refresh(&mut h, &|_| true).iter().all(|(p, _)| p == "player"));
}
