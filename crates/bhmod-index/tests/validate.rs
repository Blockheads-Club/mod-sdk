mod common;

use std::fs;
use std::path::Path;

use bhmod_index::repo::Repo;
use bhmod_index::validate::{
    compare_manifest, downloads, new_versions, validate, ChangeKind, Options, LABEL_NEW_MOD, LABEL_SECURITY, LABEL_YANK,
};
use bhmod_index::Issue;
use bhmod_manifest::Manifest;
use common::*;
use semver::Version;

fn load(d: &tempfile::TempDir) -> Repo {
    Repo::load(d.path()).unwrap()
}

fn opts(packages: Option<&Path>) -> Options {
    Options {
        packages: packages.map(Path::to_path_buf),
        api_versions: vec![Version::new(0, 1, 0)],
        author: Some("Me".into()),
        ..Options::default()
    }
}

fn pkgdir(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (name, bytes) in files {
        fs::write(d.path().join(name), bytes).unwrap();
    }
    d
}

fn has(issues: &[Issue], needle: &str) {
    let m: Vec<String> = issues.iter().map(ToString::to_string).collect();
    assert!(m.iter().any(|x| x.contains(needle)), "missing `{needle}` in {m:#?}");
}

#[test]
fn new_mod_passes_with_package() {
    let pkg = lua_package(LUA_MANIFEST);
    let d = repo(&[("demo", &entry("demo", "\"me\"", &version("1.0.0", &pkg, "")))]);
    let p = pkgdir(&[("demo-1.0.0.bhmod", &pkg)]);
    let head = load(&d);
    let out = validate(&head, &opts(Some(p.path()))).unwrap();
    assert!(out.is_ok(), "{:#?}", out.errors);
    assert!(out.labels.contains(LABEL_NEW_MOD));
    // Every new mod gets a security review, even with harmless permissions.
    assert!(out.needs_review());
    assert_eq!(out.changes[0].kind, ChangeKind::New);
    let md = out.markdown();
    assert!(
        md.contains("**Passed.**")
            && md.contains("`players:chat`")
            && md.contains("Labels: `needs-security-review`, `new-mod`")
            && md.contains("every new mod"),
        "{md}"
    );
    assert_eq!(new_versions(&head, None).len(), 1);
    assert_eq!(downloads(&head, None), [("https://example.com/demo-1.0.0.bhmod".into(), "demo-1.0.0.bhmod".into())]);
}

#[test]
fn package_problems() {
    let manifest = LUA_MANIFEST.replace("players = [\"chat\"]", "players = [\"kick\"]\nnative = true");
    let pkg = pack(&[
        ("mod.toml", manifest.as_bytes()),
        ("server/main.luau", b"print(1)"),
        ("server/extra.luau", b" "),
        ("server/x.wasm", b"\0asm\x01\0\0\0\x99"),
        ("client/unused.luau", b"print(2)"),
    ]);
    let d = repo(&[("demo", &entry("demo", "\"other\"", &version("1.0.0", b"different", "")))]);
    let p = pkgdir(&[("demo-1.0.0.bhmod", &pkg)]);
    let out = validate(&load(&d), &opts(Some(p.path()))).unwrap();
    has(&out.errors, "the PR author `me` must be listed in owners");
    has(&out.errors, "sha256 is");
    has(&out.errors, "size is");
    has(&out.errors, "LICENSE");
    has(&out.errors, "permissions is `players:kick, native` in mod.toml");
    has(&out.errors, "server/x.wasm: invalid WASM");
    has(&out.errors, "server/extra.luau: empty Luau file");
    has(&out.warnings, "demo 1.0.0 client: files in client/ are unused");
    assert!(out.markdown().contains("**Failed.**"));

    let p = pkgdir(&[("demo-1.0.0.bhmod", b"not a zip")]);
    let d = repo(&[("demo", &entry("demo", "\"me\"", &version("1.0.0", b"not a zip", "")))]);
    has(&validate(&load(&d), &opts(Some(p.path()))).unwrap().errors, "not a valid .bhmod zip");
    has(&validate(&load(&d), &opts(Some(Path::new("/nonexistent")))).unwrap().errors, "not found (download");
    has(&validate(&load(&d), &opts(None)).unwrap().errors, "no --packages folder");
    let bad_manifest = pack(&[("mod.toml", b"schema = 1")]);
    let d = repo(&[("demo", &entry("demo", "\"me\"", &version("1.0.0", &bad_manifest, "")))]);
    let p = pkgdir(&[("demo-1.0.0.bhmod", &bad_manifest)]);
    has(&validate(&load(&d), &opts(Some(p.path()))).unwrap().errors, "mod.toml");
}

#[test]
fn version_rules() {
    let v = |ver: &str, extra: &str| version(ver, b"x", extra);
    let deps = "depends = { lib = \"^1\", gone = \"^1\", draftlib = \"^1\", bad = \"nope\" }\nsoft-depends = { opt = \"^1\", lib = \"^1\" }\nloader = \"=<>\"\n";
    let text = entry(
        "demo",
        "\"me\"",
        &format!("{}{}", v("1.0.0", "").replace("api = \"^0.1\"", "api = \"^9\""), v("0.9.0", deps)),
    )
    .replace("size = 1\n", "");
    let lib = entry("lib", "\"me\"", &format!("{}{}", v("1.0.0", "yanked = true"), v("1.1.0", "")));
    let draftlib = entry("draftlib", "\"me\"", &format!("draft = true\n{}", v("1.0.0", "")));
    let d = repo(&[("demo", &text), ("lib", &lib), ("draftlib", &draftlib)]);
    let mut o = opts(None);
    o.allow_multiple = true;
    let out = validate(&load(&d), &o).unwrap();
    for n in [
        "size is required",
        "matches no supported API version (0.1.0)",
        "version 0.9.0 must be greater than 1.0.0",
        "loader `=<>` is not a semver requirement",
        "dependency `nope` is not a semver requirement",
        "dependency `gone ^1` is not in the index",
        "dependency `draftlib ^1` is not in the index",
    ] {
        has(&out.errors, n);
    }
    has(&out.warnings, "soft dependency `opt ^1` is not in the index");
    assert!(!out.errors.iter().any(|e| e.message.contains("`lib ^1`")), "{:#?}", out.errors);
    // A draft mod may depend on a draft.
    let dd = entry("demo", "\"me\"", &format!("draft = true\n{}", v("1.0.0", "depends = { draftlib = \"^1\" }")));
    let d = repo(&[("demo", &dd), ("draftlib", &draftlib)]);
    let out = validate(&load(&d), &o).unwrap();
    assert!(!out.errors.iter().any(|e| e.message.contains("draftlib")), "{:#?}", out.errors);
}

#[test]
fn pr_rules_against_base() {
    let v = |ver: &str, extra: &str| version(ver, b"x", extra);
    let base_text = entry(
        "demo",
        "\"me\"",
        &format!("{}{}", v("1.0.0", "mirror = \"https://m.example/a.bhmod\""), v("1.1.0", "yanked = true")),
    );
    let base = repo(&[("demo", &base_text), ("other", &entry("other", "\"me\"", &v("1.0.0", "")))]);
    let mut o = opts(None);
    o.base = Some(load(&base));

    // Unchanged: nothing to do.
    let out = validate(&load(&base), &o).unwrap();
    assert!(out.is_ok() && out.changes.is_empty(), "{out:#?}");
    assert!(out.markdown().contains("No mod entries changed."));

    // Edits, un-yank, mirror change, removal and a second mod.
    let edited = entry(
        "demo",
        "\"me\", \"friend\"",
        &format!(
            "{}{}",
            v("1.0.0", "mirror = \"https://m.example/b.bhmod\"").replace("example.com", "evil.com"),
            v("1.1.0", "")
        ),
    );
    let head = repo(&[("demo", &edited), ("new", &entry("new", "\"me\"", &v("1.0.0", "")))]);
    let out = validate(&load(&head), &o).unwrap();
    has(&out.errors, "change one mod per pull request (this one changes demo, new, other)");
    has(&out.errors, "published versions cannot be edited");
    has(&out.errors, "mirror is set by CI");
    has(&out.errors, "a yanked version cannot be restored");
    has(&out.errors, "removing a mod is done by maintainers");
    assert!(out.changes.iter().any(|c| c.kind == ChangeKind::Removed));
    assert!(out.markdown().contains("(removed)"));
    assert!(out.markdown().contains("The owners list changed."));

    // Reordering / dropping a version.
    let head = repo(&[
        ("demo", &entry("demo", "\"me\"", &v("1.1.0", "yanked = true"))),
        ("other", &entry("other", "\"me\"", &v("1.0.0", ""))),
    ]);
    has(&validate(&load(&head), &o).unwrap().errors, "cannot be removed or reordered");

    // Not an owner.
    let mut stranger = opts(None);
    stranger.base = Some(load(&base));
    stranger.author = Some("stranger".into());
    let yank = base_text
        .replace("mirror = \"https://m.example/a.bhmod\"", "mirror = \"https://m.example/a.bhmod\"\nyanked = true");
    let head = repo(&[("demo", &yank), ("other", &entry("other", "\"me\"", &v("1.0.0", "")))]);
    let out = validate(&load(&head), &stranger).unwrap();
    has(&out.errors, "`stranger` is not an owner of `demo`");
    assert!(out.labels.contains(LABEL_YANK));
    assert!(out.markdown().contains("- Yanked: 1.0.0"));

    // No author: warning only.
    let mut anon = opts(None);
    anon.author = None;
    has(&validate(&load(&base), &anon).unwrap().warnings, "owner check skipped");
}

#[test]
fn permission_diff_and_labels() {
    let pkg_v = |ver: &str, perms: &str| {
        let m = LUA_MANIFEST.replace("1.0.0", ver).replace("players = [\"chat\"]", perms);
        let bytes = lua_package(&m);
        let entry_perms = match perms {
            "players = [\"chat\"]" => "{ players = [\"chat\"] }",
            "world = \"write\"" => "{ world = \"write\" }",
            _ => "{ native = true }",
        };
        (version(ver, &bytes, "").replace("{ players = [\"chat\"] }", entry_perms), bytes)
    };
    let (v1, b1) = pkg_v("1.0.0", "players = [\"chat\"]");
    let (v2, b2) = pkg_v("1.1.0", "world = \"write\"");
    let base = repo(&[("demo", &entry("demo", "\"me\"", &v1))]);
    let head = repo(&[("demo", &entry("demo", "\"me\"", &format!("{v1}{v2}")))]);
    let p = pkgdir(&[("demo-1.0.0.bhmod", &b1), ("demo-1.1.0.bhmod", &b2)]);
    let mut o = opts(Some(p.path()));
    o.base = Some(load(&base));
    let out = validate(&load(&head), &o).unwrap();
    assert!(out.is_ok(), "{:#?}", out.errors);
    assert!(out.needs_review());
    let diff = out.changes[0].permissions.clone().unwrap();
    assert_eq!(diff.from.as_deref(), Some("1.0.0"));
    assert_eq!(diff.added, ["world:write"]);
    assert_eq!(diff.removed, ["players:chat"]);
    let md = out.markdown();
    assert!(
        md.contains("`world:write` **(new)**") && md.contains("~~`players:chat`~~") && md.contains("must review"),
        "{md}"
    );

    // Update with the same permissions: no review.
    let (v2b, b2b) = pkg_v("1.1.0", "players = [\"chat\"]");
    let head = repo(&[("demo", &entry("demo", "\"me\"", &format!("{v1}{v2b}")))]);
    let p = pkgdir(&[("demo-1.1.0.bhmod", &b2b)]);
    let mut o = opts(Some(p.path()));
    o.base = Some(load(&base));
    let out = validate(&load(&head), &o).unwrap();
    assert!(out.is_ok(), "{:#?}", out.errors);
    assert!(!out.needs_review(), "{out:#?}");
    assert!(!out.markdown().contains("must review"));

    // New mods always need review, whatever they ask for.
    let (vn, bn) = pkg_v("1.0.0", "native = true");
    let head = repo(&[("demo", &entry("demo", "\"me\"", &vn))]);
    let p = pkgdir(&[("demo-1.0.0.bhmod", &bn)]);
    let out = validate(&load(&head), &opts(Some(p.path()))).unwrap();
    assert!(out.labels.contains(LABEL_SECURITY), "{out:#?}");
    assert!(out.markdown().contains("`native` - unsandboxed native code"));
    let net = entry(
        "demo",
        "\"me\"",
        &version("1.0.0", b"x", "").replace("{ players = [\"chat\"] }", "{ net-http = [\"a.com\"] }"),
    );
    let out = validate(&load(&repo(&[("demo", &net)])), &opts(None)).unwrap();
    assert!(out.needs_review());
    let none = entry("demo", "\"me\"", &version("1.0.0", b"x", "").replace("{ players = [\"chat\"] }", "{}"));
    let out = validate(&load(&repo(&[("demo", &none)])), &opts(None)).unwrap();
    assert!(out.needs_review());
    assert!(out.markdown().contains("- none"));
}

#[test]
fn manifest_comparison() {
    let m = Manifest::parse(
        &LUA_MANIFEST
            .replace("server = true", "server = true\nclient = true\nclient-required = false")
            .replace("server = { runtime = \"lua\", path = \"server/main.luau\" }", "server = { runtime = \"lua\", path = \"server/main.luau\" }\nclient = { runtime = \"lua\", path = \"client/main.luau\" }")
            .replace("[sides]", "loader = \">=0.1\"\n[sides]")
            .replace("[permissions]", "[dependencies]\nlib = \"^1\"\n[soft-dependencies]\nopt = { version = \"^2\", side = \"server\" }\n[permissions]"),
    )
    .unwrap();
    let text = entry("demo", "\"me\"", &version("1.0.0", b"x", "client-required = false\nloader = \">=0.1\"\ndepends = { lib = \"^1\" }\nsoft-depends = { opt = \"^2\" }\n").replace("sides = [\"server\"]", "sides = [\"client\", \"server\"]"));
    let d = repo(&[("demo", &text)]);
    let r = load(&d);
    let v = &r.entries["demo"].entry.versions[0];
    assert_eq!(compare_manifest(&m, "demo", v), Vec::<String>::new());
    let p = compare_manifest(&m, "other", &r.entries["demo"].entry.versions[0]);
    assert_eq!(p, ["id is `demo` in mod.toml but `other` in the entry"]);
}

#[test]
fn organisation_members_and_draft_releases() {
    let v = |ver: &str, url: &str| version(ver, b"x", "").replace("example.com", url);
    let draft = entry("demo", "\"Blockheads-Club\"", &format!("draft = true\n{}", v("1.0.0", "example.invalid")));
    let base = repo(&[("demo", &draft)]);
    let mut o = opts(None);
    o.base = Some(load(&base));

    // Publishing the draft rewrites its placeholder version; an outsider may not.
    let released = entry("demo", "\"Blockheads-Club\"", &v("1.0.0", "example.com"));
    let head = repo(&[("demo", &released), ("new", &entry("new", "\"Blockheads-Club\"", &v("1.0.0", "example.com")))]);
    let out = validate(&load(&head), &o).unwrap();
    has(&out.errors, "`me` is not an owner of `demo`; only Blockheads-Club may change it");
    has(&out.errors, "the PR author `me` must be listed in owners");

    // A member of the owning organisation may, and the draft's first release is reviewed like a new mod.
    o.member_of = vec!["blockheads-club".into()];
    o.allow_multiple = true;
    let out = validate(&load(&head), &o).unwrap();
    assert!(out.errors.iter().all(|e| e.message.contains("package not checked")), "{:#?}", out.errors);
    assert!(out.labels.contains(LABEL_NEW_MOD) && out.needs_review());
    let demo = out.changes.iter().find(|c| c.id == "demo").unwrap();
    assert_eq!(demo.added, ["1.0.0"]);
    assert_eq!(demo.permissions.as_ref().unwrap().from, None);
}
