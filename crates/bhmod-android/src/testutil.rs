//! Test helpers.

use std::fs;

use bhmod_manifest::pack::write_zip;
use bhmod_manifest::package::DirSource;

/// A `.bhmod` with a client half: `entry` (a `client/` path) holding `src`; `perms` goes into `[permissions]`.
pub fn package(id: &str, version: &str, perms: &str, entry: &str, src: &str) -> Vec<u8> {
    package_files(id, version, perms, entry, src, &[])
}

/// Like [`package`] with extra files (`path`, bytes).
pub fn package_files(id: &str, version: &str, perms: &str, entry: &str, src: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    package_api(id, version, "^0.2", perms, entry, src, files)
}

/// Like [`package_files`] for API `api` (`perms` may go on with more tables after the permissions).
pub fn package_api(
    id: &str,
    version: &str,
    api: &str,
    perms: &str,
    entry: &str,
    src: &str,
    files: &[(&str, &[u8])],
) -> Vec<u8> {
    let t = tempfile::tempdir().unwrap();
    let rt = if entry.ends_with(".wasm") { "wasm" } else { "lua" };
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"{version}\"\nname = {{ en = \"Demo {id}\" }}\nauthors = [\"t\"]\n\
         license = \"MIT\"\napi = \"{api}\"\n\n[sides]\nclient = true\nserver = true\n\n[entry]\n\
         client = {{ runtime = \"{rt}\", path = \"{entry}\" }}\nserver = {{ runtime = \"lua\", path = \"server/main.luau\" }}\n\n\
         [permissions]\nchannels = true\n{perms}"
    );
    fs::write(t.path().join("mod.toml"), toml).unwrap();
    fs::create_dir_all(t.path().join("client")).unwrap();
    fs::create_dir_all(t.path().join("server")).unwrap();
    fs::write(t.path().join(entry), src).unwrap();
    fs::write(t.path().join("server/main.luau"), "return {}").unwrap();
    for (path, bytes) in files {
        let p = t.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
    if perms.contains("[provides]") {
        fs::create_dir_all(t.path().join("wit")).unwrap();
        fs::write(t.path().join("wit/hud.wit"), "package demo:hud@1.0.0;\n").unwrap();
    }
    let mut s = DirSource::open(t.path()).unwrap();
    write_zip(&mut s, std::io::Cursor::new(Vec::new())).unwrap().into_inner()
}

/// A `.bhmod` that declares one hat cosmetic (`<id>:crown`, a 2x2 red PNG, shown on the linen cap).
pub fn content_package(id: &str) -> Vec<u8> {
    let t = tempfile::tempdir().unwrap();
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = {{ en = \"Demo {id}\" }}\nauthors = [\"t\"]\n\
         license = \"MIT\"\napi = \"^0.2\"\n\n[sides]\nclient = true\nserver = true\n\n[entry]\n\
         client = {{ runtime = \"lua\", path = \"client/main.luau\" }}\nserver = {{ runtime = \"lua\", path = \"server/main.luau\" }}\n\n\
         [content]\ncosmetics = [{{ key = \"{id}:crown\", slot = \"hat\", base = \"linen-cap\", texture = \"assets/crown.png\" }}]\n"
    );
    fs::write(t.path().join("mod.toml"), toml).unwrap();
    for d in ["client", "server", "assets"] {
        fs::create_dir_all(t.path().join(d)).unwrap();
    }
    fs::write(t.path().join("client/main.luau"), "return {}").unwrap();
    fs::write(t.path().join("server/main.luau"), "return {}").unwrap();
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, 2, 2);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[255, 0, 0, 255].repeat(4)).unwrap();
    }
    fs::write(t.path().join("assets/crown.png"), png).unwrap();
    let mut s = DirSource::open(t.path()).unwrap();
    write_zip(&mut s, std::io::Cursor::new(Vec::new())).unwrap().into_inner()
}
