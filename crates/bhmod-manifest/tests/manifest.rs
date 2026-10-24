use bhmod_manifest::manifest::{DepSide, Dependency, PlayerPermission, Runtime, WorldAccess};
use bhmod_manifest::{Issue, Manifest};

const BASE: &str = r#"
schema = 1
id = "demo"
version = "1.2.3"
name = { en = "Demo" }
authors = ["me"]
license = "MIT"
api = "^0.1"
"#;

const SERVER: &str = r#"
[sides]
server = true

[entry]
server = { runtime = "lua", path = "server/main.luau" }
"#;

const CLIENT: &str = r#"
[sides]
client = true

[entry]
client = { runtime = "lua", path = "client/main.luau" }
"#;

fn parse(extra: &str) -> Result<Manifest, Vec<Issue>> {
    Manifest::parse(&format!("{BASE}{extra}"))
}

/// Replaces `from` in BASE + SERVER, appends `extra`, and returns the issue messages.
fn errors_with(from: &str, to: &str, extra: &str) -> Vec<String> {
    let text = format!("{BASE}{SERVER}{extra}").replacen(from, to, 1);
    match Manifest::parse(&text) {
        Ok(m) => panic!("accepted: {m:?}"),
        Err(issues) => issues.iter().map(ToString::to_string).collect(),
    }
}

fn errors(extra: &str) -> Vec<String> {
    errors_with("", "", extra)
}

fn has(errs: &[String], needle: &str) {
    assert!(errs.iter().any(|e| e.contains(needle)), "no `{needle}` in {errs:?}");
}

#[test]
fn minimal_server_mod() {
    let m = parse(SERVER).unwrap();
    assert_eq!(m.id, "demo");
    assert_eq!(m.semver(), semver::Version::new(1, 2, 3));
    assert!(!m.client_required());
    assert_eq!(m.entry.server.as_ref().unwrap().runtime, Runtime::Lua);
    assert_eq!(m.permissions.world, WorldAccess::None);
    assert!(m.content.is_empty());
}

#[test]
fn full_manifest_and_accessors() {
    let m = parse(
        r#"
homepage = "https://example.com"
loader = ">=0.3"
description = { en = "A demo.", tr = "Bir demo." }

[sides]
server = true
client = true
client-required = false

[entry]
server = { runtime = "wasm", path = "server/mod.wasm" }
client = { runtime = "jvm", path = "client/mod.jar" }

[dependencies]
economy = { version = "^1.1", side = "server" }
other = { version = "1" }
plain = "^2"

[soft-dependencies]
maps = "*"

[permissions]
world = "read"
players = ["chat", "kick", "ban", "inventory", "health", "teleport"]
net-http = ["api.example.com"]
channels = true
native = true

[provides]
services = [{ name = "demo-svc", version = "1.0.0", wit = "wit/demo.wit" }]
"#,
    )
    .unwrap();
    assert!(!m.client_required());
    assert_eq!(m.dependencies["economy"], Dependency::Detailed { version: "^1.1".into(), side: DepSide::Server });
    assert_eq!(m.dependencies["other"].version(), "1");
    assert_eq!(m.dependencies["plain"].version(), "^2");
    assert!(matches!(m.dependencies["other"], Dependency::Detailed { side: DepSide::Both, .. }));
    assert_eq!(m.permissions.players[0], PlayerPermission::Chat);
    assert_eq!(Runtime::Jvm.extension(), ".jar");
}

#[test]
fn content_forces_client_required() {
    let m = parse(&format!(
        "{CLIENT}\n[content]\nblocks = [{{ key = \"demo:lamp\", texture = \"assets/lamp.png\", hardness = 1.5 }}]\n\
         items = [{{ key = \"demo:gem\", texture = \"assets/gem.png\", name = {{ en = \"Gem\" }} }}]\n\
         cosmetics = [{{ key = \"demo:hat\", slot = \"hat\", texture = \"assets/hat.png\" }}]\n"
    ))
    .unwrap();
    assert!(m.client_required());
    assert_eq!(m.content.textures().len(), 3);
    let m = parse(&CLIENT.replace("client = true", "client = true\nclient-required = true")).unwrap();
    assert!(m.client_required());
    assert!(parse(CLIENT).unwrap().client_required());
}

#[test]
fn variant_block_declarations() {
    let m = parse(&format!(
        "{CLIENT}\n[content]\nblocks = [{{ key = \"demo:brick\", texture = \"assets/b.png\", base = \"red-bricks\", \
         drops = \"demo:gem\", name = {{ en = \"Brick\" }} }}]\n\
         items = [{{ key = \"demo:gem\", texture = \"assets/gem.png\" }}]\n"
    ))
    .unwrap();
    let b = &m.content.blocks[0];
    assert_eq!(
        (b.base.as_deref(), b.drops.as_deref(), b.name["en"].as_str()),
        (Some("red-bricks"), Some("demo:gem"), "Brick")
    );
    use bhmod_manifest::content::{base_tile, base_tile_of, BASE_TILES};
    assert_eq!(base_tile("red-bricks").map(|t| (t.tile_type, t.item_type)), Some((11, 0x402)));
    assert_eq!(base_tile_of(11).map(|t| t.slug), Some("red-bricks"));
    assert!(base_tile("lava").is_none() && base_tile_of(31).is_none());
    // Slugs and types are unique.
    for (i, t) in BASE_TILES.iter().enumerate() {
        assert!(BASE_TILES[i + 1..].iter().all(|o| o.slug != t.slug && o.tile_type != t.tile_type));
    }
}

#[test]
fn toml_errors() {
    let errs = Manifest::parse("this is not toml").unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].path, "mod.toml");
    has(&errors("color = \"red\""), "unknown field");
}

#[test]
fn top_level_fields() {
    has(&errors_with("schema = 1", "schema = 7", ""), "unsupported schema 7");
    has(&errors_with("\"demo\"", "\"D\"", ""), "id: must match");
    has(&errors_with("\"1.2.3\"", "\"1.2\"", ""), "version: not a semver");
    has(&errors_with("{ en = \"Demo\" }", "{ tr = \"Demo\" }", ""), "name: needs an `en`");
    has(&errors_with("{ en = \"Demo\" }", "{ en = \"Demo\", english = \"x\" }", ""), "not a language code");
    has(&errors_with("{ en = \"Demo\" }", "{ en = \" \" }", ""), "text must be 1-64");
    has(&errors_with("[\"me\"]", "[]", ""), "authors");
    has(&errors_with("[\"me\"]", "[\"me\", \"\"]", ""), "authors");
    has(&errors_with("\"MIT\"", "\"\"", ""), "license");
    has(&errors_with("\"MIT\"", &format!("\"{}\"", "x".repeat(129)), ""), "license");
    has(&errors_with("license", "homepage = \"ftp://x\"\nlicense", ""), "homepage");
    has(&errors_with("\"^0.1\"", "\"nope\"", ""), "api: not a semver");
    has(&errors_with("\"^0.1\"", "\"^1.0\"", ""), "matches none of the API versions");
    has(&errors_with("license", "loader = \"what\"\nlicense", ""), "loader");
    has(&errors_with("license", "description = { tr = \"x\" }\nlicense", ""), "description: needs an `en`");
}

#[test]
fn sides_and_entries() {
    has(&parse("[sides]\nserver = false\n").unwrap_err().iter().map(ToString::to_string).collect::<Vec<_>>(), "sides");
    let e = |s: &str| parse(s).unwrap_err().iter().map(ToString::to_string).collect::<Vec<_>>();
    has(&e("[sides]\nserver = true\n"), "missing entry point for side server");
    has(
        &e(&format!("{SERVER}client = {{ runtime = \"lua\", path = \"client/main.luau\" }}\n")),
        "sides.client is false",
    );
    has(&errors_with("server/main.luau", "/abs/main.luau", ""), "absolute path");
    has(&errors_with("server/main.luau", "client/main.luau", ""), "inside server/");
    has(&errors_with("server/main.luau", "server/native/main.luau", ""), "inside server/");
    has(&errors_with("server/main.luau", "server/main.lua", ""), "needs a .luau file");
    has(
        &errors_with("\"lua\", path = \"server/main.luau\"", "\"jvm\", path = \"server/mod.jar\"", ""),
        "needs permissions.native",
    );
}

#[test]
fn dependencies() {
    has(&errors("[dependencies]\nBad = \"1\"\n"), "not a valid mod id");
    has(&errors("[dependencies]\ndemo = \"1\"\n"), "cannot depend on itself");
    has(&errors("[dependencies]\neco = \"one\"\n"), "not a semver requirement");
    has(&errors("[dependencies]\neco = \"1\"\n[soft-dependencies]\neco = \"1\"\n"), "also listed in dependencies");
    has(&errors("[soft-dependencies]\neco = { version = \"one\", side = \"client\" }\n"), "soft-dependencies.eco");
}

#[test]
fn permissions() {
    has(&errors("[permissions]\nnet-http = [\"Bad Host\"]\n"), "not a lowercase host name");
    has(&errors("[permissions]\nplayers = [\"chat\", \"chat\"]\n"), "duplicate entries");
    let text = format!("{BASE}{CLIENT}[permissions]\nnet-http = [\"a.com\"]\n");
    let errs: Vec<String> = Manifest::parse(&text).unwrap_err().iter().map(ToString::to_string).collect();
    has(&errs, "only available to server mods");
}

#[test]
fn provides() {
    let svc = |s: &str| errors(&format!("[provides]\nservices = [{s}]\n"));
    has(&svc("{ name = \"X\", version = \"1.0.0\", wit = \"wit/x.wit\" }"), "name must match");
    has(&svc("{ name = \"xx\", version = \"1\", wit = \"wit/x.wit\" }"), "version is not semver");
    has(&svc("{ name = \"xx\", version = \"1.0.0\", wit = \"x.wit\" }"), "wit must be");
    has(&svc("{ name = \"xx\", version = \"1.0.0\", wit = \"wit/x.txt\" }"), "wit must be");
    has(&svc("{ name = \"xx\", version = \"1.0.0\", wit = \"wit/../x.wit\" }"), "wit must be");
    has(
        &svc("{ name = \"xx\", version = \"1.0.0\", wit = \"wit/x.wit\" }, { name = \"xx\", version = \"1.0.0\", wit = \"wit/x.wit\" }"),
        "duplicate service name",
    );
    let remote = |extra: &str| svc(&format!("{{ name = \"xx\", version = \"1.0.0\", wit = \"wit/x.wit\"{extra} }}"));
    has(&remote(", remote-methods = [\"a\"]"), "remote-methods needs remote = true");
    has(&remote(", remote = true, remote-methods = [\"a\", \"a\"]"), "duplicate remote-methods");
    has(&remote(", remote = true, remote-methods = [\"a b\"]"), "remote-methods must match");
    has(&remote(", remote = true, remote-methods = [\"\"]"), "remote-methods must match");
    let long = "a".repeat(65);
    has(&remote(&format!(", remote = true, remote-methods = [\"{long}\"]")), "remote-methods must match");
    let ok = Manifest::parse(&format!(
        "{BASE}{SERVER}[provides]\nservices = [{{ name = \"xx\", version = \"1.0.0\", wit = \"wit/x.wit\", remote = true, remote-methods = [\"get_1.x-y\"] }}]\n"
    ))
    .unwrap();
    assert!(ok.provides.services[0].remote);
    assert_eq!(ok.provides.services[0].remote_methods, ["get_1.x-y"]);
}

#[test]
fn content() {
    let content = |sides: &str, c: &str| -> Vec<String> {
        match Manifest::parse(&format!("{BASE}{sides}\n[content]\n{c}\n")) {
            Ok(m) => panic!("accepted {m:?}"),
            Err(i) => i.iter().map(ToString::to_string).collect(),
        }
    };
    let lamp = "blocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.png\" }]";
    has(&content(SERVER, lamp), "content needs a client half");
    has(
        &content(&CLIENT.replace("client = true", "client = true\nclient-required = false"), lamp),
        "content needs a client half",
    );
    has(
        &content(CLIENT, "blocks = [{ key = \"other:lamp\", texture = \"assets/lamp.png\" }]"),
        "must be `demo:<name>`",
    );
    has(&content(CLIENT, "blocks = [{ key = \"demo:\", texture = \"assets/lamp.png\" }]"), "must be `demo:<name>`");
    has(&content(CLIENT, "blocks = [{ key = \"demolamp\", texture = \"assets/lamp.png\" }]"), "must be `demo:<name>`");
    has(
        &content(CLIENT, &format!("{lamp}\nitems = [{{ key = \"demo:lamp\", texture = \"assets/lamp.png\" }}]")),
        "duplicate key",
    );
    has(&content(CLIENT, "blocks = [{ key = \"demo:lamp\", texture = \"lamp.png\" }]"), "must be a .png under assets/");
    has(
        &content(CLIENT, "blocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.jpg\" }]"),
        "must be a .png under assets/",
    );
    has(
        &content(CLIENT, "blocks = [{ key = \"demo:lamp\", texture = \"assets//lamp.png\" }]"),
        "must be a .png under assets/",
    );
    has(
        &content(CLIENT, "blocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.png\", hardness = -1.0 }]"),
        "hardness",
    );
    has(
        &content(CLIENT, "blocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.png\", hardness = nan }]"),
        "hardness",
    );
    has(
        &content(CLIENT, "items = [{ key = \"demo:gem\", texture = \"assets/gem.png\", name = { tr = \"Taş\" } }]"),
        "needs an `en`",
    );
    has(
        &content(CLIENT, "blocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.png\", base = \"lava\" }]"),
        "is not a known vanilla block",
    );
    has(
        &content(CLIENT, "blocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.png\", drops = \"demo:gem\" }]"),
        "is not an item of this mod",
    );
    has(
        &content(
            CLIENT,
            "blocks = [{ key = \"demo:lamp\", texture = \"assets/lamp.png\", name = { tr = \"Lamba\" } }]",
        ),
        "needs an `en`",
    );
    has(
        &content(CLIENT, "cosmetics = [{ key = \"demo:hat\", slot = \"head\", texture = \"assets/hat.png\" }]"),
        "slot must be one of",
    );
}

fn cosmetic(fields: &str) -> String {
    format!("{CLIENT}\n[content]\ncosmetics = [{{ key = \"demo:c\", texture = \"assets/c.png\", {fields} }}]\n")
}

fn cosmetic_errors(fields: &str) -> Vec<String> {
    match parse(&cosmetic(fields)) {
        Ok(m) => panic!("accepted {m:?}"),
        Err(i) => i.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn cosmetic_rules() {
    let m = parse(&cosmetic(
        "slot = \"shirt\", texture-arm = \"assets/arm.png\", base = \"linen-shirt\", name = { en = \"C\" }",
    ))
    .unwrap();
    let c = &m.content.cosmetics[0];
    assert_eq!((c.base.as_deref(), c.texture_arm.as_deref()), (Some("linen-shirt"), Some("assets/arm.png")));
    // The arm texture counts as a texture of the declaration.
    assert_eq!(m.content.textures(), vec![("demo:c", "assets/c.png"), ("demo:c", "assets/arm.png")]);
    assert_eq!(m.content.keys().collect::<Vec<_>>(), vec!["demo:c"]);
    assert!(parse(&cosmetic("slot = \"hair\"")).is_ok());
    assert!(parse(&cosmetic("slot = \"weapon\", base = \"iron-sword\"")).is_ok());
    has(&cosmetic_errors("slot = \"hat\", texture-arm = \"assets/arm.png\""), "only for shirt");
    has(&cosmetic_errors("slot = \"shirt\", texture-arm = \"arm.png\""), "texture-arm must be a .png");
    has(&cosmetic_errors("slot = \"hat\", base = \"nope\""), "not a known vanilla item");
    has(&cosmetic_errors("slot = \"hat\", base = \"linen-pants\""), "belongs to the pants slot");
    has(&cosmetic_errors("slot = \"weapon\""), "needs a base");
    has(&cosmetic_errors("slot = \"hat\", name = { tr = \"x\" }"), "needs an `en`");
}

#[test]
fn base_item_table() {
    use bhmod_manifest::content::{base_item, base_item_for, BASE_ITEMS, SLOTS};
    assert_eq!(base_item("linen-cap").unwrap().item_type, 115);
    assert_eq!(base_item_for("weapon", 16).unwrap().slug, "flint-spade");
    assert!(base_item("nope").is_none());
    assert_eq!(base_item_for("pants", 84).unwrap().slug, "linen-pants");
    assert!(base_item_for("hat", 84).is_none());
    assert!(BASE_ITEMS.iter().all(|b| SLOTS.contains(&b.slot)));
}

#[test]
fn issue_display() {
    assert_eq!(Issue::new("a", "b").to_string(), "a: b");
}
