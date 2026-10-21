use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let argv: Vec<OsString> = std::iter::once("bhmod").chain(args.iter().copied()).map(OsString::from).collect();
    let code = bhmod::run(argv, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn help_version_and_usage_errors() {
    let (code, out, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("check"));
    let (code, out, _) = run(&["--version"]);
    assert_eq!(code, 0);
    assert!(out.contains(env!("CARGO_PKG_VERSION")));
    let (code, _, err) = run(&["frobnicate"]);
    assert_eq!(code, 2);
    assert!(err.contains("unrecognized subcommand"));
}

#[test]
fn check_example_lua() {
    let (code, out, _) = run(&["check", "--index", s(&root().join("examples/hello-lua"))]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("ok: hello-lua 0.1.0"), "{out}");
}

#[test]
fn check_reports_errors_and_io_failures() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("mod.toml"), "schema = 1").unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    let (code, out, _) = run(&["check", s(d.path())]);
    assert_eq!(code, 1);
    assert!(out.contains("note: not packed: src"), "{out}");
    assert!(out.contains("warning: LICENSE"), "{out}");
    assert!(out.contains("error: mod.toml"), "{out}");
    assert!(out.contains("failed:"), "{out}");

    let (code, _, err) = run(&["check", "/nonexistent/bhmod-dir"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: "), "{err}");

    let bogus = d.path().join("bogus.bhmod");
    fs::write(&bogus, b"not a zip").unwrap();
    let (code, _, err) = run(&["check", s(&bogus)]);
    assert_eq!(code, 1);
    assert!(err.contains("error"), "{err}");
}

#[test]
fn pack_then_check_zip() {
    let d = tempfile::tempdir().unwrap();
    let out_file = d.path().join("hello.bhmod");
    let (code, out, _) = run(&["pack", s(&root().join("examples/hello-lua")), "-o", s(&out_file)]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("sha256 "), "{out}");
    let first = fs::read(&out_file).unwrap();
    let (code, out, _) = run(&["check", "--index", s(&out_file)]);
    assert_eq!(code, 0, "{out}");
    // Deterministic output.
    run(&["pack", s(&root().join("examples/hello-lua")), "-o", s(&out_file)]);
    assert_eq!(first, fs::read(&out_file).unwrap());
}

#[test]
fn pack_default_name_warnings_and_failures() {
    let d = tempfile::tempdir().unwrap();
    let (code, out, _) = run(&["new", "packme", "--dir", s(d.path())]);
    assert_eq!(code, 0, "{out}");
    let dir = d.path().join("packme");
    let cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(d.path()).unwrap();
    let (code, out, _) = run(&["pack", "packme"]);
    std::env::set_current_dir(cwd).unwrap();
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("warning: LICENSE"), "{out}");
    assert!(d.path().join("packme-0.1.0.bhmod").is_file());

    fs::remove_file(dir.join("server/main.luau")).unwrap();
    let (code, out, _) = run(&["pack", s(&dir), "-o", s(&d.path().join("x.bhmod"))]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("not in the package"), "{out}");

    let (code, _, err) = run(&["pack", "/nonexistent/bhmod-dir"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: "));
}

#[test]
fn pack_rejects_oversized_zip() {
    // 33 MiB of incompressible data passes the directory check (no compressed size there) but not the zip check.
    let d = tempfile::tempdir().unwrap();
    run(&["new", "big", "--dir", s(d.path())]);
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let noise: Vec<u8> = (0..33 * 1024 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    fs::create_dir_all(d.path().join("big/data")).unwrap();
    fs::write(d.path().join("big/data/noise.bin"), noise).unwrap();
    let (code, out, _) = run(&["pack", s(&d.path().join("big")), "-o", s(&d.path().join("big.bhmod"))]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("bytes compressed"), "{out}");
    assert!(!d.path().join("big.bhmod").exists());
}

#[test]
fn new_lua_and_rust_templates() {
    let d = tempfile::tempdir().unwrap();
    let (code, out, _) = run(&["new", "my-mod", "--lang", "lua", "--dir", s(d.path())]);
    assert_eq!(code, 0, "{out}");
    let (code, out, _) = run(&["check", s(&d.path().join("my-mod"))]);
    assert_eq!(code, 0, "{out}");

    let (code, out, _) = run(&["new", "9lives", "--lang", "rust", "--dir", s(d.path())]);
    assert_eq!(code, 0, "{out}");
    let cargo = fs::read_to_string(d.path().join("9lives/Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"mod-9lives\""));
    let build = fs::read_to_string(d.path().join("9lives/build.sh")).unwrap();
    assert!(build.contains("mod_9lives.wasm"));
    assert!(d.path().join("9lives/bindings/blockheads-api/worlds.wit").is_file());
    // Without the built component the check fails on the missing entry point.
    let (code, out, _) = run(&["check", s(&d.path().join("9lives"))]);
    assert_eq!(code, 1);
    assert!(out.contains("server/mod.wasm is not in the package"), "{out}");

    let (code, out, _) = run(&["new", "my-mod", "--dir", s(d.path())]);
    assert_eq!(code, 1);
    assert!(out.contains("already exists"));
    let (code, out, _) = run(&["new", "Bad Id", "--dir", s(d.path())]);
    assert_eq!(code, 1);
    assert!(out.contains("not a valid mod id"));
    let blocker = d.path().join("file");
    fs::write(&blocker, "").unwrap();
    let (code, _, err) = run(&["new", "nested", "--dir", s(&blocker)]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: "));
}

#[test]
fn wit_list_matches_directory() {
    let list = |p: PathBuf| {
        let mut v: Vec<String> =
            fs::read_dir(p).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    };
    let d = tempfile::tempdir().unwrap();
    run(&["new", "witcheck", "--lang", "rust", "--dir", s(d.path())]);
    assert_eq!(list(root().join("wit/blockheads-api")), list(d.path().join("witcheck/bindings/blockheads-api")));
}

#[test]
fn binary_runs() {
    let bin = env!("CARGO_BIN_EXE_bhmod");
    let out = Command::new(bin).arg("check").arg(root().join("examples/hello-lua")).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let out = Command::new(bin).arg("check").arg("/nonexistent").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}
