//! Test helpers: packages built in memory.

use std::fs;

use bhmod_manifest::manifest::Runtime;
use bhmod_manifest::pack::write_zip;
use bhmod_manifest::package::DirSource;

use crate::verify::Caps;

/// Loader 0.1.0 with Luau only.
pub fn caps() -> Caps {
    Caps { loader: semver::Version::new(0, 1, 0), runtimes: vec![Runtime::Lua] }
}

/// A `.bhmod` with a Luau client half. `perms` is appended to `[permissions]`; `variant` changes the shape:
/// `server-only`, `api9`, `loader9`, `loader0`, `wasm`, `invalid`.
pub fn package(id: &str, version: &str, perms: &str, variant: &str) -> Vec<u8> {
    let t = tempfile::tempdir().unwrap();
    let api = if variant == "api9" { "^9.0" } else { "^0.2" };
    let loader = match variant {
        "loader9" => "loader = \"^9.0\"\n",
        "loader0" => "loader = \"^0.1\"\n",
        _ => "",
    };
    let (sides, entry) = match variant {
        "server-only" => ("server = true", "server = { runtime = \"lua\", path = \"server/main.luau\" }"),
        "wasm" => ("client = true", "client = { runtime = \"wasm\", path = \"client/main.wasm\" }"),
        _ => ("client = true", "client = { runtime = \"lua\", path = \"client/main.luau\" }"),
    };
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"{version}\"\nname = {{ en = \"Demo {id}\" }}\nauthors = [\"t\"]\nlicense = \"MIT\"\n\
         api = \"{api}\"\n{loader}\n[sides]\n{sides}\n\n[entry]\n{entry}\n\n[permissions]\nchannels = true\n{perms}"
    );
    fs::write(t.path().join("mod.toml"), toml).unwrap();
    fs::create_dir_all(t.path().join("client")).unwrap();
    fs::create_dir_all(t.path().join("server")).unwrap();
    if variant != "invalid" {
        fs::write(t.path().join("client/main.luau"), "return {}").unwrap();
        fs::write(t.path().join("server/main.luau"), "return {}").unwrap();
        fs::write(t.path().join("client/main.wasm"), b"\0asm\x0d\0\x01\0").unwrap();
    }
    let mut src = DirSource::open(t.path()).unwrap();
    write_zip(&mut src, std::io::Cursor::new(Vec::new())).unwrap().into_inner()
}
