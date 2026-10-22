mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use bhmod_index::build::{build, format_utc, utc_now, BuildOptions};
use bhmod_index::repo::Repo;
use bhmod_index::{luau, wasm};
use common::*;

fn wat(text: &str) -> Vec<u8> {
    wat::parse_str(text).unwrap()
}

#[test]
fn wasm_checks() {
    assert!(wasm::check(b"nope").iter().any(|p| p.starts_with("invalid WASM")));
    assert!(wasm::check(&wat(r#"(module (import "bh" "call" (func)) (import "env" "abort" (func)))"#)).is_empty());
    let bad = wasm::check(&wat(r#"(module (import "evil" "x" (func)))"#));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].contains("evil.x"));
    let comp = r#"(component
        (import "blockheads:api/log@0.1.0" (instance))
        (import "wasi:sockets/tcp@0.2.0" (instance))
        (core module (import "evil" "x" (func))))"#;
    let p = wasm::check(&wat(comp));
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("wasi:sockets/tcp"));
}

fn script(dir: &Path, body: &str) -> std::path::PathBuf {
    let p = dir.join("luau-compile");
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn luau_checks() {
    let files = vec![
        ("a.luau".to_string(), b"print(1)".to_vec()),
        ("b.luau".to_string(), vec![0xff]),
        ("c.luau".to_string(), b"  \n".to_vec()),
    ];
    let p = luau::check(&files, None).unwrap();
    assert_eq!(p.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["b.luau", "c.luau"]);
    let d = tempfile::tempdir().unwrap();
    let ok = script(d.path(), "exit 0");
    assert_eq!(luau::check(&files, Some(&ok)).unwrap().len(), 2);
    let fail = script(d.path(), "echo \"$2:1: syntax error\" >&2; exit 1");
    let p = luau::check(&files, Some(&fail)).unwrap();
    assert!(p.iter().any(|(f, m)| f == "a.luau" && m.contains("a.luau:1: syntax error")), "{p:?}");
    assert!(luau::check(&files, Some(&d.path().join("missing"))).is_err());
}

#[test]
fn utc_formatting() {
    assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(format_utc(1_792_586_962), "2026-10-21T12:49:22Z");
    assert_eq!(utc_now().len(), 20);
}

fn opts() -> BuildOptions {
    BuildOptions { generated: "2026-10-21T00:00:00Z".into(), api_versions: vec!["0.1.0".into()], ..Default::default() }
}

#[test]
fn build_index() {
    let draft = entry("draft", "\"me\"", &format!("draft = true\n{}", version("1.0.0", b"x", "")));
    let mirrored = version("1.0.0", b"x", "mirror = \"https://m.example/x.bhmod\"");
    let d = repo(&[("demo", &entry("demo", "\"me\"", &mirrored)), ("draft", &draft)]);
    let r = Repo::load(d.path()).unwrap();
    let text = build(&r, &opts()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["mods"].as_array().unwrap().len(), 1);
    let o = BuildOptions { include_drafts: true, mirror_base: Some("https://gh.example/dl/".into()), ..opts() };
    let v: serde_json::Value = serde_json::from_str(&build(&r, &o).unwrap()).unwrap();
    assert_eq!(v["mods"][0]["versions"][0]["mirror"], "https://m.example/x.bhmod");
    assert_eq!(v["mods"][1]["versions"][0]["mirror"], "https://gh.example/dl/draft-1.0.0/draft-1.0.0.bhmod");
    assert!(v["mods"][1].get("draft").is_none());

    let no_size = entry("demo", "\"me\"", &version("1.0.0", b"x", "").replace("size = 1\n", ""));
    let d = repo(&[("demo", &no_size)]);
    write(d.path(), "mods/junk", b"");
    let errs = build(&Repo::load(d.path()).unwrap(), &opts()).unwrap_err();
    let msgs: Vec<String> = errs.iter().map(ToString::to_string).collect();
    assert!(msgs.iter().any(|m| m.contains("size is required")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.starts_with("mods/junk")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.starts_with("index.json")), "{msgs:?}");
}
