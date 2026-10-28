//! Mod settings (API 0.3.3): values in `mods.toml`, `bh.settings`, `settings-changed`, `/mods settings` and `/mods set`,
//! hot reload of `mods.toml` edits and the `client-required` override.

mod common;

use std::fs;
use std::path::Path;

use bhmod_core::types::{CommandSender, LogLevel, Role, Side};
use bhmod_core::{HostOptions, ModHost};
use common::*;
use serde_json::json;

const SETTINGS: &str = r#"
[settings.radius]
type = "int"
default = 32
min = 4
max = 128
label = "Radius"

[settings.gain]
type = "float"
default = 1.0
label = "Gain"
restart = true

[settings.who]
type = "roles"
default = ["admin"]
label = "Who"

[settings.talk]
type = "matrix"
rows = ["global"]
cols = ["admin", "everyone"]
default = { global = { admin = true, everyone = false } }
label = "Talk"

[settings.channels]
type = "list"
default = []
label = "Channels"
item = { name = { type = "string", default = "", label = "Name" } }
"#;

/// Records `settings-changed` events of its own and of `other`, and offers commands to call `bh.settings`.
const MAIN: &str = r#"
local mod = {}
local seen = {}
function mod.on_enable()
    assert(type(bh.settings.get("radius")) == "number")
    assert(bh.settings.get("nope") == nil)
    local all = bh.settings.all()
    assert(all.who ~= nil and all.talk.global ~= nil and all.channels ~= nil)
    bh.events.on_custom("cfg:settings-changed", function(ev)
        table.insert(seen, table.concat(ev.payload.keys, ","))
    end)
    bh.commands.register({ name = "seen", usage = "/seen" }, function()
        local s = table.concat(seen, ";")
        seen = {}
        return s
    end)
    bh.commands.register({ name = "radius", usage = "/radius", console = true }, function()
        return tostring(bh.settings.get("radius"))
    end)
    bh.commands.register({ name = "setr", usage = "/setr <n>", console = true }, function(_, _, args)
        local ok, err = pcall(bh.settings.set, "radius", tonumber(args[1]))
        return ok and "ok" or tostring(err)
    end)
    bh.commands.register({ name = "setbad", usage = "/setbad", console = true }, function()
        local a = pcall(bh.settings.set, "ghost", 1)
        local b = pcall(bh.settings.set, "who", { 1 })
        local c = pcall(bh.settings.set, "channels", { { name = "a" } })
        local d = pcall(bh.settings.set, "gain", 0.5)
        return tostring(a) .. tostring(b) .. tostring(c) .. tostring(d) .. bh.settings.get("channels")[1].name
    end)
end
return mod
"#;

/// A second mod that listens to the first one's changes.
const WATCHER: &str = r#"
local mod = {}
local n = 0
function mod.on_enable()
    bh.events.on_custom("cfg:settings-changed", function(ev) n = n + 1 end)
    bh.commands.register({ name = "watched", usage = "/watched", console = true }, function() return tostring(n) end)
    -- Another mod's settings are not reachable: `bh.settings` is always the caller's own.
    assert(bh.settings.get("radius") == nil)
    assert(next(bh.settings.all()) == nil)
    assert(not pcall(bh.settings.set, "radius", 8))
end
return mod
"#;

struct Setup {
    _tmp: tempfile::TempDir,
    mods: std::path::PathBuf,
    bridge: std::rc::Rc<FakeBridge>,
}

fn setup(config: &str) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    lua_mod(&mods, "cfg", SETTINGS, MAIN);
    lua_mod(&mods, "watcher", "", WATCHER);
    fs::write(mods.join("mods.toml"), config).unwrap();
    Setup { _tmp: tmp, mods, bridge: FakeBridge::new() }
}

impl Setup {
    fn host(&self) -> ModHost {
        let mut opts = HostOptions::new(&self.mods, Side::Server);
        opts.clock = Box::new(FakeClock::ahead());
        let mut h = ModHost::load(opts, self.bridge.clone());
        h.enable("w", "id");
        h
    }
}

fn console(h: &mut ModHost, line: &str) -> Vec<String> {
    h.command(CommandSender::Console, line).unwrap()
}

fn read(dir: &Path) -> String {
    fs::read_to_string(dir.join("mods.toml")).unwrap()
}

#[test]
fn stored_values_and_schema() {
    let s = setup("[cfg.settings]\nradius = 48\ngain = 500\nghost = 1\nwho = 1979-05-27\n");
    let h = s.host();
    assert_eq!(h.mods().len(), 2, "{}", s.bridge.log_text());
    let v = h.settings_values("cfg").unwrap();
    assert_eq!(v["radius"], json!(48));
    assert_eq!(v["gain"], json!(500.0), "no max, so 500 is fine");
    assert_eq!(v["who"], json!(["admin"]), "a date is invalid and falls back to the default");
    assert!(s.bridge.has_log("ignoring unknown setting `ghost`"));
    assert!(s.bridge.has_log("invalid setting who: expected a list of role names, got a text; using the default"));
    let schema = h.settings_schema("cfg").unwrap();
    assert_eq!(schema["values"]["radius"], json!(48));
    assert_eq!(schema["fields"][0]["key"], json!("radius"));
    assert_eq!(schema["fields"][4]["item"][0]["key"], json!("name"));
    assert_eq!(h.settings_schema("watcher").unwrap(), json!({ "fields": [], "values": {} }));
    assert!(h.settings_schema("nope").is_none());
    assert!(h.settings_values("nope").is_none());
    assert_eq!(h.client_required("cfg"), Some(false));
    assert_eq!(h.client_required("nope"), None);
}

#[test]
fn mods_read_and_change_their_settings() {
    let s = setup("# keep me\n[cfg]\nenabled = true\n");
    let mut h = s.host();
    assert_eq!(console(&mut h, "/radius"), ["32"]);
    assert_eq!(console(&mut h, "/setr 64"), ["ok"]);
    assert_eq!(console(&mut h, "/radius"), ["64"]);
    assert_eq!(read(&s.mods), "# keep me\n[cfg]\nenabled = true\n\n[cfg.settings]\nradius = 64\n");
    let out = console(&mut h, "/setr 2");
    assert!(out[0].contains("invalid argument: radius: must be at least 4"), "{out:?}");
    assert_eq!(console(&mut h, "/setbad"), ["falsefalsetruetruea"]);
    assert_eq!(console(&mut h, "/watched"), ["3"], "radius, channels, gain");
    // The same value again changes nothing and fires nothing.
    assert_eq!(console(&mut h, "/setr 64"), ["ok"]);
    assert_eq!(console(&mut h, "/watched"), ["3"]);
    // Back to the default removes the stored value.
    assert_eq!(console(&mut h, "/setr 32"), ["ok"]);
    assert!(!read(&s.mods).contains("radius"));
    s.bridge.join("p1", "ADMIN", Role::Admin);
    let seen = h.command(CommandSender::Player("p1".into()), "/seen").unwrap();
    assert_eq!(seen, ["radius;channels;gain;radius"]);
}

/// The server's `/mods ...` line routed to `ModHost::settings_command`.
fn mods(h: &mut ModHost, admin: bool, line: &str) -> Vec<String> {
    let args: Vec<&str> = line.trim_start_matches("/mods").split_whitespace().collect();
    h.settings_command(admin, &args).unwrap()
}

#[test]
fn admin_commands() {
    let s = setup("");
    let mut h = s.host();
    s.bridge.join("p1", "ADMIN", Role::Admin);
    s.bridge.join("p2", "BOB", Role::Moderator);
    assert!(!h.owns_command("/mod settings cfg"), "the vanilla /mod <player> stays the game's");
    assert_eq!(h.command(CommandSender::Console, "/mod p1"), None);
    assert_eq!(h.settings_command(true, &["reload"]), None, "the server's own /mods subcommands");
    assert_eq!(h.settings_command(true, &[] as &[&str]), None);
    let out = mods(&mut h, true, "/mods settings cfg");
    assert_eq!(
        out,
        [
            "cfg settings:",
            "radius = 32",
            "gain = 1.0 (restart)",
            "who = [\"admin\"]",
            "talk = {\"global\":{\"admin\":true,\"everyone\":false}}",
            "channels = []"
        ]
    );
    assert_eq!(mods(&mut h, true, "/mods settings watcher"), ["watcher has no settings."]);
    assert_eq!(mods(&mut h, true, "/mods settings nope"), ["No mod nope is loaded."]);
    assert_eq!(mods(&mut h, true, "/mods set nope a 1"), ["No mod nope is loaded."]);
    assert_eq!(mods(&mut h, true, "/mods set cfg nope 1"), ["cfg has no setting nope (see /mods settings cfg)."]);
    assert_eq!(mods(&mut h, true, "/mods set cfg radius 100"), ["cfg: radius = 100."]);
    assert_eq!(mods(&mut h, true, "/mods set cfg gain 1.5"), ["cfg: gain = 1.5. It takes effect after a restart."]);
    assert_eq!(mods(&mut h, true, "/mods set cfg who owner, admin"), ["cfg: who = [\"owner\",\"admin\"]."]);
    let talk = r#"/mods set cfg talk {"global": {"admin": false, "everyone": true}}"#;
    assert_eq!(mods(&mut h, true, talk), ["cfg: talk = {\"global\":{\"admin\":false,\"everyone\":true}}."]);
    assert_eq!(mods(&mut h, true, "/mods set cfg radius 1"), ["Not changed: radius: must be at least 4"]);
    for usage in ["/mods set cfg radius", "/mods settings", "/mods set"] {
        assert_eq!(mods(&mut h, true, usage), ["Usage: /mods settings <mod> | /mods set <mod> <key> <value>"]);
    }
    assert_eq!(mods(&mut h, true, "/mods set cfg radius 8"), ["cfg: radius = 8."]);
    let modr = mods(&mut h, false, "/mods set cfg radius 9");
    assert_eq!(modr, ["You do not have permission to change mod settings."]);
    assert_eq!(h.settings_values("cfg").unwrap()["radius"], json!(8));
    let text = read(&s.mods);
    assert!(
        text.contains("radius = 8") && text.contains("gain = 1.5") && text.contains("who = [\"owner\", \"admin\"]")
    );
    // The host API used by the host app.
    assert_eq!(h.set_setting("cfg", "radius", &json!(16.0)), Ok(json!(16)));
    assert_eq!(h.set_setting("cfg", "ghost", &json!(1)).unwrap_err().code(), "settingKey");
    assert_eq!(h.set_setting("nope", "radius", &json!(1)).unwrap_err().code(), "settingKey");
    assert_eq!(h.set_setting("cfg", "radius", &json!("x")).unwrap_err().code(), "setting");
    let cbor = h.settings_schema_cbor("cfg").unwrap();
    let back = bhmod_core::settings::from_cbor(&cbor).unwrap();
    assert_eq!(back, h.settings_schema("cfg").unwrap());
    assert!(h.settings_schema_cbor("nope").is_none());
    assert_eq!(h.set_setting_cbor("cfg", "radius", &bhmod_core::settings::to_cbor(&json!(20))), Ok(json!(20)));
    assert_eq!(h.set_setting_cbor("cfg", "radius", &[0xff]).unwrap_err().rule(), Some("type"));
    assert_eq!(console(&mut h, "/watched"), ["7"]);
}

#[test]
fn mods_may_register_mod_and_the_host_owns_nothing_cold() {
    let s = setup("");
    lua_mod(
        &s.mods,
        "thief",
        "",
        "local m = {}\nfunction m.on_enable()\n assert(pcall(bh.commands.register, { name = \"x\", aliases = { \"mod\" }, usage = \"\" }, function() end))\nend\nreturn m\n",
    );
    let h = s.host();
    assert!(!s.bridge.has_log("on-enable failed"), "{}", s.bridge.log_text());
    assert_eq!(h.mods().len(), 3);
    // Before the world is loaded the host owns nothing.
    let cold = ModHost::load(HostOptions::new(&s.mods, Side::Server), s.bridge.clone());
    assert!(!cold.owns_command("/x"));
}

#[test]
fn edits_of_mods_toml_hot_reload() {
    let s = setup("[cfg.settings]\nradius = 48\n");
    let mut h = s.host();
    assert_eq!(console(&mut h, "/radius"), ["48"]);
    // `poll` looks at the file (the first time at once).
    fs::write(s.mods.join("mods.toml"), "[cfg.settings]\nradius = 50\nwho = [\"mod\"]\n").unwrap();
    h.poll();
    assert_eq!(console(&mut h, "/radius"), ["50"]);
    assert!(s.bridge.has_log("settings changed: radius, who") || s.bridge.has_log("settings changed: who, radius"));
    // Within the interval nothing is looked at.
    fs::write(s.mods.join("mods.toml"), "[cfg.settings]\nradius = 60\n").unwrap();
    h.poll();
    assert_eq!(console(&mut h, "/radius"), ["50"]);
    assert_eq!(h.reload_settings(), ["cfg"]);
    assert_eq!(console(&mut h, "/radius"), ["60"]);
    // A broken file keeps the values.
    fs::write(s.mods.join("mods.toml"), "[cfg.settings\n").unwrap();
    assert!(h.reload_settings().is_empty());
    assert_eq!(console(&mut h, "/radius"), ["60"]);
    assert!(s.bridge.logs.borrow().iter().any(|(_, l, m)| *l == LogLevel::Error && m.contains("(settings stay)")));
    // An invalid value falls back to the default; nothing changed gives nothing.
    fs::write(s.mods.join("mods.toml"), "[cfg.settings]\nradius = 1\nwho = [\"mod\"]\n").unwrap();
    assert_eq!(h.reload_settings(), ["cfg"]);
    assert_eq!(console(&mut h, "/radius"), ["32"]);
    assert!(h.reload_settings().is_empty());
    assert_eq!(console(&mut h, "/watched"), ["3"]);
    // A reloaded mod keeps the stored values.
    assert_eq!(h.set_setting("cfg", "radius", &json!(77)), Ok(json!(77)));
    h.reload(Some("cfg"));
    assert_eq!(console(&mut h, "/radius"), ["77"]);
}

#[test]
fn a_write_failure_keeps_the_value_in_memory() {
    let s = setup("[cfg]\nenabled = true\n");
    let mut h = s.host();
    // A directory where the file should be: reading and writing fail.
    fs::remove_file(s.mods.join("mods.toml")).unwrap();
    fs::create_dir(s.mods.join("mods.toml")).unwrap();
    assert_eq!(console(&mut h, "/setr 40"), ["ok"]);
    assert_eq!(console(&mut h, "/radius"), ["40"]);
    assert!(s.bridge.has_log("setting radius not saved"));
    // Without the file the memory stays as the config to reload from.
    assert!(h.reload_settings().is_empty());
}

#[test]
fn settings_change_before_the_world_fires_nothing() {
    let s = setup("");
    let mut h = ModHost::load(HostOptions::new(&s.mods, Side::Server), s.bridge.clone());
    assert_eq!(h.set_setting("cfg", "radius", &json!(10)), Ok(json!(10)));
    assert!(h.reload_settings().is_empty(), "the file has the new value");
    h.poll();
}

#[test]
fn client_required_override_from_mods_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    let d = mods.join("both");
    fs::create_dir_all(d.join("server")).unwrap();
    fs::create_dir_all(d.join("client")).unwrap();
    let toml = LUA_TOML
        .replace("ID", "both")
        .replace("client = false", "client = true")
        .replace("[entry]\n", "[entry]\nclient = { runtime = \"lua\", path = \"client/main.luau\" }\n");
    fs::write(d.join("mod.toml"), toml).unwrap();
    fs::write(d.join("server/main.luau"), "return {}").unwrap();
    fs::write(d.join("client/main.luau"), "return {}").unwrap();
    let bridge = FakeBridge::new();
    let h = ModHost::load(HostOptions::new(&mods, Side::Server), bridge.clone());
    assert_eq!(h.client_required("both"), Some(true), "{}", bridge.log_text());
    fs::write(mods.join("mods.toml"), "[both]\nclient-required = false\n").unwrap();
    let h = ModHost::load(HostOptions::new(&mods, Side::Server), bridge);
    assert_eq!(h.client_required("both"), Some(false));
}
