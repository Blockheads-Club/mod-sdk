use super::*;
use crate::registry::Wire;

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    out
}

const MANIFEST: &str = "schema = 1\nid = \"hats\"\nversion = \"1.0.0\"\nname = { en = \"Hats\" }\nauthors = [\"me\"]\n\
    license = \"MIT\"\napi = \"^0.1\"\n[sides]\nclient = true\n[entry]\n\
    client = { runtime = \"lua\", path = \"client/main.luau\" }\n[content]\ncosmetics = [\n\
    { key = \"hats:crown\", slot = \"hat\", texture = \"assets/crown.png\" },\n\
    { key = \"hats:suit\", slot = \"shirt\", texture = \"assets/suit.png\", texture-arm = \"assets/arm.png\" },\n\
    { key = \"hats:ghost\", slot = \"pants\", texture = \"assets/missing.png\" },\n]\n";

fn source_dir(dir: &Path) {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::create_dir_all(dir.join("client")).unwrap();
    std::fs::write(dir.join("mod.toml"), MANIFEST).unwrap();
    std::fs::write(dir.join("client/main.luau"), "-- x\n").unwrap();
    std::fs::write(dir.join("assets/crown.png"), png(3, 2, [255, 0, 0, 255])).unwrap();
    std::fs::write(dir.join("assets/suit.png"), png(2, 2, [0, 255, 0, 128])).unwrap();
    std::fs::write(dir.join("assets/arm.png"), png(1, 1, [0, 0, 255, 255])).unwrap();
}

fn registry() -> Wire {
    Wire::from([(
        "cosmetics".to_string(),
        BTreeMap::from([("hats:crown".to_string(), 1), ("hats:suit".to_string(), 2), ("hats:ghost".to_string(), 3)]),
    )])
}

fn zip_of(dir: &Path, out: &Path) {
    let mut src = DirSource::open(dir).unwrap();
    bhmod_manifest::pack::write_zip(&mut src, File::create(out).unwrap()).unwrap();
}

#[test]
fn loads_folders_and_zips() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("hats-src");
    source_dir(&dir);
    let zip = tmp.path().join("hats.bhmod");
    zip_of(&dir, &zip);
    for pkg in [dir, zip] {
        let mut c = Content::new();
        let problems = c.load(&registry(), std::slice::from_ref(&pkg));
        assert!(problems.is_empty(), "{problems:?}");
        assert!(c.is_active());
        assert_eq!(c.table().all().count(), 3);
        let t = c.texture(1, Part::Main).unwrap().unwrap();
        assert_eq!((t.wide, t.high, t.content_wide, t.content_high), (4, 2, 3, 2));
        assert_eq!(&t.rgba[..4], [255, 0, 0, 255]);
        // Premultiplied.
        assert_eq!(&c.texture(2, Part::Main).unwrap().unwrap().rgba[..4], [0, 128, 0, 128]);
        assert_eq!(c.texture(2, Part::Arm).unwrap().unwrap().wide, 1);
        // No arm texture declared, unknown id.
        assert!(c.texture(1, Part::Arm).unwrap().is_none());
        assert!(c.texture(1, Part::Arm).unwrap().is_none());
        assert!(c.texture(99, Part::Main).unwrap().is_none());
        // A missing file is reported once, then cached as absent.
        assert!(c.texture(3, Part::Main).unwrap_err().contains("assets/missing.png"));
        assert!(c.texture(3, Part::Main).unwrap().is_none());
        // Cached.
        assert!(Arc::ptr_eq(&c.texture(1, Part::Main).unwrap().unwrap(), &t));
    }
}

#[test]
fn reports_unreadable_packages() {
    let tmp = tempfile::tempdir().unwrap();
    let junk = tmp.path().join("junk.bhmod");
    std::fs::write(&junk, b"not a zip").unwrap();
    let bad_manifest = tmp.path().join("bad");
    std::fs::create_dir_all(&bad_manifest).unwrap();
    std::fs::write(bad_manifest.join("mod.toml"), "schema = 7").unwrap();
    let no_manifest = tmp.path().join("none");
    std::fs::create_dir_all(no_manifest.join("assets")).unwrap();
    let missing = tmp.path().join("missing.bhmod");
    let pkgs = [junk, bad_manifest, no_manifest, missing];
    let mut c = Content::new();
    let problems = c.load(&registry(), &pkgs);
    assert_eq!(problems.len(), 4, "{problems:?}");
    assert!(!c.is_active());
    assert!(problems[3].contains("missing.bhmod"));
}

#[test]
fn wearing_and_serial() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("hats-src");
    source_dir(&dir);
    let mut c = Content::new();
    c.load(&registry(), &[dir]);
    let msg = |full: bool, rows: &[(&str, &[(&str, u64)])]| Cosmetics {
        full,
        players: rows
            .iter()
            .map(|(p, s)| (p.to_string(), s.iter().map(|(a, b)| (a.to_string(), *b)).collect()))
            .collect(),
    };
    let base = c.serial();
    assert!(c.apply(&msg(true, &[])).is_empty());
    assert_eq!(c.serial(), base);
    assert_eq!(c.apply(&msg(true, &[("a", &[("hat", 1)])])), ["a"]);
    assert_eq!(c.serial(), base + 1);
    let r = c.resolve("a", "hat", None).unwrap();
    assert_eq!((r.def.key.as_str(), r.wear), ("hats:crown", Some(115)));
    assert!(c.resolve("b", "hat", None).is_none());
    c.clear();
    assert_eq!(c.serial(), base + 2);
    assert!(c.resolve("a", "hat", None).is_none());
    c.clear();
    assert_eq!(c.serial(), base + 2);
}

#[test]
fn texture_of_an_unloaded_package() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("hats-src");
    source_dir(&dir);
    let mut c = Content::new();
    c.load(&registry(), &[dir.clone()]);
    c.packages.clear();
    assert!(c.texture(1, Part::Main).unwrap_err().contains("not loaded"));
    let _ = dir;
}

#[test]
fn look_of_a_blockhead() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("hats-src");
    source_dir(&dir);
    let mut c = Content::new();
    c.load(&registry(), &[dir]);
    let wear = |rows: &[(&str, &[(&str, u64)])]| Cosmetics {
        full: true,
        players: rows
            .iter()
            .map(|(p, s)| (p.to_string(), s.iter().map(|(a, b)| (a.to_string(), *b)).collect()))
            .collect(),
    };
    assert!(c.look("a", [None; 4]).is_empty());
    c.apply(&wear(&[("a", &[("hat", 1), ("shirt", 2)])]));
    let look = c.look("a", [Some(124), None, None, Some(142)]);
    // Index 3 is the hat (worn: a santa hat, the cosmetic has no base), index 2 the shirt (empty: default shirt).
    assert_eq!(look.clothing, [None, None, Some((2, Some(85))), Some((1, None))]);
    assert_eq!(look.hair, None);
    assert!(!look.is_empty());
    assert_eq!(CLOTHING[3], "hat");
}

#[test]
fn wearing_survives_a_reload() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("hats-src");
    source_dir(&dir);
    let mut c = Content::new();
    let msg = Cosmetics {
        full: true,
        players: BTreeMap::from([("a".to_string(), BTreeMap::from([("hat".to_string(), 1)]))]),
    };
    // The server's message arrives before the packages are loaded.
    c.apply(&msg);
    assert!(!c.is_active());
    assert!(c.resolve("a", "hat", None).is_none());
    c.load(&registry(), &[dir]);
    assert!(c.is_active());
    assert_eq!(c.resolve("a", "hat", None).unwrap().def.key, "hats:crown");
}

#[test]
fn local_registry_for_developer_tests() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("hats-src");
    source_dir(&dir);
    let junk = tmp.path().join("junk.bhmod");
    std::fs::write(&junk, b"nope").unwrap();
    let wire = local_registry(&[dir.clone(), junk]);
    // Sorted keys, ids from 1.
    assert_eq!(
        wire["cosmetics"],
        BTreeMap::from([("hats:crown".into(), 1), ("hats:ghost".into(), 2), ("hats:suit".into(), 3)])
    );
    assert!(local_registry(&[]).is_empty());
    assert_eq!(read_manifest(&dir).unwrap().id, "hats");
}
