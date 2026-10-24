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
        assert_eq!(&t.pixels[..4], [255, 0, 0, 255]);
        // Premultiplied.
        assert_eq!(&c.texture(2, Part::Main).unwrap().unwrap().pixels[..4], [0, 128, 0, 128]);
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
    c.load(&registry(), std::slice::from_ref(&dir));
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

#[test]
fn weapons_follow_the_held_item() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("arms-src");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::create_dir_all(dir.join("client")).unwrap();
    std::fs::write(dir.join("client/main.luau"), "-- x\n").unwrap();
    std::fs::write(dir.join("assets/blade.png"), png(6, 3, [255, 255, 0, 255])).unwrap();
    std::fs::write(
        dir.join("mod.toml"),
        "schema = 1\nid = \"arms\"\nversion = \"1.0.0\"\nname = { en = \"Arms\" }\nauthors = [\"me\"]\nlicense = \"MIT\"\n\
         api = \"^0.1\"\n[sides]\nclient = true\n[entry]\nclient = { runtime = \"lua\", path = \"client/main.luau\" }\n\
         [content]\ncosmetics = [{ key = \"arms:blade\", slot = \"weapon\", texture = \"assets/blade.png\", base = \"iron-sword\" }]\n",
    )
    .unwrap();
    let mut c = Content::new();
    assert!(!c.has_weapons());
    let reg = Wire::from([("cosmetics".to_string(), BTreeMap::from([("arms:blade".to_string(), 7)]))]);
    assert!(c.load(&reg, &[dir]).is_empty());
    assert!(c.has_weapons());
    c.apply(&Cosmetics { full: true, players: [("a".to_string(), [("weapon".to_string(), 7)].into())].into() });
    // the iron sword (item 68) of player a carries the skin; other items and other players do not
    assert_eq!(c.weapon("a", 68), Some(7));
    assert_eq!(c.weapon("a", 50), None);
    assert_eq!(c.weapon("b", 68), None);
    assert_eq!(c.worn_in("a", "weapon"), Some(7));
    assert_eq!((c.worn_in("a", "hat"), c.worn_in("b", "weapon")), (None, None));
    let g = c.texture(7, Part::Main).unwrap().unwrap();
    assert_eq!(g.uv(), [0.0, 0.0, 6.0 / 8.0, 3.0 / 4.0]);
}

const DECOR: &str = "schema = 1\nid = \"decor\"\nversion = \"1.0.0\"\nname = { en = \"Decor\" }\nauthors = [\"me\"]\n\
    license = \"MIT\"\napi = \"^0.3\"\n[sides]\nclient = true\n[entry]\n\
    client = { runtime = \"lua\", path = \"client/main.luau\" }\n[content]\n\
    items = [{ key = \"decor:lamp\", texture = \"assets/lamp.png\", name = { en = \"Brass lamp\" } },\n\
    { key = \"decor:gone\", texture = \"assets/gone.png\" }]\n\
    blocks = [{ key = \"decor:brick\", texture = \"assets/brick.png\", base = \"red-bricks\" }]\n";

fn decor_dir(dir: &Path) {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::create_dir_all(dir.join("client")).unwrap();
    std::fs::write(dir.join("mod.toml"), DECOR).unwrap();
    std::fs::write(dir.join("client/main.luau"), "-- x\n").unwrap();
    std::fs::write(dir.join("assets/lamp.png"), png(2, 2, [200, 100, 0, 255])).unwrap();
    std::fs::write(dir.join("assets/brick.png"), png(4, 4, [160, 40, 30, 255])).unwrap();
}

fn decor_registry() -> Wire {
    Wire::from([
        ("items".to_string(), BTreeMap::from([("decor:lamp".to_string(), 1107), ("decor:gone".to_string(), 1108)])),
        ("blocks".to_string(), BTreeMap::from([("decor:brick".to_string(), 1)])),
    ])
}

#[test]
fn items_and_blocks_have_names_and_textures() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("decor-src");
    decor_dir(&dir);
    let mut c = Content::new();
    assert!(c.load(&decor_registry(), &[dir]).is_empty());
    // Items and blocks alone do not turn the cosmetics on.
    assert!(!c.is_active());
    assert_eq!(c.items().all().count(), 2);
    assert_eq!(c.blocks().all().count(), 1);
    assert_eq!(c.item_name(1107, "en").as_deref(), Some("BRASS LAMP"));
    assert_eq!(c.item_name(5, "en"), None);
    let icon = c.item_texture(1107).unwrap().unwrap();
    assert_eq!((icon.wide, icon.high), (2, 2));
    assert!(Arc::ptr_eq(&c.item_texture(1107).unwrap().unwrap(), &icon));
    let block = c.block_texture(1).unwrap().unwrap();
    assert_eq!((block.wide, block.content_wide), (4, 4));
    assert!(c.block_texture(2).unwrap().is_none());
    assert!(c.item_texture(5).unwrap().is_none());
    // A missing file is reported once, then cached as absent.
    assert!(c.item_texture(1108).unwrap_err().contains("assets/gone.png"));
    assert!(c.item_texture(1108).unwrap().is_none());
    c.packages.clear();
    assert!(c.block_texture(1).unwrap().is_some());
    c.extra_cache.clear();
    assert!(c.block_texture(1).unwrap_err().contains("not loaded"));
}

#[test]
fn cell_pixels_of_items_and_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("decor-src");
    decor_dir(&dir);
    let mut c = Content::new();
    c.load(&decor_registry(), &[dir]);
    let icon = c.item_cell_pixels(1107, 2).unwrap().unwrap();
    assert_eq!(icon, [200, 100, 0, 255].repeat(4));
    let block = c.block_cell_pixels(1, 8).unwrap().unwrap();
    assert_eq!((block.len(), &block[..4]), (8 * 8 * 4, &[160u8, 40, 30, 255][..]));
    assert_eq!(c.item_cell_pixels(5, 2).unwrap(), None);
    assert_eq!(c.block_cell_pixels(5, 2).unwrap(), None);
    // A missing file and an unloaded package are errors.
    assert!(c.item_cell_pixels(1108, 2).unwrap_err().contains("assets/gone.png"));
    c.packages.clear();
    assert!(c.block_cell_pixels(1, 2).unwrap_err().contains("not loaded"));
}
