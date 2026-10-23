use super::*;

fn manifest() -> Manifest {
    Manifest::parse(
        "schema = 1\nid = \"hats\"\nversion = \"1.0.0\"\nname = { en = \"Hats\" }\nauthors = [\"me\"]\nlicense = \"MIT\"\n\
         api = \"^0.1\"\n[sides]\nclient = true\n[entry]\nclient = { runtime = \"lua\", path = \"client/main.luau\" }\n\
         [content]\ncosmetics = [\n\
         { key = \"hats:crown\", slot = \"hat\", texture = \"assets/crown.png\", base = \"linen-cap\" },\n\
         { key = \"hats:any\", slot = \"hat\", texture = \"assets/any.png\" },\n\
         { key = \"hats:ember\", slot = \"hair\", texture = \"assets/ember.png\" },\n\
         { key = \"hats:suit\", slot = \"shirt\", texture = \"assets/suit.png\", texture-arm = \"assets/suit-arm.png\" },\n\
         { key = \"hats:unnumbered\", slot = \"shoes\", texture = \"assets/u.png\" },\n\
         ]\n",
    )
    .unwrap()
}

fn registry() -> BTreeMap<String, BTreeMap<String, u64>> {
    let ids = [("hats:crown", 1), ("hats:any", 2), ("hats:ember", 3), ("hats:suit", 4)];
    BTreeMap::from([("cosmetics".to_string(), ids.iter().map(|(k, i)| (k.to_string(), *i)).collect())])
}

fn msg(full: bool, rows: &[(&str, &[(&str, u64)])]) -> Cosmetics {
    Cosmetics {
        full,
        players: rows
            .iter()
            .map(|(p, s)| (p.to_string(), s.iter().map(|(slot, id)| (slot.to_string(), *id)).collect()))
            .collect(),
    }
}

#[test]
fn table_joins_manifests_and_registry() {
    let m = manifest();
    let t = Table::build(&registry(), [&m]);
    assert_eq!(t.all().count(), 4);
    assert!(!t.is_empty());
    let crown = t.get(1).unwrap();
    assert_eq!((crown.key.as_str(), crown.mod_id.as_str(), crown.slot.as_str()), ("hats:crown", "hats", "hat"));
    assert_eq!((crown.texture.as_str(), crown.base, crown.texture_arm.clone()), ("assets/crown.png", Some(115), None));
    assert_eq!(t.get(2).unwrap().base, None);
    assert_eq!(t.get(4).unwrap().texture_arm.as_deref(), Some("assets/suit-arm.png"));
    assert!(t.get(9).is_none());
    assert!(Table::build(&BTreeMap::new(), [&m]).is_empty());
    assert!(Table::build(&BTreeMap::from([("blocks".to_string(), BTreeMap::new())]), [&m]).is_empty());
}

#[test]
fn wearing_applies_full_and_delta() {
    let mut w = Wearing::default();
    assert!(w.is_empty());
    assert_eq!(w.apply(&msg(true, &[("a", &[("hat", 1)]), ("b", &[("hair", 3)])])), ["a", "b"]);
    assert_eq!(w.of("a").unwrap()["hat"], 1);
    // A delta changes only the listed players; the same set again is no change.
    assert!(w.apply(&msg(false, &[("a", &[("hat", 1)])])).is_empty());
    assert_eq!(w.apply(&msg(false, &[("a", &[("hat", 2)]), ("c", &[("hat", 1)])])), ["a", "c"]);
    assert_eq!(w.apply(&msg(false, &[("b", &[])])), ["b"]);
    assert!(w.of("b").is_none());
    // Clearing a player who wears nothing is no change.
    assert!(w.apply(&msg(false, &[("zz", &[])])).is_empty());
    // A full message drops the players it does not mention.
    assert_eq!(w.apply(&msg(true, &[("c", &[("hat", 1)])])), ["a"]);
    assert!(w.of("a").is_none());
    w.clear();
    assert!(w.is_empty());
}

#[test]
fn resolve_follows_the_worn_item() {
    let m = manifest();
    let t = Table::build(&registry(), [&m]);
    let mut w = Wearing::default();
    w.apply(&msg(
        true,
        &[("p", &[("hat", 1), ("hair", 3), ("shirt", 4)]), ("q", &[("hat", 2)]), ("r", &[("hat", 3), ("shoes", 9)])],
    ));
    let key = |d: Option<&CosmeticDef>| d.map(|d| d.key.clone());
    // Restricted to the linen cap.
    assert_eq!(key(resolve(&t, &w, "p", "hat", Some(115))), Some("hats:crown".into()));
    assert_eq!(key(resolve(&t, &w, "p", "hat", Some(142))), None);
    assert_eq!(key(resolve(&t, &w, "p", "hat", None)), None);
    // Without a base: any worn item, but not an empty slot.
    assert_eq!(key(resolve(&t, &w, "q", "hat", Some(142))), Some("hats:any".into()));
    assert_eq!(key(resolve(&t, &w, "q", "hat", None)), None);
    // Hair does not depend on the item.
    assert_eq!(key(resolve(&t, &w, "p", "hair", None)), Some("hats:ember".into()));
    // Not worn, unknown player, unknown id, id of another slot.
    assert_eq!(key(resolve(&t, &w, "p", "pants", Some(84))), None);
    assert_eq!(key(resolve(&t, &w, "nobody", "hat", Some(115))), None);
    assert_eq!(key(resolve(&t, &w, "r", "shoes", Some(124))), None);
    assert_eq!(key(resolve(&t, &w, "r", "hat", Some(115))), None);
    assert_eq!(key(resolve(&t, &w, "p", "shirt", Some(85))), Some("hats:suit".into()));
}
