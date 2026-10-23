//! `bh.content` (API 0.3): ids, the cosmetics of loaded mods, equip rules, and the default bridge.

mod common;

use std::fs;

use bhmod_core::types::{ApiError, ContentKind, Role, Side};
use bhmod_core::{GameBridge, HostOptions, ModHost, ModState};
use common::*;

const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 31, 21, 196,
    137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0,
    0, 73, 69, 78, 68, 174, 66, 96, 130,
];

const CONTENT: &str = "[content]\ncosmetics = [\n\
  { key = \"MID:crown\", slot = \"hat\", texture = \"assets/crown.png\" },\n\
  { key = \"MID:ember\", slot = \"hair\", texture = \"assets/ember.png\" },\n]\n";

fn mod_with_content(dir: &std::path::Path, id: &str, main: &str) {
    let d = dir.join(id);
    for side in ["server", "client"] {
        fs::create_dir_all(d.join(side)).unwrap();
        fs::write(d.join(format!("{side}/main.luau")), main).unwrap();
    }
    fs::create_dir_all(d.join("assets")).unwrap();
    for f in ["crown", "ember"] {
        fs::write(d.join(format!("assets/{f}.png")), PNG).unwrap();
    }
    let toml = LUA_TOML
        .replace("ID", id)
        .replace("client = false", "client = true")
        .replace("api = \"^0.1\"", "api = \"^0.3\"")
        .replace("[entry]\n", "[entry]\nclient = { runtime = \"lua\", path = \"client/main.luau\" }\n");
    fs::write(d.join("mod.toml"), format!("{toml}{}", CONTENT.replace("MID", id))).unwrap();
}

fn load(dir: &std::path::Path, side: Side) -> (std::rc::Rc<FakeBridge>, ModHost) {
    let b = FakeBridge::new();
    b.join("p1", "ALICE", Role::Player);
    b.cosmetic_ids.borrow_mut().extend([("crowns:crown".to_string(), 1), ("crowns:ember".to_string(), 2)]);
    let mut o = HostOptions::new(dir, side);
    o.budgets.hard = std::time::Duration::from_secs(3);
    o.budgets.soft = o.budgets.hard;
    o.budgets.step = o.budgets.hard;
    (b.clone(), ModHost::load(o, b))
}

const SERVER: &str = r#"
local mod = {}
function mod.on_enable()
    local p = bh.players.get("p1")
    assert(bh.content.id_of("cosmetic", "crowns:crown") == 1)
    assert(bh.content.id_of("cosmetic", "crowns:none") == nil)
    assert(bh.content.id_of("block", "crowns:crown") == nil)
    assert(bh.content.key_of("cosmetic", 2) == "crowns:ember")
    assert(bh.content.key_of("cosmetic", 9) == nil)
    assert(not pcall(bh.content.id_of, "sound", "x"))
    local list = bh.content.cosmetics()
    assert(#list == 4, "both mods")
    local by = {}
    for _, c in ipairs(list) do by[c.key] = c end
    assert(by["crowns:crown"].mod_id == "crowns" and by["crowns:crown"].slot == "hat" and by["crowns:crown"].id == 1)
    assert(by["crowns:ember"].slot == "hair" and by["rival:crown"].id == nil)
    assert(next(bh.content.equipped(p)) == nil)
    bh.content.equip(p, "crowns:crown")
    bh.content.equip(p, "crowns:ember")
    assert(bh.content.equipped(p).hat == "crowns:crown" and bh.content.equipped(p).hair == "crowns:ember")
    bh.content.unequip(p, "hat")
    assert(bh.content.equipped(p).hat == nil)
    local function fails(f, text)
        local ok, err = pcall(f)
        assert(not ok and string.find(tostring(err), text, 1, true), tostring(err))
    end
    fails(function() bh.content.equip(p, "other:thing") end, "invalid argument")
    fails(function() bh.content.equip(p, "rival:crown") end, "not permitted")
    fails(function() bh.content.unequip(p, "cape") end, "invalid argument")
    fails(function() bh.content.equip("p1", "crowns:crown") end, "invalid argument: player")
    fails(function() bh.content.equip(bh.value, "crowns:crown") end, "invalid argument: player")
    bh.content.equip(p, "crowns:crown")
    bh.players.get("p1") -- still online
    mod.p = p
end
return mod
"#;

const RIVAL: &str = "local mod = {}\nfunction mod.on_enable() end\nreturn mod\n";

#[test]
fn server_mod_uses_content() {
    let tmp = tempfile::tempdir().unwrap();
    mod_with_content(tmp.path(), "crowns", SERVER);
    mod_with_content(tmp.path(), "rival", RIVAL);
    let (b, mut h) = load(tmp.path(), Side::Server);
    h.enable("World", "wid");
    assert_eq!(h.mods().len(), 2, "{}", b.log_text());
    assert!(h.mods().iter().all(|m| m.state == ModState::Enabled), "{}", b.log_text());
    assert_eq!(
        *b.worn.borrow(),
        vec![("p1".to_string(), "hair".to_string(), 2), ("p1".to_string(), "hat".to_string(), 1)]
    );
}

#[test]
fn stale_player_and_missing_id() {
    let tmp = tempfile::tempdir().unwrap();
    let main = r#"
local mod = {}
function mod.on_enable()
    local p = bh.players.get("p1")
    bh.events.on("tick", function()
        local function fails(f, text)
            local ok, err = pcall(f)
            assert(not ok and string.find(tostring(err), text, 1, true), tostring(err))
        end
        fails(function() bh.content.equip(p, "crowns:crown") end, "stale")
        fails(function() bh.content.equipped(p) end, "stale")
        fails(function() bh.content.unequip(p, "hat") end, "stale")
        bh.log.info("stale ok")
    end)
    local q = bh.players.get("p1")
    bh.players.broadcast("x")
end
return mod
"#;
    mod_with_content(tmp.path(), "crowns", &main.replace("bh.players.broadcast(\"x\")", ""));
    let (b, mut h) = load(tmp.path(), Side::Server);
    h.enable("World", "wid");
    b.leave("p1");
    h.tick(0.2);
    assert!(b.has_log("stale ok"), "{}", b.log_text());
    // A key without an id in this world cannot be equipped.
    let (b, mut h) = load(tmp.path(), Side::Server);
    b.cosmetic_ids.borrow_mut().clear();
    h.enable("World", "wid");
    let main2 = r#"
local mod = {}
function mod.on_enable()
    local ok, err = pcall(bh.content.equip, bh.players.get("p1"), "crowns:crown")
    assert(not ok and string.find(tostring(err), "no id in this world", 1, true), tostring(err))
    bh.log.info("noid ok")
end
return mod
"#;
    for side in ["server", "client"] {
        fs::write(tmp.path().join(format!("crowns/{side}/main.luau")), main2).unwrap();
    }
    let (b, mut h) = load(tmp.path(), Side::Server);
    b.cosmetic_ids.borrow_mut().clear();
    h.enable("World", "wid");
    assert!(b.has_log("noid ok"), "{}", b.log_text());
}

#[test]
fn clients_cannot_equip() {
    let tmp = tempfile::tempdir().unwrap();
    let main = r#"
local mod = {}
function mod.on_enable()
    local p = bh.players.get("p1")
    assert(#bh.content.cosmetics() == 2)
    for _, f in ipairs({
        function() bh.content.equip(p, "crowns:crown") end,
        function() bh.content.unequip(p, "hat") end,
        function() bh.content.equipped(p) end,
    }) do
        local ok, err = pcall(f)
        assert(not ok and string.find(tostring(err), "unsupported", 1, true), tostring(err))
    end
    bh.log.info("client ok")
end
return mod
"#;
    mod_with_content(tmp.path(), "crowns", main);
    let (b, mut h) = load(tmp.path(), Side::Client);
    h.enable("World", "wid");
    assert!(b.has_log("client ok"), "{}", b.log_text());
}

#[test]
fn default_bridge_has_no_registry() {
    struct Bare;
    impl GameBridge for Bare {
        fn side(&self) -> Side {
            Side::Client
        }
        fn host_name(&self) -> String {
            String::new()
        }
        fn log(&self, _: Option<&str>, _: bhmod_core::types::LogLevel, _: &str) {}
        fn online_players(&self) -> Vec<String> {
            vec![]
        }
        fn player_alias(&self, _: &str) -> Option<String> {
            None
        }
        fn player_role(&self, _: &str) -> Role {
            Role::Player
        }
        fn send_chat(&self, _: &str, _: &str) -> bool {
            false
        }
        fn broadcast(&self, _: &str) {}
        fn kick(&self, _: &str, _: &str) -> bool {
            false
        }
        fn is_builtin_command(&self, _: &str) -> bool {
            false
        }
    }
    let b = Bare;
    assert_eq!(b.content_id(ContentKind::Cosmetic, "a:b"), None);
    assert_eq!(b.content_key(ContentKind::Item, 1), None);
    assert_eq!(b.set_cosmetic("p", "hat", Some(1)), Err(ApiError::Unsupported));
    assert!(b.cosmetics_of("p").is_empty());
    assert_eq!(ContentKind::parse("item"), Some(ContentKind::Item));
    assert_eq!(ContentKind::parse("x"), None);
    assert_eq!(ContentKind::Block.name(), "block");
}
