//! Helpers shared by the index tests.
#![allow(dead_code)]

use std::fs;
use std::io::Cursor;
use std::path::Path;

use bhmod_manifest::pack::{sha256_hex, write_zip};
use bhmod_manifest::package::DirSource;

pub const LUA_MANIFEST: &str = r#"
schema = 1
id = "demo"
version = "1.0.0"
name = { en = "Demo" }
authors = ["me"]
license = "MIT"
api = "^0.1"

[sides]
server = true

[entry]
server = { runtime = "lua", path = "server/main.luau" }

[permissions]
players = ["chat"]
"#;

pub fn write(dir: &Path, path: &str, data: &[u8]) {
    let p = dir.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

/// Packs files into a .bhmod (bytes).
pub fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    for (p, data) in files {
        write(d.path(), p, data);
    }
    let mut src = DirSource::open(d.path()).unwrap();
    write_zip(&mut src, Cursor::new(Vec::new())).unwrap().into_inner()
}

pub fn lua_package(manifest: &str) -> Vec<u8> {
    pack(&[("mod.toml", manifest.as_bytes()), ("LICENSE", b"MIT"), ("server/main.luau", b"print('hi')\n")])
}

/// A version table for `bytes`.
pub fn version(v: &str, bytes: &[u8], extra: &str) -> String {
    format!(
        "\n[[versions]]\nversion = \"{v}\"\nurl = \"https://example.com/demo-{v}.bhmod\"\nsha256 = \"{}\"\nsize = {}\nsides = [\"server\"]\napi = \"^0.1\"\npermissions = {{ players = [\"chat\"] }}\npublished = 2026-10-21\n{extra}",
        sha256_hex(bytes),
        bytes.len()
    )
}

pub fn entry(id: &str, owners: &str, versions: &str) -> String {
    format!("id = \"{id}\"\nname = \"{id}\"\nowners = [{owners}]\nrepo = \"https://example.com/{id}\"\n{versions}")
}

/// Writes an index repo: `(id, toml)` pairs.
pub fn repo(entries: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (id, text) in entries {
        write(d.path(), &format!("mods/{}/{id}.toml", &id[..1]), text.as_bytes());
    }
    d
}
