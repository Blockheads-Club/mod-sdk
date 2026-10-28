//! API 0.3.4 through the Luau mapping: `bh.voice` (permission, sides, ranges, players, the VoiceHost seam),
//! `bh.terrain.solid_count`, `bh.input` and the `bh.ui` additions, the `voice:*`, `<mod>:input` and `<mod>:ui-button`
//! events, and client-scope settings stored on the device.

mod common;

use std::path::Path;
use std::rc::Rc;

use bhmod_core::input::InputCommand;
use bhmod_core::types::{Role, Side};
use bhmod_core::ui::{ButtonAction, OverheadIcon, UiCommand};
use bhmod_core::voice::{MicState, Mode, VoiceEvent, VoiceState};
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;
use serde_json::json;

/// Writes an unpacked Luau mod for `side` with `[permissions]` and extra manifest text.
fn write_mod(mods: &Path, id: &str, side: Side, perms: &str, extra: &str, main: &str) {
    let d = mods.join(id);
    let (dir, server, client) = match side {
        Side::Server => ("server", true, false),
        Side::Client => ("client", false, true),
    };
    std::fs::create_dir_all(d.join(dir)).unwrap();
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = {{ en = \"N\" }}\nauthors = [\"a\"]\nlicense = \"MIT\"\n\
         api = \"^0.3.4\"\n[sides]\nserver = {server}\nclient = {client}\n[entry]\n{dir} = {{ runtime = \"lua\", path = \"{dir}/main.luau\" }}\n\
         [permissions]\n{perms}\n{extra}"
    );
    std::fs::write(d.join("mod.toml"), toml).unwrap();
    std::fs::write(d.join(format!("{dir}/main.luau")), main).unwrap();
}

struct Rig {
    _t: tempfile::TempDir,
    b: Rc<FakeBridge>,
    h: ModHost,
}

fn rig(side: Side, mods: &[(&str, &str, &str)], grant: &str, opts: impl FnOnce(&mut HostOptions, &Path)) -> Rig {
    let t = tempfile::tempdir().unwrap();
    for (id, perms, main) in mods {
        write_mod(t.path(), id, side, perms, "", main);
    }
    std::fs::write(t.path().join("mods.toml"), grant).unwrap();
    let b = FakeBridge::new();
    b.has_voice.set(true);
    b.client.set(side == Side::Client);
    b.join("p1", "ALICE", Role::Admin);
    b.join("p2", "BOB", Role::Player);
    let mut o = HostOptions::new(t.path(), side);
    opts(&mut o, t.path());
    let mut h = ModHost::load(o, b.clone());
    h.enable("World", "wid");
    Rig { _t: t, b, h }
}

const SERVER: &str = r#"
local mod = {}
local function fails(what, f, ...)
    local ok, err = pcall(f, ...)
    assert(not ok and string.find(tostring(err), what, 1, true), tostring(err))
end
function mod.on_enable()
    local a, b = bh.players.get("p1"), bh.players.get("p2")
    assert(bh.voice.state(a) == "self-muted")
    local i = bh.voice.icons()
    assert(i.no_voice == false and i.speaking == true)
    bh.voice.set_routes(a, { { listener = b, modes = { "proximity", "global" }, gain = 0.5, pan = -2, muffle = 0.1 },
        { listener = b } })
    bh.voice.mute(b, 1700000000000)
    bh.voice.mute(b)
    assert(bh.voice.unmute(b) == true)
    bh.voice.set_speaker_gain(a, 150)
    bh.voice.set_caps({ max_bitrate = 16000 })
    bh.voice.set_transport({ mode = "udp", udp_port = 15151 })
    bh.voice.set_transport({})
    bh.voice.set_allowed(a, { "proximity", "channel" }, { "ptt" })
    bh.voice.set_icons({ deafen = false })
    bh.voice.set_broadcast_overrides_deafen(true)
    -- the other side's functions
    fails("unsupported", bh.voice.set_master, 50)
    fails("unsupported", bh.voice.talk, nil, true)
    -- ranges, names, players
    fails("invalid argument", bh.voice.set_speaker_gain, a, 201)
    fails("invalid argument", bh.voice.set_caps, { max_speakers = 0 })
    fails("unknown mode", bh.voice.set_allowed, a, { "shout" }, {})
    fails("unknown activation", bh.voice.set_allowed, a, {}, { "think" })
    fails("unknown mode \"pigeon\"", bh.voice.set_transport, { mode = "pigeon" })
    fails("invalid argument: player", bh.voice.set_routes, a, { { listener = "p2" } })
    local routes = {}
    for n = 1, 257 do routes[n] = { listener = b } end
    fails("limit exceeded", bh.voice.set_routes, a, routes)
    -- input is client only
    fails("unsupported", bh.input.bind, "ptt", "v")
    bh.events.on_custom("voice:talk-start", function(ev) print("start", ev.payload.player, ev.payload.mode) end)
    bh.events.on_custom("voice:state", function(ev) print("state", ev.payload.player, ev.payload.state) end)
    bh.commands.register({ name = "gone", usage = "" }, function()
        local ok, err = pcall(bh.voice.state, a)
        return tostring(string.find(tostring(err), "stale handle", 1, true) ~= nil)
    end)
end
return mod
"#;

const DEAF: &str = r#"
local mod = {}
function mod.on_enable()
    local ok, err = pcall(bh.voice.state, bh.players.get("p1"))
    assert(not ok and string.find(tostring(err), "not permitted: voice", 1, true), tostring(err))
    bh.events.on_custom("voice:talk-start", function() print("EAVESDROP") end)
end
return mod
"#;

#[test]
fn server_voice_api() {
    let mut r = rig(
        Side::Server,
        &[("vc", "voice = true", SERVER), ("deaf", "", DEAF)],
        "[vc]\ngrant = [\"voice\"]\n",
        |_, _| {},
    );
    let states: Vec<_> = r.h.mods().iter().map(|m| m.state.clone()).collect();
    assert_eq!(states, vec![ModState::Enabled; 2], "{}", r.b.log_text());
    let calls = r.b.voice.take();
    let want = [
        "state p1",
        "routes p1 [Route { listener: \"p2\", modes: 3, gain: 0.5, pan: -1.0, muffle: 0.1 }, Route { listener: \"p2\", modes: 0, gain: 1.0, pan: 0.0, muffle: 0.0 }]",
        "mute p2 Some(1700000000000)",
        "mute p2 None",
        "unmute p2",
        "gain p1 150",
        "caps Caps { max_bitrate: 16000, max_speakers: 8, max_frames_per_sec: 60 }",
        "transport TransportConfig { mode: Udp, udp_port: Some(15151) }",
        "transport TransportConfig { mode: Udp, udp_port: None }",
        "allowed p1 5 1",
        "icons Icons { speaking: true, no_voice: true, self_mute: true, deafen: false, admin_mute: true }",
        "override true",
    ];
    assert_eq!(calls, want);

    // events reach mods with the permission only
    r.h.voice_event(&VoiceEvent::TalkStart { player: "p1".into(), mode: Mode::Channel });
    r.h.voice_event(&VoiceEvent::State { player: "p2".into(), state: VoiceState::AdminMuted });
    assert!(r.b.has_log("start\tp1\tchannel") && r.b.has_log("state\tp2\tadmin-muted"), "{}", r.b.log_text());
    assert!(!r.b.has_log("EAVESDROP"));

    // a player who left
    r.b.leave("p1");
    assert_eq!(r.h.command(bhmod_core::types::CommandSender::Console, "/gone"), Some(vec!["true".into()]));
}

/// The terrain count needs the tiles before `on_enable`: a separate host whose tiles are set at load.
#[test]
fn terrain_before_enable_and_without_voice() {
    let t = tempfile::tempdir().unwrap();
    write_mod(
        t.path(),
        "tt",
        Side::Server,
        "voice = true",
        "",
        "local m = {}\nfunction m.on_enable()\n print(bh.terrain.solid_count(98, 0, 2, 0))\n local ok, e = pcall(bh.voice.icons)\n print(tostring(e))\nend\nreturn m\n",
    );
    std::fs::write(t.path().join("mods.toml"), "[tt]\ngrant = [\"voice\"]\n").unwrap();
    let b = FakeBridge::new();
    b.width.set(Some(100));
    b.solid.borrow_mut().extend([(99, 0), (0, 0), (50, 0)]);
    let mut h = ModHost::load(HostOptions::new(t.path(), Side::Server), b.clone());
    h.enable("W", "w");
    assert!(b.log_text().contains("tt: 2\n"), "{}", b.log_text());
    assert!(b.has_log("unsupported"), "no voice host: unsupported");
}

const CLIENT: &str = r#"
local mod = {}
local function fails(what, f, ...)
    local ok, err = pcall(f, ...)
    assert(not ok and string.find(tostring(err), what, 1, true), tostring(err))
end
function mod.on_enable()
    local a = bh.players.get("p1")
    bh.voice.set_default_mode("global")
    bh.voice.set_activation("toggle")
    bh.voice.talk(nil, true)
    bh.voice.talk("broadcast", false)
    bh.voice.set_mic(true)
    bh.voice.set_self_mute(true)
    bh.voice.set_deafen(false)
    bh.voice.set_vad_threshold(-50)
    bh.voice.set_master(120)
    assert(bh.voice.master() == 75)
    bh.voice.set_player_volume(a, 30)
    assert(bh.voice.player_volume(a) == 60)
    bh.voice.mute_player(a, true)
    assert(bh.voice.player_muted(a) == true)
    fails("unsupported", bh.voice.mute, a)
    fails("unknown mode", bh.voice.talk, "shout", true)
    fails("unknown mode", bh.voice.set_default_mode, "shout")
    fails("unknown activation", bh.voice.set_activation, "telepathy")
    fails("invalid argument", bh.voice.set_vad_threshold, 3)
    fails("invalid argument", bh.voice.set_master, 201)
    fails("invalid argument", bh.voice.set_player_volume, a, 300)
    fails("unsupported", bh.terrain.solid_count, 0, 0, 1, 1)
    -- input
    bh.input.bind("ptt", "v")
    bh.input.bind("ptt", "b")
    fails("unknown key name", bh.input.bind, "x", "hyper")
    assert(bh.input.key_of("ptt") == "b")
    assert(bh.input.key_of("nope") == nil)
    bh.input.bind("gone", "g")
    assert(bh.input.unbind("gone") == true and bh.input.unbind("gone") == false)
    bh.events.on_custom("vcl:input", function(ev) print("input", ev.payload.action, tostring(ev.payload.pressed)) end)
    -- ui additions
    bh.ui.set_overhead(a, "speaking")
    fails("unknown overhead icon", bh.ui.set_overhead, a, "dancing")
    fails("invalid argument: player", bh.ui.set_overhead, "p1", "speaking")
    assert(bh.ui.clear_overhead(a) == true and bh.ui.clear_overhead(a) == false)
    bh.ui.set_overhead(a, "deafen")
    bh.ui.set_button("ptt", { icon = "mic", label = "Talk" })
    bh.ui.set_button("full", { icon = "mic", x = 0.1, y = 0.2, size = 100, opacity = 50 })
    fails("invalid argument", bh.ui.set_button, "big", { icon = "mic", size = 500 })
    fails("icon is required", bh.ui.set_button, "x", {})
    assert(bh.ui.remove_button("full") == true and bh.ui.remove_button("full") == false)
    bh.ui.open_settings("vcl")
    fails("no loaded mod", bh.ui.open_settings, "ghost")
    bh.events.on_custom("vcl:ui-button", function(ev) print("button", ev.payload.id, ev.payload.action) end)
    bh.events.on_custom("voice:mic", function(ev) print("mic", ev.payload.state) end)
    local p2 = bh.players.get("p2")
    bh.commands.register({ name = "who", usage = "" }, function()
        local ok, err = pcall(bh.ui.set_overhead, p2, "speaking")
        return tostring(string.find(tostring(err), "stale handle", 1, true) ~= nil)
    end)
end
return mod
"#;

#[test]
fn client_voice_input_and_ui() {
    let mut r = rig(Side::Client, &[("vcl", "voice = true", CLIENT)], "[vcl]\ngrant = [\"voice\"]\n", |_, _| {});
    assert_eq!(r.h.mods()[0].state, ModState::Enabled, "{}", r.b.log_text());
    assert_eq!(
        r.b.voice.take(),
        [
            "default global",
            "activation toggle",
            "talk None true",
            "talk Some(\"broadcast\") false",
            "mic true",
            "self-mute true",
            "deafen false",
            "vad -50",
            "master 120",
            "volume p1 30",
            "mute-player p1 true",
        ]
    );
    r.b.rebinds.borrow_mut().push(("vcl".into(), "ptt".into(), Some("b".into())));
    let input = std::mem::take(&mut *r.b.input.borrow_mut());
    assert_eq!(input[0], ("vcl".to_string(), InputCommand::Bind { action: "ptt".into(), default_key: "v".into() }));
    assert!(input.contains(&("vcl".to_string(), InputCommand::Unbind { action: "gone".into() })));
    let ui: Vec<UiCommand> = r.b.take_ui().into_iter().map(|(_, c)| c).collect();
    assert_eq!(ui[0], UiCommand::SetOverhead { player: "p1".into(), icon: OverheadIcon::Speaking });
    assert!(ui.contains(&UiCommand::ClearOverhead { player: "p1".into() }));
    assert!(ui.contains(&UiCommand::OpenSettings { mod_id: "vcl".into() }));
    assert!(ui.contains(&UiCommand::RemoveButton { id: "full".into() }));

    // input and button events go to the owner only; unknown ones are refused
    assert!(r.h.input_event("vcl", "ptt", true));
    assert!(!r.h.input_event("vcl", "nope", true));
    assert!(!r.h.input_event("ghost", "ptt", true));
    assert!(r.h.ui_button("vcl", "ptt", ButtonAction::LongPress));
    assert!(!r.h.ui_button("vcl", "full", ButtonAction::Press));
    assert!(!r.h.ui_button("ghost", "ptt", ButtonAction::Press));
    r.h.voice_event(&VoiceEvent::Mic(MicState::Denied));
    let log = r.b.log_text();
    assert!(
        log.contains("input\tptt\ttrue") && log.contains("button\tptt\tlong-press") && log.contains("mic\tdenied"),
        "{log}"
    );

    // overhead on a player who left
    r.b.leave("p2");
    let who = r.h.command(bhmod_core::types::CommandSender::Console, "/who").unwrap();
    assert_eq!(who, ["true"]);

    // the mod goes away: its icons, buttons and bindings are cleared
    r.h.disable_all();
    assert!(r.b.take_ui().iter().any(|(m, c)| m == "vcl" && *c == UiCommand::ClearMod));
    assert!(r.b.input.borrow().iter().any(|(m, c)| m == "vcl" && *c == InputCommand::ClearMod));
}

#[test]
fn client_without_ui_or_input() {
    let t = tempfile::tempdir().unwrap();
    write_mod(
        t.path(),
        "bare",
        Side::Client,
        "",
        "",
        "local m = {}\nfunction m.on_enable()\n for _, f in ipairs({ function() bh.input.bind('a', 'v') end, function() bh.ui.set_overhead(bh.players.get('p1'), 'speaking') end, function() bh.ui.set_button('b', { icon = 'mic' }) end }) do local ok, e = pcall(f); print(tostring(e)) end\n assert(bh.ui.clear_overhead(bh.players.get('p1')) == false)\nend\nreturn m\n",
    );
    let b = FakeBridge::new();
    b.no_ui.set(true);
    b.join("p1", "A", Role::Player);
    let mut h = ModHost::load(HostOptions::new(t.path(), Side::Client), b.clone());
    h.enable("W", "w");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    assert_eq!(b.log_text().matches("unsupported").count(), 3, "{}", b.log_text());
    h.disable_all();
}

const SETTINGS_EXTRA: &str = "\n[settings.volume]\ntype = \"int\"\ndefault = 100\nmin = 0\nmax = 200\nscope = \"client\"\nlabel = { en = \"Volume\" }\n\
[settings.radius]\ntype = \"int\"\ndefault = 32\nmin = 4\nmax = 64\nlabel = { en = \"Radius\" }\n";

#[test]
fn client_settings_live_on_the_device() {
    let t = tempfile::tempdir().unwrap();
    let main = "local m = {}\nfunction m.on_enable()\n print('vol', bh.settings.get('volume'), bh.settings.get('radius'))\n bh.settings.set('volume', 80)\n local ok, e = pcall(bh.settings.set, 'radius', 8)\n print(tostring(e))\nend\nreturn m\n";
    write_mod(t.path(), "cs", Side::Client, "", SETTINGS_EXTRA, main);
    let device = t.path().join("device/client-settings.toml");
    std::fs::create_dir_all(device.parent().unwrap()).unwrap();
    std::fs::write(&device, "[cs.settings]\nvolume = 150\nradius = 10\n").unwrap();
    std::fs::write(t.path().join("mods.toml"), "[cs.settings]\nvolume = 5\n").unwrap();
    let b = FakeBridge::new();
    let mut o = HostOptions::new(t.path(), Side::Client);
    o.client_settings = Some(device.clone());
    let mut h = ModHost::load(o, b.clone());
    h.enable("W", "w");
    assert!(b.has_log("vol\t150\t32"), "device value for client keys, default for server keys: {}", b.log_text());
    assert!(b.has_log("a server setting"), "a server key cannot be set on a client: {}", b.log_text());
    assert!(std::fs::read_to_string(&device).unwrap().contains("volume = 80"));
    assert!(!std::fs::read_to_string(t.path().join("mods.toml")).unwrap().contains("80"));
    // the loader's own form: client fields only
    let (form, values) = h.settings_form("cs", bhmod_manifest::settings::Scope::Client).unwrap();
    assert_eq!(form["fields"].as_array().unwrap().len(), 1);
    assert_eq!(values, json!({"volume": 80}));
    assert!(h.settings_form("cs", bhmod_manifest::settings::Scope::Server).is_some());
    assert!(h.settings_form("ghost", bhmod_manifest::settings::Scope::Server).is_none());
    assert_eq!(h.take_changed_settings(), ["cs"]);
    assert!(h.take_changed_settings().is_empty());

    // a broken device file: defaults, logged
    std::fs::write(&device, "not toml [").unwrap();
    let b2 = FakeBridge::new();
    let mut o = HostOptions::new(t.path(), Side::Client);
    o.client_settings = Some(device);
    let _h = ModHost::load(o, b2.clone());
    assert!(b2.has_log("client settings are not read"), "{}", b2.log_text());
    // the option is ignored on a server
    let mut o = HostOptions::new(t.path(), Side::Server);
    o.client_settings = Some(t.path().join("nope.toml"));
    let _h = ModHost::load(o, FakeBridge::new());
}

const FACTS: &str = r#"
local mod = {}
function mod.on_enable()
    bh.commands.register({ name = "facts", usage = "" }, function()
        local a = bh.players.get("p1")
        local me = bh.players.local_player()
        local out = {
            bh.host.platform(), tostring(a:sleeping()), tostring(a:alive()), tostring(me and me:id()),
            tostring(bh.players.has_mod(a, "voice")), tostring(bh.terrain.world_width()), tostring(bh.ui.has_touch()),
        }
        if bh.host.current_side() == "client" then
            local al = bh.voice.allowed()
            for _, x in ipairs({ table.concat(al.modes, "+"), table.concat(al.activations, "+"), tostring(al.max_bitrate),
                tostring(bh.voice.self_muted()), tostring(bh.voice.deafened()), bh.voice.mic() }) do
                table.insert(out, x)
            end
        end
        return table.concat(out, " ")
    end)
    for _, e in ipairs({ "player:sleep", "player:wake", "player:respawn", "vf:ui-pick", "voice:allowed" }) do
        bh.events.on_custom(e, function(ev)
            print(e, tostring(ev.payload.player), tostring(ev.payload["max-bitrate"]))
        end)
    end
    local id = bh.ui.show_dialog({ title = "Mode", buttons = { { label = "1" }, { label = "2" }, { label = "3" }, { label = "Cancel" } } })
    print("four buttons", id)
end
return mod
"#;

#[test]
fn api_gaps_for_the_voice_mod() {
    for side in [Side::Server, Side::Client] {
        let mut r = rig(side, &[("vf", "voice = true", FACTS)], "[vf]\ngrant = [\"voice\"]\n", |_, _| {});
        assert_eq!(r.h.mods()[0].state, ModState::Enabled, "{}", r.b.log_text());
        assert!(r.b.log_text().contains("four buttons\t1"), "{}", r.b.log_text());
        let console = bhmod_core::types::CommandSender::Console;
        let empty = r.h.command(console.clone(), "/facts").unwrap();
        r.b.sleeping.borrow_mut().push("p1".into());
        r.b.alive.borrow_mut().push("p1".into());
        *r.b.local.borrow_mut() = Some("p2".into());
        r.b.has_mods.borrow_mut().push(("p1".into(), "voice".into()));
        r.b.width.set(Some(8192));
        r.b.touch.set(true);
        let full = r.h.command(console, "/facts").unwrap();
        match side {
            Side::Server => {
                assert_eq!(empty, ["test-linux-x86_64 false false nil false nil false"]);
                assert_eq!(full, ["test-linux-x86_64 true true p2 true 8192 true"]);
            }
            Side::Client => {
                assert_eq!(
                    full,
                    ["test-linux-x86_64 false false p2 false 8192 true proximity+global ptt 16000 true false on"]
                );
            }
        }
        r.h.player_event(bhmod_core::PlayerEvent::Sleep, "p1");
        r.h.player_event(bhmod_core::PlayerEvent::Wake, "p1");
        r.h.player_event(bhmod_core::PlayerEvent::Respawn, "p2");
        r.h.ui_pick("p2");
        let a = bhmod_core::voice::Allowance { modes: 1, activations: 1, max_bitrate: 8000 };
        r.h.voice_event(&VoiceEvent::Allowed(a));
        let log = r.b.log_text();
        for line in
            ["player:sleep\tp1", "player:wake\tp1", "player:respawn\tp2", "vf:ui-pick\tp2", "voice:allowed\tnil\t8000"]
        {
            assert!(log.contains(line), "{line} in {log}");
        }
    }
}
