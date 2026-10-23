use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use bhmod_manifest::pack::{sha256_hex, write_zip};
use bhmod_manifest::package::{check, check_png, CheckOptions, DirSource, FileEntry, Report, Source, ZipSource};
use bhmod_manifest::Issue;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

const MANIFEST: &str = r#"
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
"#;

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend(w.to_be_bytes());
    v.extend(h.to_be_bytes());
    v.extend([8, 6, 0, 0, 0]);
    v
}

fn write(dir: &Path, path: &str, data: &[u8]) {
    let p = dir.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

fn mod_dir(manifest: &str, files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "mod.toml", manifest.as_bytes());
    for (p, data) in files {
        write(d.path(), p, data);
    }
    d
}

fn check_dir(d: &Path, index: bool) -> Report {
    let mut src = DirSource::open(d).unwrap();
    check(&mut src, CheckOptions { index })
}

fn msgs(issues: &[Issue]) -> Vec<String> {
    issues.iter().map(ToString::to_string).collect()
}

fn has(issues: &[Issue], needle: &str) {
    let m = msgs(issues);
    assert!(m.iter().any(|e| e.contains(needle)), "no `{needle}` in {m:?}");
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        if name.ends_with('/') {
            w.add_directory(name.trim_end_matches('/'), SimpleFileOptions::default()).unwrap();
        } else {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
    }
    w.finish().unwrap().into_inner()
}

fn check_zip(bytes: Vec<u8>) -> Report {
    let len = bytes.len() as u64;
    let mut src = ZipSource::open(Cursor::new(bytes), len).unwrap();
    check(&mut src, CheckOptions::default())
}

#[test]
fn valid_dir_ignores_build_files() {
    let d = mod_dir(
        MANIFEST,
        &[
            ("server/main.luau", b"print('hi')"),
            ("LICENSE", b"MIT"),
            ("src/lib.rs", b""),
            ("Cargo.toml", b""),
            ("server/.DS_Store", b""),
            ("assets/a.png", &png(16, 16)),
        ],
    );
    fs::create_dir(d.path().join("target")).unwrap();
    let mut src = DirSource::open(d.path()).unwrap();
    assert_eq!(src.ignored, vec!["Cargo.toml", "server/.DS_Store", "src", "target"]);
    assert_eq!(src.compressed_size(), None);
    let r = check(&mut src, CheckOptions { index: true });
    assert!(r.is_ok(), "{:?}", r.errors);
    assert!(r.warnings.is_empty());
    assert_eq!(r.file_count, 4);
    assert_eq!(r.manifest.unwrap().id, "demo");
}

#[test]
fn license_warning_and_index_error() {
    let d = mod_dir(MANIFEST, &[("server/main.luau", b"x")]);
    let r = check_dir(d.path(), false);
    assert!(r.is_ok());
    has(&r.warnings, "no LICENSE");
    let r = check_dir(d.path(), true);
    has(&r.errors, "no LICENSE");
}

#[test]
fn missing_bad_and_large_manifest() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "LICENSE", b"x");
    has(&check_dir(d.path(), false).errors, "mod.toml: missing");
    write(d.path(), "mod.toml", &vec![b'#'; 64 * 1024 + 1]);
    has(&check_dir(d.path(), false).errors, "larger than");
    write(d.path(), "mod.toml", b"\xff\xfe");
    has(&check_dir(d.path(), false).errors, "not valid UTF-8");
    write(d.path(), "mod.toml", b"schema = 2");
    has(&check_dir(d.path(), false).errors, "mod.toml");
    write(d.path(), "mod.toml", MANIFEST.replace("\"1.0.0\"", "\"x\"").as_bytes());
    has(&check_dir(d.path(), false).errors, "version");
}

#[test]
fn entry_points() {
    let d = mod_dir(MANIFEST, &[("LICENSE", b"x")]);
    has(&check_dir(d.path(), false).errors, "is not in the package");
    write(d.path(), "server/main.luau", b"  \n");
    has(&check_dir(d.path(), false).errors, "not a valid Lua entry point");
    write(d.path(), "server/main.luau", b"\xff");
    has(&check_dir(d.path(), false).errors, "not a valid Lua entry point");

    let wasm = MANIFEST.replace("\"lua\", path = \"server/main.luau\"", "\"wasm\", path = \"server/mod.wasm\"");
    for (bytes, ok) in [
        (&b"\0asm\x01\0\0\0rest"[..], true),
        (&b"\0asm\x0d\0\x01\0"[..], true),
        (&b"\0asm\x02\0\0\0"[..], false),
        (&b"\0asm"[..], false),
        (&b"MZ"[..], false),
    ] {
        let d = mod_dir(&wasm, &[("server/mod.wasm", bytes), ("LICENSE", b"x")]);
        assert_eq!(check_dir(d.path(), false).is_ok(), ok, "{bytes:?}");
    }

    let jvm = MANIFEST.replace("\"lua\", path = \"server/main.luau\"", "\"jvm\", path = \"server/mod.jar\"")
        + "\n[permissions]\nnative = true\n";
    let d = mod_dir(&jvm, &[("server/mod.jar", b"PK\x03\x04..."), ("LICENSE", b"x")]);
    assert!(check_dir(d.path(), false).is_ok());
    write(d.path(), "server/mod.jar", b"nope");
    has(&check_dir(d.path(), false).errors, "not a valid Jvm entry point");
}

#[test]
fn native_libraries_for_a_game_that_runs_armv7_only() {
    let native = MANIFEST.to_string() + "\n[permissions]\nnative = true\n";
    // only an x86 library: nothing of the mod would ever load
    let d = mod_dir(&native, &[("server/main.luau", b"x"), ("client/native/x86/libdemo.so", b"\x7fELF")]);
    let r = check_dir(d.path(), false);
    assert!(r.is_ok(), "{:?}", r.errors);
    has(&r.warnings, "armeabi-v7a code only");
    // with the armeabi-v7a library (and one for another ABI) no warning; a mod without libraries has none either
    let d = mod_dir(
        &native,
        &[
            ("server/main.luau", b"x"),
            ("client/native/armeabi-v7a/libdemo.so", b"\x7fELF"),
            ("client/native/arm64-v8a/libdemo.so", b"\x7fELF"),
        ],
    );
    assert!(!check_dir(d.path(), false).warnings.iter().any(|w| w.message.contains("armeabi-v7a")));
    let d = mod_dir(&native, &[("server/main.luau", b"x")]);
    assert!(!check_dir(d.path(), false).warnings.iter().any(|w| w.message.contains("armeabi-v7a")));
}

#[test]
fn unused_side_native_services_content() {
    let d = mod_dir(
        MANIFEST,
        &[("server/main.luau", b"x"), ("client/x.luau", b"x"), ("client/native/x86/libdemo.so", b"\x7fELF")],
    );
    let r = check_dir(d.path(), false);
    has(&r.warnings, "files in client/ are unused");
    has(&r.errors, "native libraries need permissions.native");

    let client_only = MANIFEST
        .replace("server = true", "client = true")
        .replace("server = {", "client = {")
        .replace("\"server/main.luau\"", "\"client/main.luau\"");
    let d = mod_dir(&client_only, &[("server/x.luau", b"x"), ("client/main.luau", b"x")]);
    let r = check_dir(d.path(), false);
    assert!(r.is_ok(), "{:?}", r.errors);
    has(&r.warnings, "files in server/ are unused");

    let svc = format!(
        "{MANIFEST}\n[provides]\nservices = [{{ name = \"svc\", version = \"1.0.0\", wit = \"wit/svc.wit\" }}]\n"
    );
    let d = mod_dir(&svc, &[("server/main.luau", b"x")]);
    has(&check_dir(d.path(), false).errors, "wit/svc.wit is not in the package");

    let content = MANIFEST.replace("server = true\n", "server = true\nclient = true\n")
        + "client = { runtime = \"lua\", path = \"client/main.luau\" }\n\n[content]\nblocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.png\" }]\n";
    let d =
        mod_dir(&content, &[("server/main.luau", b"x"), ("client/main.luau", b"x"), ("assets/big.png", &png(2048, 1))]);
    let r = check_dir(d.path(), false);
    has(&r.errors, "texture assets/lamp.png of `demo:lamp` is not in the package");
    has(&r.errors, "texture is 2048x1");
}

#[test]
fn png_header() {
    assert_eq!(check_png(&png(16, 32)), Ok((16, 32)));
    assert!(check_png(&png(0, 1)).is_err());
    assert!(check_png(&png(1, 0)).is_err());
    assert!(check_png(&png(1, 1025)).is_err());
    assert!(check_png(b"short").is_err());
    let mut bad = png(1, 1);
    bad[12] = b'X';
    assert!(check_png(&bad).is_err());
    bad = png(1, 1);
    bad[0] = 0;
    assert!(check_png(&bad).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_in_dir() {
    let d = mod_dir(MANIFEST, &[("server/main.luau", b"x")]);
    std::os::unix::fs::symlink("/etc/passwd", d.path().join("server/link")).unwrap();
    has(&check_dir(d.path(), false).errors, "server/link: symlinks are not allowed");
}

#[test]
fn open_missing_dir_fails() {
    assert!(DirSource::open(Path::new("/nonexistent/bhmod")).is_err());
}

#[test]
fn zip_rules() {
    let r = check_zip(zip_of(&[
        ("mod.toml", MANIFEST.as_bytes()),
        ("server/", b""),
        ("server/main.luau", b"x"),
        ("LICENSE", b"x"),
    ]));
    assert!(r.is_ok(), "{:?}", r.errors);

    let r = check_zip(zip_of(&[
        ("mod.toml", MANIFEST.as_bytes()),
        ("server/main.luau", b"x"),
        ("Server/Main.luau", b"x"),
        ("../evil", b"x"),
        ("src/lib.rs", b"x"),
        (".hidden/", b""),
    ]));
    has(&r.errors, "../evil");
    has(&r.errors, "duplicate name");
    has(&r.errors, "src/lib.rs: top-level directory not allowed");
    has(&r.errors, ".hidden/: hidden");
}

#[test]
fn zip_symlink_encrypted_and_method() {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.add_symlink("server/link", "/etc/passwd", SimpleFileOptions::default()).unwrap();
    w.start_file("server/b.luau", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
    w.write_all(b"x").unwrap();
    let mut bytes = w.finish().unwrap().into_inner();
    // Flip the compression method of the stored entry (local + central headers) to an unsupported one (bzip2 = 12).
    patch_method(&mut bytes, "server/b.luau", 12);
    let r = check_zip(bytes.clone());
    has(&r.errors, "server/link: symlinks are not allowed");
    has(&r.errors, "server/b.luau: only stored and deflate");
    // Mark it encrypted instead (general purpose flag bit 0).
    patch_method(&mut bytes, "server/b.luau", 0);
    patch_flag(&mut bytes, "server/b.luau", 1);
    has(&check_zip(bytes).errors, "server/b.luau: encrypted entries");
}

fn find_headers(bytes: &[u8], name: &str) -> Vec<(usize, bool)> {
    let mut out = Vec::new();
    for i in 0..bytes.len().saturating_sub(4) {
        let central = bytes[i..].starts_with(b"PK\x01\x02");
        let local = bytes[i..].starts_with(b"PK\x03\x04");
        if central || local {
            let (len_at, name_at) = if central { (28, 46) } else { (26, 30) };
            let n = u16::from_le_bytes([bytes[i + len_at], bytes[i + len_at + 1]]) as usize;
            if &bytes[i + name_at..i + name_at + n] == name.as_bytes() {
                out.push((i, central));
            }
        }
    }
    out
}

fn patch_method(bytes: &mut [u8], name: &str, method: u16) {
    for (i, central) in find_headers(bytes, name) {
        let at = i + if central { 10 } else { 8 };
        bytes[at..at + 2].copy_from_slice(&method.to_le_bytes());
    }
}

fn patch_flag(bytes: &mut [u8], name: &str, flag: u16) {
    for (i, central) in find_headers(bytes, name) {
        let at = i + if central { 8 } else { 6 };
        bytes[at..at + 2].copy_from_slice(&flag.to_le_bytes());
    }
}

#[test]
fn bad_zip_fails_to_open() {
    assert!(ZipSource::open(Cursor::new(b"not a zip".to_vec()), 9).is_err());
}

/// A fake source to hit the size and count limits without writing huge files.
struct Fake(Vec<FileEntry>, Option<u64>);

impl Source for Fake {
    fn files(&self) -> &[FileEntry] {
        &self.0
    }
    fn listing_issues(&self) -> &[Issue] {
        &[]
    }
    fn read(&mut self, _path: &str, _limit: u64) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other("boom"))
    }
    fn compressed_size(&self) -> Option<u64> {
        self.1
    }
}

#[test]
fn limits() {
    let many: Vec<FileEntry> = (0..4097).map(|i| FileEntry { path: format!("data/{i}"), size: 1 }).collect();
    has(&check(&mut Fake(many, Some(1)), CheckOptions::default()).errors, "4097 files");
    let big = vec![FileEntry { path: "data/a".into(), size: 128 * 1024 * 1024 + 1 }];
    has(&check(&mut Fake(big, Some(32 * 1024 * 1024 + 1)), CheckOptions::default()).errors, "bytes uncompressed");
    let big = vec![FileEntry { path: "data/a".into(), size: 1 }];
    has(&check(&mut Fake(big, Some(32 * 1024 * 1024 + 1)), CheckOptions::default()).errors, "bytes compressed");
    let unreadable = vec![FileEntry { path: "mod.toml".into(), size: 10 }];
    has(&check(&mut Fake(unreadable, None), CheckOptions::default()).errors, "cannot read: boom");
}

#[test]
fn pack_is_deterministic_and_valid() {
    let d =
        mod_dir(MANIFEST, &[("server/main.luau", b"print('hi')"), ("LICENSE", b"MIT"), ("assets/a.png", &png(4, 4))]);
    let build = || {
        let mut src = DirSource::open(d.path()).unwrap();
        write_zip(&mut src, Cursor::new(Vec::new())).unwrap().into_inner()
    };
    let a = build();
    let b = build();
    assert_eq!(a, b);
    assert_eq!(sha256_hex(&a), sha256_hex(&b));
    assert_eq!(sha256_hex(b"").len(), 64);
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let r = check_zip(a);
    assert!(r.is_ok(), "{:?}", r.errors);
    assert_eq!(r.file_count, 4);
}

#[test]
fn pack_reports_read_errors() {
    let mut fake = Fake(vec![FileEntry { path: "mod.toml".into(), size: 1 }], None);
    assert!(write_zip(&mut fake, Cursor::new(Vec::new())).is_err());
}
