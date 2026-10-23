use super::*;

fn want(kind: Kind, keys: &[&str]) -> BTreeMap<Kind, BTreeSet<String>> {
    BTreeMap::from([(kind, keys.iter().map(|k| k.to_string()).collect())])
}

#[test]
fn kinds() {
    assert_eq!(Kind::ALL.map(Kind::name), ["blocks", "items", "cosmetics"]);
    assert_eq!(Kind::from_name("items"), Some(Kind::Items));
    assert_eq!(Kind::from_name("sounds"), None);
    assert_eq!((Kind::Items.first_id(), Kind::Cosmetics.first_id(), Kind::Blocks.first_id()), (1107, 1, 1));
    assert_eq!(Kind::Items.last_id(), 65535);
}

#[test]
fn keys() {
    assert!(is_key("hats:crown"));
    assert!(is_key("a-b:c_d"));
    assert!(!is_key("crown"));
    assert!(!is_key("h:crown"));
    assert!(!is_key("hats:"));
    assert!(!is_key("Hats:crown"));
}

#[test]
fn allocation_is_sorted_stable_and_never_reuses() {
    let mut r = Registry::new();
    assert!(r.is_empty());
    assert!(r.allocate(&want(Kind::Cosmetics, &["hats:zed", "hats:alpha"])).unwrap());
    assert_eq!(r.id(Kind::Cosmetics, "hats:alpha"), Some(1));
    assert_eq!(r.id(Kind::Cosmetics, "hats:zed"), Some(2));
    assert_eq!(r.key(Kind::Cosmetics, 2), Some("hats:zed"));
    assert_eq!(r.key(Kind::Cosmetics, 3), None);
    // Same keys again: nothing changes. A new key gets the next id, even though "hats:beta" sorts before "zed".
    assert!(!r.allocate(&want(Kind::Cosmetics, &["hats:alpha", "hats:zed"])).unwrap());
    assert!(r.allocate(&want(Kind::Cosmetics, &["hats:beta"])).unwrap());
    assert_eq!(r.id(Kind::Cosmetics, "hats:beta"), Some(3));
    // Items start at 1107; kinds are separate id spaces.
    r.allocate(&want(Kind::Items, &["hats:token"])).unwrap();
    assert_eq!(r.id(Kind::Items, "hats:token"), Some(1107));
    assert_eq!(r.id(Kind::Blocks, "hats:token"), None);
    assert_eq!(r.len(), 4);
}

#[test]
fn allocation_fills_holes_and_reports_exhaustion() {
    let mut r = Registry::parse("schema = 1\n[cosmetics]\n\"a-b:x\" = 2\n").unwrap();
    r.allocate(&want(Kind::Cosmetics, &["a-b:y"])).unwrap();
    assert_eq!(r.id(Kind::Cosmetics, "a-b:y"), Some(1));
    r.allocate(&want(Kind::Cosmetics, &["a-b:z"])).unwrap();
    assert_eq!(r.id(Kind::Cosmetics, "a-b:z"), Some(3));

    let full = format!("schema = 1\n[blocks]\n\"a-b:x\" = {}\n", u16::MAX);
    let mut r = Registry::parse(&full).unwrap();
    let before = r.clone();
    // The only free ids are below the taken one, so a second key still fits; a third one finds the range empty.
    assert!(r.allocate(&want(Kind::Blocks, &["a-b:y"])).unwrap());
    let mut keys = want(Kind::Blocks, &[]);
    keys.get_mut(&Kind::Blocks).unwrap().extend((0..70000).map(|i| format!("a-b:k{i}")));
    let err = r.clone().allocate(&keys).unwrap_err();
    assert_eq!(err, Error::Exhausted(Kind::Blocks));
    assert_eq!(err.to_string(), "no free blocks id left");
    assert_eq!(Error::Io("denied".into()).to_string(), "registry.toml: denied");
    // A failed allocation leaves the registry as it was.
    let mut r2 = before.clone();
    assert!(r2.allocate(&keys).is_err());
    assert_eq!(r2, before);
}

#[test]
fn wire_lists_only_wanted_keys() {
    let mut r = Registry::new();
    r.allocate(&want(Kind::Cosmetics, &["hats:a", "hats:b"])).unwrap();
    let wire = r.wire(&want(Kind::Cosmetics, &["hats:b", "other:gone"]));
    assert_eq!(wire, Wire::from([("cosmetics".to_string(), BTreeMap::from([("hats:b".to_string(), 2)]))]));
    assert!(r.wire(&want(Kind::Blocks, &["hats:a"])).is_empty());
    assert!(r.wire(&BTreeMap::new()).is_empty());
}

#[test]
fn toml_round_trip() {
    let mut r = Registry::new();
    r.allocate(&want(Kind::Cosmetics, &["hats:crown", "hats:beanie"])).unwrap();
    r.allocate(&want(Kind::Items, &["hats:token"])).unwrap();
    let text = r.to_toml();
    assert!(text.contains("schema = 1"));
    assert!(text.contains("[cosmetics]\n\"hats:beanie\" = 1\n\"hats:crown\" = 2\n"));
    assert_eq!(Registry::parse(&text).unwrap(), r);
    assert_eq!(Registry::parse("").unwrap(), Registry::new());
    assert!(!Registry::new().to_toml().contains('['));
}

#[test]
fn parse_rejects_bad_sidecars() {
    let bad = |text: &str, needle: &str| {
        let e = Registry::parse(text).unwrap_err().to_string();
        assert!(e.contains(needle), "{text:?}: {e}");
    };
    bad("this is not toml", "registry.toml");
    bad("schema = 2", "unsupported schema");
    bad("schema = \"1\"", "unsupported schema");
    bad("[sounds]\n", "unknown kind");
    bad("cosmetics = 3", "must be a table");
    bad("[cosmetics]\n\"a-b:x\" = 0", "range");
    bad("[items]\n\"a-b:x\" = 1106", "range");
    bad("[cosmetics]\n\"a-b:x\" = 70000", "range");
    bad("[cosmetics]\n\"a-b:x\" = \"1\"", "range");
    bad("[cosmetics]\n\"a-b:x\" = -1", "range");
    bad("[cosmetics]\n\"nokey\" = 1", "not a content key");
    bad("[cosmetics]\n\"a-b:x\" = 1\n\"a-b:y\" = 1", "used twice");
}

#[test]
fn files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mods/registry.toml");
    assert_eq!(Registry::read(&path).unwrap(), Registry::new());
    let mut r = Registry::new();
    r.allocate(&want(Kind::Cosmetics, &["hats:crown"])).unwrap();
    r.write(&path).unwrap();
    r.write(&path).unwrap();
    assert_eq!(Registry::read(&path).unwrap(), r);
    assert!(!dir.path().join("mods/registry.toml.tmp").exists());
    std::fs::write(&path, "garbage =").unwrap();
    assert!(matches!(Registry::read(&path), Err(Error::Format(_))));
    // A directory in the way: reads and writes fail with an I/O error.
    let blocked = dir.path().join("blocked");
    std::fs::create_dir_all(blocked.join("registry.toml")).unwrap();
    let e = Registry::read(&blocked.join("registry.toml")).unwrap_err();
    assert!(matches!(e, Error::Io(_)), "{e:?}");
    assert!(r.write(&blocked.join("registry.toml")).is_err());
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    assert!(r.write(&file.join("sub/registry.toml")).is_err());
    // No parent directory at all.
    assert!(r.write(Path::new("")).is_err());
}

#[test]
fn declared_from_manifests() {
    let m = |extra: &str| {
        Manifest::parse(&format!(
            "schema = 1\nid = \"hats\"\nversion = \"1.0.0\"\nname = {{ en = \"Hats\" }}\nauthors = [\"me\"]\n\
             license = \"MIT\"\napi = \"^0.1\"\n[sides]\nclient = true\n[entry]\nclient = {{ runtime = \"lua\", path = \"client/main.luau\" }}\n{extra}"
        ))
        .unwrap()
    };
    let a = m("[content]\ncosmetics = [{ key = \"hats:crown\", slot = \"hat\", texture = \"assets/c.png\" }]\n\
               items = [{ key = \"hats:token\", texture = \"assets/t.png\" }]\n\
               blocks = [{ key = \"hats:marble\", texture = \"assets/m.png\" }]\n");
    let none = m("");
    let d = declared([&a, &none]);
    assert_eq!(d.len(), 3);
    assert!(d[&Kind::Cosmetics].contains("hats:crown"));
    assert!(declared([&none]).is_empty());
    let mut r = Registry::new();
    assert!(r.allocate(&d).unwrap());
    assert_eq!(r.len(), 3);
}
