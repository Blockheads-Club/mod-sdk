//! `bhmod index` tests (in the cli test binary so `run` has one test instantiation).

use std::ffi::OsString;
use std::fs;
use std::path::Path;

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

fn write(dir: &Path, path: &str, data: &[u8]) {
    let p = dir.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

const ENTRY: &str = r#"id = "hello-lua"
name = "Hello"
owners = ["me"]
repo = "https://example.com/hello"
draft = DRAFT

[[versions]]
version = "0.1.0"
url = "https://example.com/hello-lua-0.1.0.bhmod"
sha256 = "SHA"
size = SIZE
sides = ["server"]
api = "^0.1"
permissions = { players = ["chat"] }
published = 2026-10-21
"#;

/// An index repo with the packed hello-lua example and its package folder.
fn setup(draft: bool) -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let pkg = d.path().join("packages/hello-lua-0.1.0.bhmod");
    fs::create_dir_all(pkg.parent().unwrap()).unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello-lua");
    let (code, out, _) = run(&["pack", s(&example), "-o", s(&pkg)]);
    assert_eq!(code, 0, "{out}");
    let sha = out.lines().find_map(|l| l.strip_prefix("sha256 ")).unwrap().to_string();
    let size = fs::metadata(&pkg).unwrap().len().to_string();
    let text = ENTRY.replace("SHA", &sha).replace("SIZE", &size).replace("DRAFT", &draft.to_string());
    write(d.path(), "repo/mods/h/hello-lua.toml", text.as_bytes());
    (d, text)
}

#[test]
fn validate_downloads_and_build() {
    let (d, _) = setup(false);
    let root = d.path();
    let (repo, pkgs) = (root.join("repo"), root.join("packages"));
    let (report, labels) = (root.join("report.md"), root.join("labels.txt"));
    let (code, out, _) = run(&[
        "index",
        "validate",
        "--repo",
        s(&repo),
        "--packages",
        s(&pkgs),
        "--author",
        "me",
        "--report",
        s(&report),
        "--labels",
        s(&labels),
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("label: new-mod") && out.contains("ok: 1 changed mod(s): hello-lua"), "{out}");
    assert!(fs::read_to_string(&report).unwrap().contains("**Passed.**"));
    assert_eq!(fs::read_to_string(&labels).unwrap(), "needs-security-review\nnew-mod\n");

    // Against itself as base: nothing changed.
    let (code, out, _) = run(&["index", "validate", "--repo", s(&repo), "--base", s(&repo)]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("warning: owners: owner check skipped") && out.contains("ok: 0 changed mod(s)\n"), "{out}");

    // No packages: fails.
    let (code, out, _) = run(&["index", "validate", "--repo", s(&repo), "--api", "0.1.0", "--allow-multiple"]);
    assert_eq!(code, 1);
    assert!(out.contains("error: hello-lua 0.1.0: package not checked") && out.contains("failed: 1 error(s)"), "{out}");

    let (code, out, _) = run(&["index", "downloads", "--repo", s(&repo)]);
    assert_eq!(code, 0);
    assert_eq!(out, "https://example.com/hello-lua-0.1.0.bhmod hello-lua-0.1.0.bhmod\n");
    let (_, out, _) = run(&["index", "downloads", "--repo", s(&repo), "--base", s(&repo)]);
    assert_eq!(out, "");

    let index = root.join("index.json");
    let (code, out, _) = run(&[
        "index",
        "build",
        "--repo",
        s(&repo),
        "-o",
        s(&index),
        "--generated",
        "2026-10-21T00:00:00Z",
        "--mirror-base",
        "https://example.com/dl",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("(1 mods,"), "{out}");
    let (code, out, _) = run(&["index", "build", "--repo", s(&repo), "-o", s(&index)]);
    assert_eq!(code, 0, "{out}");
    assert!(fs::read_to_string(&index).unwrap().contains("\"generated\""));

    write(&repo, "mods/junk", b"");
    let (code, out, _) = run(&["index", "build", "--repo", s(&repo), "-o", s(&index)]);
    assert_eq!(code, 1);
    assert!(out.contains("error: mods/junk") && out.contains("failed: 1 error(s)"), "{out}");
    let (code, _, err) = run(&["index", "validate", "--repo", s(&root.join("missing"))]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn drafts_are_left_out() {
    let (d, _) = setup(true);
    let index = d.path().join("index.json");
    let (code, out, _) = run(&["index", "build", "--repo", s(&d.path().join("repo")), "-o", s(&index)]);
    assert_eq!(code, 0);
    assert!(out.contains("(0 mods,"), "{out}");
    let (_, out, _) =
        run(&["index", "build", "--repo", s(&d.path().join("repo")), "-o", s(&index), "--include-drafts"]);
    assert!(out.contains("(1 mods,"), "{out}");
}
