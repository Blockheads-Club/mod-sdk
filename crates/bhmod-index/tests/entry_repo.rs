mod common;

use std::fs;

use bhmod_index::entry::{permission_labels, schema_errors, Entry, Schema};
use bhmod_index::repo::Repo;
use bhmod_manifest::manifest::Permissions;
use common::*;
use serde_json::json;

#[test]
fn parse_ok_and_errors() {
    let text = entry("demo", "\"me\"", &version("1.0.0", b"x", ""));
    let (json, e) = Entry::parse("a.toml", &text).unwrap();
    assert_eq!(json["versions"][0]["published"], "2026-10-21");
    assert_eq!(e.version("1.0.0").unwrap().size, Some(1));
    assert!(e.version("2.0.0").is_none());
    let err = Entry::parse("a.toml", "id = ").unwrap_err();
    assert_eq!(err[0].path, "a.toml");
    let err = Entry::parse("a.toml", "id = \"demo\"\nweight = 1.5\nlist = [true]").unwrap_err();
    assert!(err.iter().any(|i| i.message.contains("weight")), "{err:?}");
}

#[test]
fn schema_errors_for_index() {
    assert!(!schema_errors(Schema::Index, &json!({ "schema": 2 })).is_empty());
    let ok = json!({ "schema": 1, "generated": "2026-10-21T00:00:00Z", "api": ["0.1.0"], "mods": [] });
    assert!(schema_errors(Schema::Index, &ok).is_empty());
}

#[test]
fn permission_labels_cover_everything() {
    let p: Permissions = serde_json::from_value(json!({
        "world": "write", "players": ["kick", "chat"], "net-http": ["b.com", "a.com"], "channels": true, "voice": true, "native": true
    }))
    .unwrap();
    assert_eq!(
        permission_labels(&p),
        [
            "world:write",
            "players:chat",
            "players:kick",
            "net-http:a.com",
            "net-http:b.com",
            "channels",
            "voice",
            "native"
        ]
    );
    let p: Permissions = serde_json::from_value(json!({ "world": "read" })).unwrap();
    assert_eq!(permission_labels(&p), ["world:read"]);
    assert!(permission_labels(&Permissions::default()).is_empty());
}

#[test]
fn repo_load() {
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(Repo::load(empty.path()).unwrap(), Repo::default());

    let good = entry("demo", "\"me\"", &version("1.0.0", b"x", ""));
    let d = repo(&[("demo", &good)]);
    let root = d.path();
    write(root, "mods/README.md", b"x");
    write(root, "mods/d/notes.txt", b"x");
    fs::create_dir_all(root.join("mods/d/sub")).unwrap();
    write(root, "mods/d/bad.toml", &[0xff, 0xfe]);
    write(root, "mods/d/broken.toml", b"id = ");
    write(root, "mods/d/other.toml", good.as_bytes());
    write(root, "mods/x/demo2.toml", entry("demo2", "\"me\"", &version("1.0.0", b"x", "")).as_bytes());
    write(root, "mods/de/demo3.toml", entry("demo3", "\"me\"", &version("1.0.0", b"x", "")).as_bytes());
    let r = Repo::load(root).unwrap();
    assert_eq!(r.entries.keys().collect::<Vec<_>>(), ["demo"]);
    assert_eq!(r.entries["demo"].path, "mods/d/demo.toml");
    let msgs: Vec<String> = r.issues.iter().map(ToString::to_string).collect();
    let want = [
        "mods/README.md: only folders",
        "mods/d/bad.toml: not valid UTF-8",
        "mods/d/broken.toml",
        "mods/d/notes.txt: expected",
        "mods/d/other.toml: file name says `other` but id is `demo`",
        "mods/d/sub: expected",
        "mods/de/demo3.toml: `demo3` belongs in mods/d/",
        "mods/x/demo2.toml: `demo2` belongs in mods/d/",
    ];
    for w in want {
        assert!(msgs.iter().any(|m| m.starts_with(w)), "missing {w}: {msgs:#?}");
    }
}
