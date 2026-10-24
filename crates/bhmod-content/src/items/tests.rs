use super::*;

const MANIFEST: &str = "schema = 1\nid = \"decor\"\nversion = \"1.0.0\"\nname = { en = \"Decor\" }\nauthors = [\"me\"]\n\
    license = \"MIT\"\napi = \"^0.3\"\n[sides]\nclient = true\n[entry]\n\
    client = { runtime = \"lua\", path = \"client/main.luau\" }\n[content]\n\
    items = [\n\
    { key = \"decor:lamp\", texture = \"assets/lamp.png\", name = { en = \"Brass lamp\", tr = \"Pirinç lamba\" } },\n\
    { key = \"decor:wall-plate\", texture = \"assets/plate.png\" },\n\
    { key = \"decor:unlisted\", texture = \"assets/unlisted.png\" },\n]\n\
    blocks = [\n\
    { key = \"decor:warm-brick\", texture = \"assets/brick.png\", base = \"red-bricks\", name = { en = \"Warm brick\" } },\n\
    { key = \"decor:plain\", texture = \"assets/plain.png\" },\n\
    { key = \"decor:unlisted-block\", texture = \"assets/plain.png\" },\n]\n";

fn registry() -> Wire {
    Wire::from([
        (
            "items".to_string(),
            BTreeMap::from([("decor:lamp".to_string(), 1107), ("decor:wall-plate".to_string(), 1108)]),
        ),
        ("blocks".to_string(), BTreeMap::from([("decor:warm-brick".to_string(), 1), ("decor:plain".to_string(), 2)])),
    ])
}

fn manifest() -> Manifest {
    Manifest::parse(MANIFEST).unwrap()
}

#[test]
fn items_join_the_registry() {
    let m = manifest();
    let t = ItemTable::build(&registry(), [&m]);
    assert!(!t.is_empty());
    assert_eq!(t.all().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["decor:lamp", "decor:wall-plate"]);
    let lamp = t.get(1107).unwrap();
    assert_eq!((lamp.mod_id.as_str(), lamp.texture.as_str()), ("decor", "assets/lamp.png"));
    assert!(t.get(1109).is_none());
    assert!(ItemTable::build(&Wire::new(), [&m]).is_empty());
}

#[test]
fn names_follow_the_language_then_english_then_the_key() {
    let m = manifest();
    let t = ItemTable::build(&registry(), [&m]);
    assert_eq!(t.display_name(1107, "tr").as_deref(), Some("PIRINÇ LAMBA"));
    assert_eq!(t.display_name(1107, "de").as_deref(), Some("BRASS LAMP"));
    assert_eq!(t.display_name(1108, "en").as_deref(), Some("WALL PLATE"));
    assert_eq!(t.display_name(1, "en"), None);
    let only_tr: Localized = [("tr".to_string(), "x".to_string())].into();
    assert_eq!(pick_name(&only_tr, "de"), Some("x"));
    assert_eq!(pick_name(&Localized::new(), "en"), None);
}

#[test]
fn blocks_join_the_registry_and_resolve_their_base() {
    let m = manifest();
    let t = BlockTable::build(&registry(), [&m]);
    assert!(!t.is_empty());
    assert_eq!(t.all().count(), 2);
    let b = t.get(1).unwrap();
    assert_eq!((b.key.as_str(), b.base, b.texture.as_str()), ("decor:warm-brick", Some(11), "assets/brick.png"));
    assert_eq!(t.get(2).unwrap().base, None);
    assert!(t.get(3).is_none());
    assert!(BlockTable::build(&Wire::new(), [&m]).is_empty());
}
