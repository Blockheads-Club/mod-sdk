//! Test helpers.

use std::fs;

use bhmod_manifest::pack::write_zip;
use bhmod_manifest::package::DirSource;

/// A `.bhmod` with a client half: `entry` (a `client/` path) holding `src`; `perms` goes into `[permissions]`.
pub fn package(id: &str, version: &str, perms: &str, entry: &str, src: &str) -> Vec<u8> {
    let t = tempfile::tempdir().unwrap();
    let rt = if entry.ends_with(".wasm") { "wasm" } else { "lua" };
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"{version}\"\nname = {{ en = \"Demo {id}\" }}\nauthors = [\"t\"]\n\
         license = \"MIT\"\napi = \"^0.2\"\n\n[sides]\nclient = true\nserver = true\n\n[entry]\n\
         client = {{ runtime = \"{rt}\", path = \"{entry}\" }}\nserver = {{ runtime = \"lua\", path = \"server/main.luau\" }}\n\n\
         [permissions]\nchannels = true\n{perms}"
    );
    fs::write(t.path().join("mod.toml"), toml).unwrap();
    fs::create_dir_all(t.path().join("client")).unwrap();
    fs::create_dir_all(t.path().join("server")).unwrap();
    fs::write(t.path().join(entry), src).unwrap();
    fs::write(t.path().join("server/main.luau"), "return {}").unwrap();
    let mut s = DirSource::open(t.path()).unwrap();
    write_zip(&mut s, std::io::Cursor::new(Vec::new())).unwrap().into_inner()
}
