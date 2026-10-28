use std::cell::Cell;

use super::*;
use crate::testutil::{content_package, package, package_files};
use bhmod_client::cache::sha256;
use bhmod_proto::{
    mods_hash, Channel, ChannelInfo, FileChunk, Message, ModInfo, ModList, ModPermissions, RejectText, ServerHello,
    ServiceCall, ServiceResult,
};
use serde_json::Value;

#[derive(Default)]
struct Fake {
    now: Cell<u64>,
    logs: RefCell<Vec<String>>,
    sent: RefCell<Vec<Vec<u8>>>,
    joins: RefCell<Vec<Option<Hash>>>,
    aborts: Cell<u32>,
    ui: RefCell<Vec<Value>>,
    /// `load_native` calls: (mod id, library file name, data folder exists), failing for the mods in `native_fail`.
    natives: RefCell<Vec<(String, String, bool)>>,
    unloaded: RefCell<Vec<String>>,
    native_fail: RefCell<Vec<String>>,
    /// Voice: the audio backend (`None`: none on this phone), the permission at start, dialogs asked.
    backend: RefCell<Option<crate::voice::audio::fake::FakeBackend>>,
    permission: Cell<crate::voice::audio::Permission>,
    asked: Cell<u32>,
    unsequenced: RefCell<Vec<Vec<u8>>>,
    camera: Cell<Option<Camera>>,
}

impl Platform for Fake {
    fn now_ms(&self) -> u64 {
        self.now.get()
    }
    fn log(&self, _level: LogLevel, msg: &str) {
        self.logs.borrow_mut().push(msg.to_string());
    }
    fn send(&self, packet: &[u8]) {
        self.sent.borrow_mut().push(packet.to_vec());
    }
    fn release_join(&self, mod_loader: Option<Hash>) {
        self.joins.borrow_mut().push(mod_loader);
    }
    fn abort_join(&self) {
        self.aborts.set(self.aborts.get() + 1);
    }
    fn ui(&self, json: String) {
        self.ui.borrow_mut().push(serde_json::from_str(&json).unwrap());
    }
    fn load_native(&self, mod_id: &str, lib: &Path, data_dir: &Path) -> Result<(), String> {
        let name = lib.file_name().unwrap().to_string_lossy().into_owned();
        self.natives.borrow_mut().push((mod_id.into(), name, data_dir.is_dir()));
        if self.native_fail.borrow().iter().any(|m| m == mod_id) {
            return Err("init returned 3".into());
        }
        Ok(())
    }
    fn unload_native(&self, mod_id: &str) {
        self.unloaded.borrow_mut().push(mod_id.into());
    }
    fn audio(&self) -> Option<AudioIo> {
        let b = self.backend.borrow().clone()?;
        Some(AudioIo::new(Box::new(b), self.permission.get()))
    }
    fn ask_mic(&self) {
        self.asked.set(self.asked.get() + 1);
    }
    fn send_unsequenced(&self, packet: &[u8]) -> bool {
        self.unsequenced.borrow_mut().push(packet.to_vec());
        true
    }
    fn camera(&self) -> Option<Camera> {
        self.camera.get()
    }
    fn local_player(&self) -> Option<String> {
        Some("me".into())
    }
}

impl Fake {
    fn msgs(&self) -> Vec<Message> {
        self.sent.borrow_mut().drain(..).map(|b| Message::decode(&b).unwrap()).collect()
    }
    fn ui_types(&self) -> Vec<String> {
        self.ui.borrow().iter().map(|v| v["t"].as_str().unwrap().to_string()).collect()
    }
    fn logged(&self, needle: &str) -> bool {
        self.logs.borrow().iter().any(|l| l.contains(needle))
    }
}

struct Rig {
    l: Loader<Fake>,
    p: Rc<Fake>,
    _t: tempfile::TempDir,
    dirs: Dirs,
}

fn rig() -> Rig {
    let t = tempfile::tempdir().unwrap();
    let dirs = Dirs::under(t.path());
    let p = Rc::new(Fake::default());
    let l =
        Loader::new(p.clone(), dirs.clone(), crate::caps(), crate::options(), Box::new(bhmod_core::default_runtimes))
            .with_budgets(crate::budgets());
    Rig { l, p, _t: t, dirs }
}

const ECHO_CLIENT: &str = r#"
local mod = {}
function mod.on_enable()
    local ch: number
    ch = bh.channels.open("demo:echo", function(_origin, data)
        bh.log.info(`client got {data}`)
    end)
    bh.events.on("world-loaded", function(_ev)
        bh.channels.send(ch, "server", "ping")
    end)
end
return mod
"#;

fn info(id: &str, bytes: &[u8]) -> ModInfo {
    ModInfo {
        id: id.into(),
        version: "1.0.0".into(),
        name: [("en".to_string(), id.to_string())].into(),
        description: Default::default(),
        sides: vec!["server".into(), "client".into()],
        client_required: true,
        sha256: sha256(bytes),
        size: bytes.len() as u64,
        permissions: ModPermissions { world: "none".into(), channels: true, ..Default::default() },
        source_url: None,
        index_listed: false,
    }
}

fn server_hello(hash: Hash, count: u64) -> Vec<u8> {
    Message::ServerHello(ServerHello {
        proto: 1,
        mods_hash: hash,
        client_required: true,
        count,
        players: None,
        features: vec![],
        voice_udp_port: None,
    })
    .encode()
}

#[test]
fn vanilla_server() {
    let mut r = rig();
    assert_eq!(r.l.session_state(), "none");
    // no 0x23 seen: the 0x1f goes at once
    r.l.on_join_request();
    assert_eq!(*r.p.joins.borrow(), [None]);
    // callbacks queued by another game thread arrive through on_net
    r.l.on_net(NetEvent::Contact("10.0.2.2:15151".into()));
    assert!(matches!(r.p.msgs()[..], [Message::Hello(_)]));
    r.l.on_join_request();
    r.l.on_frame();
    assert_eq!(r.p.joins.borrow().len(), 1);
    r.p.now.set(1500);
    r.l.on_frame();
    assert_eq!(*r.p.joins.borrow(), [None, None]);
    assert_eq!(r.l.session_state(), "vanilla");
    // non-0xBC packets go to the game
    assert!(!r.l.on_packet(&[0x23, 1]));
    assert!(!r.l.on_packet(&[]));
    r.l.on_net(NetEvent::Disconnect);
    assert_eq!(r.p.ui_types(), ["joined", "closed"]);
    // a 0xBC packet with no session is swallowed
    assert!(r.l.on_packet(&[0xbc, 0x01]));
}

#[test]
fn modded_join_runs_the_mod_and_its_channel() {
    let mut r = rig();
    let pkg = package("demo", "1.0.0", "", "client/main.luau", ECHO_CLIENT);
    let demo = info("demo", &pkg);
    let hash = mods_hash([(demo.id.as_str(), demo.version.as_str(), &demo.sha256)]);
    // an installed client mod runs too (and one with the same id as a server mod is left out)
    let inst = r.l.installed();
    inst.install(&package("extra", "1.0.0", "", "client/main.luau", "return {}"), None).unwrap();
    inst.install(&package("demo", "0.9.0", "", "client/main.luau", "return {}"), None).unwrap();
    inst.install(&package("offmod", "1.0.0", "", "client/main.luau", "return {}"), None).unwrap();
    inst.set_enabled("offmod", false).unwrap();

    r.l.on_contact("10.0.2.2:15151");
    r.p.msgs();
    r.l.on_join_request();
    assert!(r.l.on_packet(&server_hello(hash, 1)));
    assert!(matches!(r.p.msgs()[..], [Message::ListRequest]));
    let list = ModList {
        mods: vec![demo.clone()],
        channels: vec![ChannelInfo { name: "demo:echo".into(), id: 7 }],
        registry: Default::default(),
    };
    r.l.on_packet(&Message::ModList(list).encode());
    assert_eq!(r.p.ui_types(), ["prompt"]);
    assert_eq!(r.p.ui.borrow()[0]["mods"][0]["id"], "demo");
    r.l.on_ui(UiEvent::Decide(true));
    assert!(matches!(r.p.msgs()[..], [Message::FileRequest(_)]));
    r.p.now.set(50);
    let chunk = Message::FileChunk(FileChunk { sha256: demo.sha256, offset: 0, total: demo.size, data: pkg.clone() });
    r.l.on_packet(&chunk.encode());
    let m = r.p.msgs();
    assert!(matches!(&m[..], [Message::FileRequest(_), Message::Ready(_)]), "{m:?}");
    let Message::Ready(ready) = &m[1] else { panic!() };
    assert_eq!(ready.loaded, [("demo".to_string(), "1.0.0".to_string()), ("extra".to_string(), "1.0.0".to_string())]);
    assert_eq!(*r.p.joins.borrow(), [Some(hash)]);
    assert!(r.p.ui_types().contains(&"progress".to_string()));
    assert!(r.l.running());

    // the world opens: on-enable, world-loaded -> the mod sends "ping" to the server
    r.l.on_world(Some("Sunny"));
    let m = r.p.msgs();
    assert!(matches!(&m[..], [Message::Channel(Channel { channel_id: 7, payload })] if payload == b"ping"), "{m:?}");
    assert_eq!(r.p.ui.borrow().last().unwrap()["mods"], 2);
    // the server echoes (on a phone the game may receive it on another thread); the client mod gets it on the next
    // frame
    r.l.on_net(NetEvent::Packet(Message::Channel(Channel { channel_id: 7, payload: b"echo:ping".to_vec() }).encode()));
    r.p.now.set(400);
    r.l.on_frame();
    r.p.now.set(500);
    r.l.on_frame();
    assert!(r.p.logged("client got echo:ping"), "{:?}", r.p.logs.borrow());
    // disconnect while in the world keeps the host until the world closes
    r.l.on_disconnect();
    assert!(r.l.running());
    r.l.on_world(None);
    assert!(!r.l.running());
    assert!(r.dirs.cache.join(format!("{}.bhmod", bhmod_proto::hex(&demo.sha256))).is_file());
}

const CALLER: &str = r#"
local mod = {}
function mod.on_enable()
    bh.services.provide("hud", "1.0.0", { title = function(n) return "title " .. n end })
    local function ask(item)
        bh.services.call_remote("server", "prices", "^1", "price", { item }, function(ok, v)
            bh.log.info("answer " .. item .. " " .. tostring(ok) .. " " .. (ok and tostring(v) or v.kind))
        end)
    end
    bh.events.on("tick", function(ev)
        if ev.tick <= 3 then ask("t" .. ev.tick) end
    end)
end
return mod
"#;

const HUD_PROVIDES: &str =
    "\n[provides]\nservices = [{ name = \"hud\", version = \"1.0.0\", wit = \"wit/hud.wit\", remote = true }]\n";

/// A joined, modded session whose world is open and whose client mod `demo` runs `src`.
fn in_world(src: &str, perms: &str) -> Rig {
    let mut r = rig();
    let pkg = package("demo", "1.0.0", perms, "client/main.luau", src);
    let demo = info("demo", &pkg);
    let hash = mods_hash([(demo.id.as_str(), demo.version.as_str(), &demo.sha256)]);
    r.l.on_contact("10.0.2.2:15151");
    r.l.on_join_request();
    r.l.on_packet(&server_hello(hash, 1));
    let list = ModList { mods: vec![demo.clone()], channels: vec![], registry: Default::default() };
    r.l.on_packet(&Message::ModList(list).encode());
    r.l.on_ui(UiEvent::Decide(true));
    r.p.now.set(50);
    let chunk = Message::FileChunk(FileChunk { sha256: demo.sha256, offset: 0, total: demo.size, data: pkg });
    r.l.on_packet(&chunk.encode());
    r.l.on_world(Some("Sunny"));
    r.p.msgs();
    r
}

fn call(id: u32, service: &str, method: &str, args: Vec<u8>) -> Vec<u8> {
    let (service, method) = (service.to_string(), method.to_string());
    let call = ServiceCall { id, mod_id: "srv".into(), service, version_req: "^1".into(), method, args };
    Message::ServiceCall(call).encode()
}

#[test]
fn services_cross_the_wire_both_ways() {
    let mut r = in_world(CALLER, HUD_PROVIDES);
    assert!(r.p.logged("enabled"), "{:?}", r.p.logs.borrow());
    // tick 1: the mod calls the server; the call leaves as SERVICE_CALL
    r.p.now.set(400);
    r.l.on_frame();
    let m = r.p.msgs();
    let [Message::ServiceCall(c)] = &m[..] else { panic!("{m:?}") };
    assert_eq!((c.id, c.mod_id.as_str(), c.service.as_str(), c.method.as_str()), (1, "demo", "prices", "price"));
    assert_eq!(c.args, bhmod_core::value::Value::List(vec![bhmod_core::value::Value::text("t1")]).encode());
    // the server's answer reaches the callback
    let ok = ServiceResult { id: 1, outcome: Ok(bhmod_core::value::Value::Int(3).encode()) };
    r.l.on_net(NetEvent::Packet(Message::ServiceResult(ok).encode()));
    r.p.now.set(500);
    r.l.on_frame();
    assert!(r.p.logged("answer t1 true 3"), "{:?}", r.p.logs.borrow());
    // an answer for nothing is ignored
    let stray = ServiceResult { id: 77, outcome: Err(ServiceFailure { code: 4, message: String::new() }) };
    assert!(r.l.on_packet(&Message::ServiceResult(stray).encode()));

    // the server calls the client's `hud` service (exposed with remote = true); a bad call fails typed
    let args = bhmod_core::value::Value::List(vec![bhmod_core::value::Value::Int(7)]).encode();
    assert!(r.l.on_packet(&call(9, "hud", "title", args.clone())));
    assert!(r.l.on_packet(&call(10, "nothing", "x", args)));
    let m = r.p.msgs();
    let [Message::ServiceResult(a), Message::ServiceResult(b)] = &m[..] else { panic!("{m:?}") };
    assert_eq!((a.id, a.outcome.clone()), (9, Ok(bhmod_core::value::Value::text("title 7").encode())));
    assert_eq!((b.id, b.outcome.clone().unwrap_err().code), (10, bhmod_proto::service_error::NOT_FOUND));

    // tick 2 calls again; the connection drops before the answer: `disconnected`; tick 3 has nobody to ask
    r.p.now.set(600);
    r.l.on_frame();
    assert!(matches!(r.p.msgs()[..], [Message::ServiceCall(_)]));
    r.l.on_disconnect();
    assert!(r.p.logged("answer t2 false disconnected"), "{:?}", r.p.logs.borrow());
    r.p.now.set(800);
    r.l.on_frame();
    assert!(r.p.logged("answer t3 false unavailable"), "{:?}", r.p.logs.borrow());
    assert!(r.p.msgs().is_empty());
}

#[test]
fn service_packets_without_mods_are_ignored() {
    let mut r = rig();
    r.l.on_contact("h:1");
    // no host: the call is dropped, nothing is answered
    r.l.handle(vec![Output::ServiceCall(ServiceCall {
        id: 1,
        mod_id: "srv".into(),
        service: "s".into(),
        version_req: "*".into(),
        method: "m".into(),
        args: vec![0x80],
    })]);
    r.l.handle(vec![Output::ServiceResult(ServiceResult { id: 1, outcome: Ok(vec![0xf6]) })]);
    r.l.handle(vec![
        Output::Voice(bhmod_proto::Message::VoiceState(Default::default())),
        Output::SettingsResult(bhmod_proto::SettingsResult { outcome: Ok(()), mod_id: None, key: None }),
    ]);
    assert!(r.p.msgs().iter().all(|m| !matches!(m, Message::ServiceResult(_))));
}

#[test]
fn failures_abort_the_join() {
    let mut r = rig();
    let pkg = package("demo", "1.0.0", "", "client/main.luau", "return {}");
    let demo = info("demo", &pkg);
    r.l.on_contact("h:1");
    r.l.on_join_request();
    r.l.on_packet(&server_hello([3; 32], 1));
    let list = ModList { mods: vec![demo], channels: vec![], registry: Default::default() };
    r.l.on_packet(&Message::ModList(list).encode());
    r.l.on_ui(UiEvent::Decide(false));
    assert_eq!(r.p.aborts.get(), 1);
    assert_eq!(r.p.ui.borrow().last().unwrap()["code"], "declined");
    // REJECT_TEXT is shown
    let rt = RejectText { reason_code: 3, text: "update".into(), url: None };
    r.l.on_packet(&Message::RejectText(rt).encode());
    assert_eq!(r.p.ui_types().last().unwrap(), "reject");
    // with no session a UI answer does nothing
    r.l.on_disconnect();
    r.l.on_ui(UiEvent::Decide(true));
    r.l.on_ui(UiEvent::Cancel);
    assert_eq!(r.p.aborts.get(), 1);
    // cancel during a join
    r.l.on_contact("h:1");
    r.l.on_ui(UiEvent::Cancel);
    assert_eq!(r.p.aborts.get(), 2);
    assert_eq!(r.l.session_state(), "none");
}

#[test]
fn a_server_mod_that_does_not_start_fails_the_join() {
    let mut r = rig();
    // depends on a mod nobody has: resolve leaves it out
    let pkg = package("broken", "1.0.0", "\n[dependencies]\nmissing-mod = \"^1.0\"\n", "client/main.luau", "return {}");
    let m = info("broken", &pkg);
    r.l.on_contact("h:1");
    r.l.on_join_request();
    r.l.on_packet(&server_hello([3; 32], 1));
    r.l.on_packet(
        &Message::ModList(ModList { mods: vec![m.clone()], channels: vec![], registry: Default::default() }).encode(),
    );
    r.l.on_ui(UiEvent::Decide(true));
    let chunk = Message::FileChunk(FileChunk { sha256: m.sha256, offset: 0, total: m.size, data: pkg });
    r.l.on_packet(&chunk.encode());
    assert_eq!(r.p.aborts.get(), 1, "{:?} {:?}", r.p.ui.borrow(), r.p.logs.borrow());
    assert_eq!(r.p.ui.borrow().last().unwrap()["code"], "load-failed");
    assert!(r.p.logged("server mods did not start"));
}

#[test]
fn single_player_with_installed_mods() {
    let mut r = rig();
    // nothing installed: no host
    r.l.on_world(Some("Solo"));
    assert!(!r.l.running());
    r.l.on_world(None);
    let src = r#"
local mod = {}
function mod.on_enable()
    local ch = bh.channels.open("solo:x", function() end)
    bh.events.on("world-loaded", function(ev) bh.log.info(`hello {ev.world_name}`); bh.channels.send(ch, "server", "x") end)
end
return mod
"#;
    r.l.installed().install(&package("solo", "1.0.0", "", "client/main.luau", src), None).unwrap();
    r.l.on_world(Some("Solo"));
    assert!(r.l.running());
    assert!(r.p.logged("not connected to a modded server"));
    for t in [100, 300, 450] {
        r.p.now.set(t);
        r.l.on_frame();
    }
    r.l.on_world(None);
    assert!(!r.l.running());
}

#[test]
fn hot_reload_restarts_changed_mods_and_reports_timings() {
    let mut r = rig();
    assert_eq!(r.l.reload(None), Err("no mods are running".to_string()));
    assert!(r.l.timings(None).is_empty());
    assert!(!r.l.reset_timings(None));
    let src = |tag: &str| {
        format!(
            "local mod = {{}}\nfunction mod.on_enable()\n bh.events.on(\"world-loaded\", function(ev) bh.log.info(\"{tag} \" .. ev.world_name) end)\nend\nreturn mod\n"
        )
    };
    r.l.installed().install(&package("solo", "1.0.0", "", "client/main.luau", &src("one")), None).unwrap();
    r.l.on_world(Some("Solo"));
    assert!(r.l.running());
    assert!(r.p.logged("one Solo"));
    let t = r.l.timings(Some("solo"));
    assert_eq!(t.len(), 1);
    assert!(t[0].calls() >= 2, "{t:?}");
    // The player installs a newer version while the world is open.
    r.l.installed().install(&package("solo", "1.1.0", "", "client/main.luau", &src("two")), None).unwrap();
    let res = r.l.reload(None).unwrap();
    assert_eq!(res.len(), 1, "{res:?}");
    assert_eq!(res[0].to_string(), "solo: reloaded (1.0.0 -> 1.1.0)");
    assert!(r.p.logged("two Solo"));
    assert!(r.l.reload(None).unwrap().iter().all(|x| x.to_string() == "solo: unchanged"));
    assert!(r.l.reset_timings(Some("solo")));
    assert_eq!(r.l.timings(None)[0].calls(), 0);
    // A server package that vanished from the cache fails the reload and leaves the mods alone.
    let pkg = package("gone", "1.0.0", "", "client/main.luau", "return {}");
    let manifest = r.l.installed().install(&pkg, None).unwrap().manifest;
    r.l.accepted = vec![Accepted { info: info("gone", &pkg), path: r.dirs.cache.join("missing.bhmod"), manifest }];
    assert!(r.l.reload(None).is_err());
    assert!(r.l.running());
    // Nothing enabled any more: the mod is stopped.
    r.l.accepted = Vec::new();
    r.l.installed().set_enabled("solo", false).unwrap();
    r.l.installed().set_enabled("gone", false).unwrap();
    let res = r.l.reload(None).unwrap();
    assert!(res.iter().any(|x| x.to_string().starts_with("solo: stopped")), "{res:?}");
}

#[test]
fn installed_mods_that_cannot_be_copied_are_skipped() {
    let mut r = rig();
    r.l.installed().install(&package("solo", "1.0.0", "", "client/main.luau", "return {}"), None).unwrap();
    // the run directory cannot be created (a file is in the way)
    fs::write(&r.dirs.run, b"x").unwrap();
    std::fs::create_dir_all(r.dirs.run.parent().unwrap()).unwrap();
    let res = r.l.start_host(&[]);
    // remove_dir_all fails on a file, copy fails: nothing is granted, no host
    assert_eq!(res, Ok(vec![]));
    assert!(!r.l.running());
}

#[test]
fn a_server_mod_that_cannot_be_copied_fails_the_start() {
    let mut r = rig();
    let pkg = package("gone", "1.0.0", "", "client/main.luau", "return {}");
    let manifest = r.l.installed().install(&pkg, None).unwrap().manifest;
    // the cached package vanished between the check and the start
    let missing = r.dirs.cache.join("missing.bhmod");
    let accepted = Accepted { info: info("gone", &pkg), path: missing, manifest };
    let err = r.l.start_host(&[accepted]).unwrap_err();
    assert!(!err.is_empty());
    assert!(!r.l.running());
}

#[test]
fn bridge_answers() {
    let p = Rc::new(Fake::default());
    let t = tempfile::tempdir().unwrap();
    let device = DeviceStore::new(t.path());
    let b = Bridge {
        p: p.clone(),
        outbox: Rc::new(RefCell::new(Vec::new())),
        hud: Rc::default(),
        slot: Rc::default(),
        device: device.clone(),
    };
    assert_eq!(b.side(), Side::Client);
    assert!(b.host_name().starts_with("bhmod-android "));
    b.log(None, LogLevel::Info, "hi");
    assert!(p.logged("[loader] hi"));
    assert!(b.online_players().is_empty());
    assert_eq!(b.player_alias("x"), None);
    assert_eq!(b.player_role("x"), Role::Player);
    assert!(!b.send_chat("x", "y"));
    b.broadcast("z");
    assert!(!b.kick("x", "y"));
    assert!(!b.is_builtin_command("help"));
    assert_eq!(b.channel_send("m", "m:c", &ChannelTarget::AllPlayers, b"x"), Err(ApiError::Unsupported));
    assert_eq!(b.channel_send("m", "m:c", &ChannelTarget::Server, b"x"), Ok(()));
    assert_eq!(b.ui("m", &bhmod_core::ui::UiCommand::Toast { text: "t".into() }), Ok(()));
    assert_eq!(p.ui.borrow()[0]["t"], "ui-toast");
    // API 0.3.4: overhead icons stay in the loader, buttons get their saved position, settings screens are queued
    use bhmod_core::ui::{Button, OverheadIcon};
    let icon = OverheadIcon::Speaking;
    b.ui("m", &UiCommand::SetOverhead { player: "p1".into(), icon }).unwrap();
    assert!(b.hud.borrow().board.active());
    b.ui("m", &UiCommand::ClearOverhead { player: "p1".into() }).unwrap();
    assert!(!b.hud.borrow().board.active());
    b.ui("m", &UiCommand::SetOverhead { player: "p1".into(), icon }).unwrap();
    b.ui("m", &UiCommand::ClearMod).unwrap();
    assert!(!b.hud.borrow().board.active());
    device.set_button_pos("m", "ptt", 0.1, 0.2).unwrap();
    let button = Button { icon: "mic".into(), label: None, x: 0.9, y: 0.7, size: 64, opacity: 80 };
    b.ui("m", &UiCommand::SetButton { id: "ptt".into(), button }).unwrap();
    let last = p.ui.borrow().last().cloned().unwrap();
    assert_eq!((last["t"].as_str(), last["saved"][0].as_f64()), (Some("btn-set"), Some(0.1f32 as f64)));
    b.ui("m", &UiCommand::OpenSettings { mod_id: "other".into() }).unwrap();
    assert_eq!(b.hud.borrow().open_settings, vec!["other".to_string()]);
    assert!(b.voice().is_none());
    let link = Rc::new(crate::voice::link::fake::FakeLink::default());
    link.names.borrow_mut().push(("p1".into(), "ALICE".into()));
    b.slot.set(Some(link));
    assert!(b.voice().is_some());
    assert_eq!(b.online_players(), ["p1"]);
    assert_eq!(b.player_alias("p1").as_deref(), Some("ALICE"));
    assert_eq!(b.player_alias("x"), None);
    assert_eq!(b.local_player().as_deref(), Some("me"));
    assert_eq!(b.platform(), "android-armv7");
    assert!(b.has_touch());
    let bind = InputCommand::Bind { action: "talk".into(), default_key: "v".into() };
    assert_eq!(b.input("m", &bind), Ok(()));
    assert_eq!(b.input_key("m", "talk", "v"), Some("v".into()));
    device.set_binding("m", "talk", Some(None)).unwrap();
    assert_eq!(b.input_key("m", "talk", "v"), None);
}

#[test]
fn mod_ui_reaches_java_and_dialog_answers_come_back() {
    let mut r = rig();
    let src = r#"
local mod = {}
function mod.on_enable()
    bh.ui.set_widget("coins", { slot = "top-right", kind = "counter", icon = "coin", text = "5" })
    bh.ui.remove_widget("coins")
    bh.ui.notice("hello", "success")
    bh.ui.show_dialog({ title = "Q", text = "?", field = { initial = "1" }, buttons = { { label = "OK", kind = "primary" } } })
    bh.events.on_custom("ui:ui-action", function(ev)
        bh.ui.toast("answered " .. ev.payload.button .. " " .. tostring(ev.payload.text))
    end)
end
return mod
"#;
    r.l.installed().install(&package("ui", "1.0.0", "", "client/main.luau", src), None).unwrap();
    r.l.on_world(Some("Solo"));
    assert!(r.l.running());
    assert_eq!(r.p.ui_types(), ["ui-set", "ui-remove", "ui-notice", "ui-dialog", "world"]);
    let ui = r.p.ui.borrow().clone();
    assert_eq!(ui[0]["mod"], "ui");
    assert_eq!(ui[0]["slot"], "top-right");
    assert_eq!(ui[0]["kind"], "counter");
    assert_eq!(ui[0]["icon"], "coin");
    assert_eq!(ui[1]["id"], "coins");
    assert_eq!(ui[2]["level"], "success");
    assert_eq!(ui[3]["field"]["initial"], "1");
    assert_eq!(ui[3]["buttons"][0]["kind"], "primary");
    r.p.ui.borrow_mut().clear();
    // the answer is delivered to the mod, which answers with a toast
    r.l.on_ui(UiEvent::Dialog { dialog: 1, button: 0, text: Some("7".into()) });
    assert_eq!(r.p.ui.borrow()[0]["t"], "ui-toast");
    assert_eq!(r.p.ui.borrow()[0]["text"], "answered 0 7");
    // nothing of the mod is left on screen, so leaving the world sends no clear; an answer with no mods is ignored
    r.l.on_world(None);
    assert!(r.p.ui.borrow().iter().all(|v| v["t"] != "ui-clear"));
    r.l.on_ui(UiEvent::Dialog { dialog: 1, button: 0, text: None });
}

#[test]
fn dirs_layout() {
    let d = Dirs::under(Path::new("/d"));
    assert_eq!(d.cache, Path::new("/d/cache"));
    assert_eq!(d.storage, Path::new("/d/storage"));
}

#[test]
fn installed_mods_that_cannot_start_are_logged() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = rig();
    r.l.installed().install(&package("solo", "1.0.0", "", "client/main.luau", "return {}"), None).unwrap();
    // `run/mods.toml` is a directory that cannot be emptied, so the run directory cannot be rebuilt
    let locked = r.dirs.run.join(bhmod_core::config::FILE_NAME);
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("x"), b"x").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    r.l.on_world(Some("Solo"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(r.p.logged("installed mods did not start"), "{:?}", r.p.logs.borrow());
}

#[test]
fn cosmetics_follow_the_registry_and_the_server() {
    let mut r = rig();
    let pkg = content_package("crowns");
    let m = info("crowns", &pkg);
    let hash = mods_hash([(m.id.as_str(), m.version.as_str(), &m.sha256)]);
    r.l.on_contact("10.0.2.2:15151");
    r.l.on_join_request();
    r.l.on_packet(&server_hello(hash, 1));
    let registry = std::collections::BTreeMap::from([(
        "cosmetics".to_string(),
        std::collections::BTreeMap::from([("crowns:crown".to_string(), 4u64)]),
    )]);
    let list = ModList { mods: vec![m.clone()], channels: vec![], registry };
    r.l.on_packet(&Message::ModList(list).encode());
    r.l.on_ui(UiEvent::Decide(true));
    let chunk = Message::FileChunk(FileChunk { sha256: m.sha256, offset: 0, total: m.size, data: pkg });
    r.l.on_packet(&chunk.encode());
    assert!(r.l.content().lock().unwrap().is_active());
    // COSMETICS after the join: player p wears the crown (on an empty hat slot it wears the base item).
    let wear = Message::Cosmetics(bhmod_proto::Cosmetics {
        full: true,
        players: [("p".to_string(), [("hat".to_string(), 4u64)].into())].into(),
    });
    r.l.on_packet(&wear.encode());
    let look = r.l.content().lock().unwrap().look("p", [None; 4]);
    assert_eq!(look.clothing[3], Some((4, Some(115))));
    let tex = r.l.content().lock().unwrap().texture(4, bhmod_content::Part::Main).unwrap().unwrap();
    assert_eq!((tex.wide, tex.high), (2, 2));
    // The world closing ends it all.
    r.l.on_world(Some("Sunny"));
    r.l.on_world(None);
    assert!(!r.l.content().lock().unwrap().is_active());
    assert!(r.l.content().lock().unwrap().look("p", [None; 4]).is_empty());
}

#[test]
fn a_broken_content_package_is_reported_not_fatal() {
    let mut r = rig();
    let junk = r._t.path().join("junk.bhmod");
    std::fs::write(&junk, b"not a zip").unwrap();
    r.l.load_content(&[junk]);
    assert!(r.p.logged("content:"), "{:?}", r.p.logs.borrow());
    assert!(!r.l.content().lock().unwrap().is_active());
}

const LIB: &str = "client/native/armeabi-v7a/libnat.so";

fn native_package(id: &str, libs: &[(&str, &[u8])]) -> Vec<u8> {
    package_files(id, "1.0.0", "native = true\n", "client/main.luau", "return {}", libs)
}

#[test]
fn confirmed_native_mods_load_with_the_world_and_unload_with_it() {
    let mut r = rig();
    let index = Some("index:https://x/nat.bhmod");
    r.l.installed().install_with(&native_package("nat", &[(LIB, b"ELF")]), index, true).unwrap();
    r.l.installed().install(&package("plain", "1.0.0", "", "client/main.luau", "return {}"), None).unwrap();
    r.l.on_world(Some("Solo"));
    assert_eq!(*r.p.natives.borrow(), [("nat".to_string(), "libnat.so".to_string(), true)]);
    assert!(r.p.logged("native code loaded"));
    assert!(r.p.unloaded.borrow().is_empty());
    // the extracted file is where the Platform was told
    let sha = hex(&sha256(&fs::read(r.l.installed().list()[0].path.clone()).unwrap()));
    assert!(r.dirs.native.join(sha).join("armeabi-v7a/libnat.so").is_file());
    r.l.on_world(None);
    assert_eq!(*r.p.unloaded.borrow(), ["nat"]);
    // the next world loads it again
    r.l.on_world(Some("Solo"));
    assert_eq!(r.p.natives.borrow().len(), 2);
}

#[test]
fn native_failures_are_logged_and_undone() {
    let mut r = rig();
    let index = Some("index:https://x/nat.bhmod");
    // a failing init, a package without a library for this ABI
    r.l.installed().install_with(&native_package("bad", &[(LIB, b"ELF")]), index, true).unwrap();
    r.l.installed()
        .install_with(&native_package("empty", &[("client/native/x86/libn.so", b"ELF")]), index, true)
        .unwrap();
    r.p.native_fail.borrow_mut().push("bad".into());
    r.l.on_world(Some("Solo"));
    assert!(r.p.logged("native code failed: init returned 3"));
    assert!(r.p.logged("empty: no armeabi-v7a library"));
    assert_eq!(*r.p.unloaded.borrow(), ["bad"]);
    r.l.on_world(None);
    // nothing was left to unload at the end
    assert_eq!(*r.p.unloaded.borrow(), ["bad"]);
}

#[test]
fn native_code_that_is_not_the_confirmed_file_is_not_loaded() {
    let mut r = rig();
    let index = Some("index:https://x/nat.bhmod");
    let m = r.l.installed().install_with(&native_package("nat", &[(LIB, b"ELF")]), index, true).unwrap();
    // a package of a mod the player installed natively but whose file is now another one: the installed list hides
    // it, and a direct candidate with an unconfirmed hash is refused by the loader itself
    let evil = native_package("nat", &[(LIB, b"EVIL")]);
    let evil_path = r.dirs.run.join("evil.bhmod");
    fs::create_dir_all(&r.dirs.run).unwrap();
    fs::write(&evil_path, &evil).unwrap();
    r.l.start_natives(
        &[("nat".into(), evil_path), ("gone".into(), r.dirs.run.join("none.bhmod"))],
        &[("nat".into(), "1.0.0".into()), ("gone".into(), "1".into())],
    );
    assert!(r.p.logged("not an installed, confirmed package"));
    assert!(r.p.natives.borrow().is_empty());
    // the unpack step can fail (the native folder is a file)
    fs::write(&r.dirs.native, b"x").unwrap();
    r.l.start_natives(&[("nat".into(), m.path)], &[("nat".into(), "1.0.0".into())]);
    assert!(r.p.logged("cannot unpack the native library"));
}

#[test]
fn a_server_may_list_native_code_only_when_the_player_installed_that_file() {
    let mut r = rig();
    let pkg = native_package("nat", &[(LIB, b"ELF")]);
    let index = Some("index:https://x/nat.bhmod");
    let mut m = info("nat", &pkg);
    m.permissions.native = true;
    let hash = mods_hash([(m.id.as_str(), m.version.as_str(), &m.sha256)]);
    // not installed: refused at the prompt answer
    let flow = |r: &mut Rig| {
        r.l.on_contact("10.0.2.2:15151");
        r.l.on_join_request();
        r.l.on_packet(&server_hello(hash, 1));
        r.l.on_packet(
            &Message::ModList(ModList { mods: vec![m.clone()], channels: vec![], registry: Default::default() })
                .encode(),
        );
        r.l.on_ui(UiEvent::Decide(true));
    };
    flow(&mut r);
    assert!(r.p.logged("modded join stopped"), "{:?}", r.p.logs.borrow());
    r.l.on_disconnect();
    // installed and confirmed: the cached file loads
    r.l.installed().install_with(&pkg, index, true).unwrap();
    flow(&mut r);
    assert_eq!(r.p.natives.borrow().len(), 1, "{:?}", r.p.logs.borrow());
}

// ---- API 0.3.4: voice, buttons, keys, overhead icons, settings screen -------------------------------------------

use crate::voice::audio::fake::FakeBackend;
use crate::voice::link::fake::FakeLink;
use crate::voice::pipeline::Outgoing as VoiceOut;
use bhmod_core::voice::{Icons, MicState, Mode, VoiceEvent, VoiceState as CoreState};
use bhmod_proto::{SettingsFailure, SettingsSchema, VoiceState};

const VOICE_CLIENT: &str = r#"
local mod = {}
function mod.on_enable()
    bh.input.bind("talk", "v")
    bh.ui.set_button("ptt", { icon = "mic", label = "Talk" })
    bh.events.on_custom("vtest:ui-button", function(ev)
        bh.log.info("button " .. ev.payload.action)
        if ev.payload.action == "press" then bh.voice.set_mic(true) end
        if ev.payload.action ~= "long-press" then bh.voice.talk(nil, ev.payload.action == "press") end
    end)
    bh.events.on_custom("vtest:input", function(ev)
        bh.log.info("key " .. ev.payload.action .. " " .. tostring(ev.payload.pressed))
    end)
    bh.events.on_custom("voice:mic", function(ev) bh.log.info("mic " .. ev.payload.state) end)
    bh.events.on_custom("voice:state", function(ev)
        local p = bh.players.get(ev.payload.player)
        if p then
            bh.ui.set_overhead(p, "speaking")
            bh.ui.open_settings("vtest")
        end
    end)
    bh.events.on_custom("vtest:settings-changed", function(_ev) bh.log.info("settings changed") end)
end
return mod
"#;

const VOICE_PERMS: &str =
    "voice = true\n\n[settings.ring]\ntype = \"bool\"\ndefault = false\nlabel = \"Ring\"\nscope = \"client\"\n";

fn hello_with(hash: Hash, count: u64, features: &[&str]) -> Vec<u8> {
    Message::ServerHello(ServerHello {
        proto: 1,
        mods_hash: hash,
        client_required: true,
        count,
        players: None,
        features: features.iter().map(|f| f.to_string()).collect(),
        voice_udp_port: None,
    })
    .encode()
}

/// A loader with a voice client and an audio backend, joined to a server listing `features` (no server mods) and
/// an installed client mod `vtest` holding `voice`.
fn voice_rig(features: &[&str], audio: bool) -> (Rig, Rc<FakeLink>) {
    voice_rig_mods(features, audio, true)
}

fn voice_rig_mods(features: &[&str], audio: bool, with_mod: bool) -> (Rig, Rc<FakeLink>) {
    let t = tempfile::tempdir().unwrap();
    let dirs = Dirs::under(t.path());
    let p = Rc::new(Fake::default());
    if audio {
        *p.backend.borrow_mut() = Some(FakeBackend::default());
    }
    let link = Rc::new(FakeLink::default());
    let handle = link.clone();
    let l =
        Loader::new(p.clone(), dirs.clone(), crate::caps(), crate::options(), Box::new(bhmod_core::default_runtimes))
            .with_budgets(crate::budgets())
            .with_voice(Box::new(move || Some(handle.clone() as Rc<dyn VoiceLink>)));
    let mut r = Rig { l, p, _t: t, dirs };
    let pkg =
        crate::testutil::package_api("vtest", "1.0.0", "^0.3.4", VOICE_PERMS, "client/main.luau", VOICE_CLIENT, &[]);
    if with_mod {
        r.l.installed().install(&pkg, None).unwrap();
    }
    r.l.on_contact("127.0.0.1:15151");
    let Message::Hello(h) = &r.p.msgs()[0] else { panic!() };
    assert_eq!(h.features, ["voice", "settings"]);
    r.l.on_join_request();
    r.l.on_packet(&hello_with([0; 32], 0, features));
    assert_eq!(r.p.joins.borrow().len(), 1);
    (r, link)
}

fn ui_of(r: &Rig, t: &str) -> Vec<Value> {
    r.p.ui.borrow().iter().filter(|v| v["t"] == t).cloned().collect()
}

#[test]
fn voice_flows_through_the_pipeline_and_the_mods() {
    let (mut r, link) = voice_rig(&["voice", "settings"], true);
    std::fs::create_dir_all(&r.dirs.device).unwrap();
    std::fs::write(r.dirs.device.join("voice-prefs.txt"), "master=90").unwrap();
    r.l.on_world(Some("w"));
    assert!(r.l.running());
    // VOICE_KEY went out reliably, the prefs were loaded, the button was drawn and the mod's settings can open
    assert!(r.p.sent.borrow().iter().any(|p| p == &[0xBC, 0x15]));
    assert_eq!(link.prefs(), "master=90");
    assert_eq!(ui_of(&r, "btn-set")[0]["icon"], "mic");
    assert_eq!(ui_of(&r, "world")[0]["settings"][0], "vtest");
    // the player presses the button: the mod opens the mic (first time: the system dialog), talks
    r.l.on_ui(UiEvent::Button { mod_id: "vtest".into(), id: "ptt".into(), action: "press".into() });
    r.l.on_ui(UiEvent::Button { mod_id: "vtest".into(), id: "ptt".into(), action: "bogus".into() });
    assert!(r.p.logged("button press"));
    assert_eq!(*link.talks.borrow(), [(None, true)]);
    r.p.now.set(10);
    r.l.on_frame();
    assert_eq!(r.p.asked.get(), 1);
    r.l.on_ui(UiEvent::MicPermission(true));
    assert!(r.p.logged("mic on"), "{:?}", r.p.logs.borrow());
    // captured audio reaches the client, its packets leave on the right paths, the indicator shows
    let backend = r.p.backend.borrow().clone().unwrap();
    let mut capture = backend.0.lock().unwrap().capture.take().unwrap();
    capture(&[0; crate::voice::audio::FRAME]);
    link.talking.set(Some(Mode::Proximity));
    link.send.borrow_mut().extend([
        VoiceOut::Reliable(vec![0xBC, 0x18]),
        VoiceOut::Unsequenced(vec![0xBC, 0x16, 1]),
        VoiceOut::Udp(vec![1]),
    ]);
    r.p.now.set(20);
    r.l.on_frame();
    assert_eq!(link.frames.get(), 1);
    assert!(r.p.sent.borrow().iter().any(|p| p == &[0xBC, 0x18]));
    assert_eq!(*r.p.unsequenced.borrow(), [vec![0xBC, 0x16, 1]]);
    assert_eq!(ui_of(&r, "voice-mic")[0]["mode"], "proximity");
    // a UDP datagram without a port announced goes nowhere; with one, the socket opens and answers come back
    let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    link.udp_port.set(Some(server.local_addr().unwrap().port()));
    link.send.borrow_mut().push(VoiceOut::Udp(vec![2]));
    r.l.on_frame();
    let mut buf = [0u8; 8];
    let (n, from) = server.recv_from(&mut buf).unwrap();
    assert_eq!(&buf[..n], [2]);
    link.send.borrow_mut().push(VoiceOut::Udp(vec![3]));
    r.l.on_frame();
    assert_eq!(server.recv_from(&mut buf).unwrap().0, 1);
    server.send_to(&[7], from).unwrap();
    for _ in 0..100 {
        r.l.on_frame();
        if !link.udp_in.borrow().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(*link.udp_in.borrow(), [vec![7]]);
    // VOICE_STATE: the allowed modes go to the picker once, players become known, the client gets the message
    let state = |full, players: &[(&str, &str)], modes| {
        Output::Voice(Message::VoiceState(VoiceState {
            full,
            players: players.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            modes,
            ..Default::default()
        }))
    };
    r.l.handle(vec![state(true, &[("p1", "ready")], Some(0b0101)), state(false, &[], Some(0b0101))]);
    r.l.handle(vec![Output::Voice(Message::ListRequest)]);
    assert_eq!(ui_of(&r, "voice-modes").len(), 1);
    assert_eq!(ui_of(&r, "voice-modes")[0]["modes"], serde_json::json!(["proximity", "channel"]));
    assert_eq!(link.messages.borrow().len(), 3);
    // a state event: the mod puts an icon over p1 and asks for its settings screen
    link.names.borrow_mut().push(("p1".into(), "ALICE".into()));
    link.events.borrow_mut().push(VoiceEvent::State { player: "p1".into(), state: CoreState::Talking });
    r.p.now.set(100);
    r.l.on_frame();
    assert_eq!(ui_of(&r, "settings").len(), 1, "{:?}", r.p.logs.borrow());
    // the overhead icon: no camera yet -> an empty list; with the camera and a tracked blockhead -> its position
    assert!(r.l.tracker().active());
    let mut p = [0.0f32; 16];
    (p[0], p[5], p[10], p[15], p[12], p[13]) = (0.02, 0.04, 1.0, 1.0, -1.0, -1.0);
    let mut id = [0.0f32; 16];
    (id[0], id[5], id[10], id[15]) = (1.0, 1.0, 1.0, 1.0);
    r.p.camera.set(Some(Camera { projection: p, modelview: id, viewport: [0, 0, 100, 50] }));
    r.l.tracker().report(1, "p1", (50.0, 10.0), (0, 0), 150);
    r.p.now.set(200);
    r.l.on_frame();
    let heads = ui_of(&r, "overhead");
    assert_eq!(heads.last().unwrap()["items"][0]["icons"][0], "speaking", "{heads:?}");
    // the admin hides "speaking": the icon goes, and the tracking stops once no icon is left
    link.icons.set(Some(Icons { speaking: false, ..Default::default() }));
    r.p.now.set(300);
    r.l.on_frame();
    assert!(ui_of(&r, "overhead").last().unwrap()["items"].as_array().unwrap().is_empty());
    // the long-press mode picker sets the default mode; a bad name is ignored; the ptt seam talks directly
    r.l.on_ui(UiEvent::VoiceMode("global".into()));
    r.l.on_ui(UiEvent::VoiceMode("shout".into()));
    assert_eq!(link.default_mode.get(), Some(Mode::Global));
    r.l.on_ui(UiEvent::Ptt(false));
    assert_eq!(link.talks.borrow().last(), Some(&(None, false)));
    // a dropped button is remembered on the device
    r.l.on_ui(UiEvent::ButtonMoved { mod_id: "vtest".into(), id: "ptt".into(), x: 0.3, y: 0.4 });
    assert_eq!(DeviceStore::new(&r.dirs.device).button_pos("vtest", "ptt"), Some((0.3, 0.4)));
    // a hardware key bound by the mod
    r.l.on_ui(UiEvent::Key("v".into(), true));
    r.l.on_ui(UiEvent::Key("v".into(), false));
    assert!(r.p.logged("key talk true") && r.p.logged("key talk false"));
    // the preferences are saved when they changed (every second, and when voice stops)
    link.set_master(120).unwrap();
    r.p.now.set(2000);
    r.l.on_frame();
    assert_eq!(DeviceStore::new(&r.dirs.device).voice_prefs(), "master=120");
    link.set_master(130).unwrap();
    link.talking.set(None);
    r.l.on_world(None);
    assert_eq!(DeviceStore::new(&r.dirs.device).voice_prefs(), "master=130");
    assert!(r.p.logged("mic off"));
    assert!(!r.l.running());
    assert!(ui_of(&r, "world").last().unwrap()["settings"].as_array().unwrap().is_empty());
    // the icons went with the mods: the overlay is cleared once
    let before = ui_of(&r, "overhead").len();
    r.l.on_frame();
    r.l.on_frame();
    assert_eq!(ui_of(&r, "overhead").len(), before + 1);
}

fn last_result(r: &Rig) -> Value {
    ui_of(r, "settings-result").last().cloned().unwrap()
}

#[test]
fn the_settings_screen_sets_client_keys_voice_and_server_values() {
    let (mut r, link) = voice_rig(&["voice", "settings"], true);
    r.l.on_world(Some("w"));
    r.l.on_ui(UiEvent::OpenSettings("vtest".into()));
    let screen = ui_of(&r, "settings").pop().unwrap();
    assert_eq!(screen["client"]["fields"][0]["key"], "ring");
    assert_eq!(screen["keys"][0]["key"], "v");
    assert_eq!(screen["voice"]["master"], 0);
    assert!(screen["server"].is_null());
    // client scope: stored on the device, the mod hears about it
    let set = |r: &mut Rig, scope: &str, key: &str, json: &str| {
        r.l.on_ui(UiEvent::Setting { mod_id: "vtest".into(), scope: scope.into(), key: key.into(), json: json.into() })
    };
    set(&mut r, "client", "ring", "true");
    assert_eq!(last_result(&r)["ok"], true);
    assert_eq!(ui_of(&r, "settings").last().unwrap()["client"]["values"]["ring"], true);
    assert!(std::fs::read_to_string(r.dirs.device.join("client-settings.toml")).unwrap().contains("ring"));
    r.p.now.set(300);
    r.l.on_frame();
    assert!(r.p.logged("settings changed"), "{:?}", r.p.logs.borrow());
    set(&mut r, "client", "ring", "3");
    assert_eq!(last_result(&r)["ok"], false);
    set(&mut r, "client", "ring", "{");
    assert_eq!(last_result(&r)["message"], "not a value");
    // keys: rebind, clear, back to the default, nonsense
    set(&mut r, "keys", "talk", "\"b\"");
    assert_eq!(ui_of(&r, "settings").last().unwrap()["keys"][0]["key"], "b");
    assert_eq!(ui_of(&r, "settings").last().unwrap()["keys"][0]["isDefault"], false);
    set(&mut r, "keys", "talk", "null");
    assert!(ui_of(&r, "settings").last().unwrap()["keys"][0]["key"].is_null());
    set(&mut r, "keys", "talk", "false");
    assert_eq!(ui_of(&r, "settings").last().unwrap()["keys"][0]["key"], "v");
    set(&mut r, "keys", "talk", "\"hyper\"");
    assert_eq!(last_result(&r)["message"], "not a key");
    // voice master volume
    set(&mut r, "voice", "master", "150");
    assert_eq!(link.master.get(), 150);
    set(&mut r, "voice", "master", "250");
    set(&mut r, "voice", "master", "\"x\"");
    assert_eq!(last_result(&r)["ok"], false);
    set(&mut r, "other", "x", "1");
    assert_eq!(last_result(&r)["message"], "unknown section");
    // server scope: the admin gets the form; a change goes out as SETTINGS_SET; the answer comes back
    let schema = serde_json::json!({"fields": [{"key": "radius", "type": "int", "default": 32, "label": "R", "scope": "server", "restart": false}]});
    let values = serde_json::json!({"radius": 32});
    r.l.handle(vec![Output::SettingsSchema(SettingsSchema {
        mod_id: "vtest".into(),
        schema: bhmod_core::settings::to_cbor(&schema),
        values: bhmod_core::settings::to_cbor(&values),
    })]);
    assert_eq!(ui_of(&r, "settings").last().unwrap()["server"]["values"]["radius"], 32);
    r.l.handle(vec![Output::SettingsSchema(SettingsSchema {
        mod_id: "srvonly".into(),
        schema: bhmod_core::settings::to_cbor(&schema),
        values: vec![0xff],
    })]);
    assert!(r.p.logged("unreadable form"));
    r.p.sent.borrow_mut().clear();
    set(&mut r, "server", "radius", "64");
    assert!(matches!(&r.p.msgs()[..], [Message::SettingsSet(s)] if s.key == "radius"));
    r.l.handle(vec![Output::SettingsResult(bhmod_proto::SettingsResult {
        outcome: Ok(()),
        mod_id: Some("vtest".into()),
        key: Some("radius".into()),
    })]);
    assert_eq!((last_result(&r)["ok"].as_bool(), last_result(&r)["key"].as_str()), (Some(true), Some("radius")));
    let refused = SettingsFailure { code: 1, message: None };
    r.l.handle(vec![Output::SettingsResult(bhmod_proto::SettingsResult {
        outcome: Err(refused),
        mod_id: None,
        key: None,
    })]);
    assert_eq!(last_result(&r)["mod"], "vtest", "an old server: the open screen's mod");
    assert_eq!(last_result(&r)["message"], "refused (code 1)");
    // after leaving the world: nothing runs, the server is gone
    r.l.on_world(None);
    set(&mut r, "client", "ring", "false");
    assert_eq!(last_result(&r)["message"], "no mods are running");
    set(&mut r, "server", "radius", "1");
    assert_eq!(last_result(&r)["ok"], false);
    r.l.on_ui(UiEvent::Button { mod_id: "vtest".into(), id: "ptt".into(), action: "press".into() });
    r.l.on_ui(UiEvent::Key("v".into(), true));
    r.l.on_ui(UiEvent::MicPermission(true));
    r.l.handle(vec![Output::SettingsResult(bhmod_proto::SettingsResult { outcome: Ok(()), mod_id: None, key: None })]);
    // the unsaved device: a button position and prefs that cannot be written are logged
    std::fs::remove_dir_all(&r.dirs.device).unwrap();
    std::fs::write(&r.dirs.device, b"file").unwrap();
    r.l.on_ui(UiEvent::ButtonMoved { mod_id: "vtest".into(), id: "ptt".into(), x: 0.3, y: 0.4 });
    assert!(r.p.logged("button position not saved"));
    set(&mut r, "keys", "talk", "null");
    assert_eq!(last_result(&r)["ok"], false);
}

#[test]
fn voice_stays_off_when_it_cannot_run() {
    // the server does not list voice
    let (mut r, link) = voice_rig(&["settings"], true);
    r.l.on_world(Some("w"));
    assert!(!r.p.sent.borrow().iter().any(|p| p == &[0xBC, 0x15]));
    assert!(link.messages.borrow().is_empty());
    r.l.handle(vec![Output::Voice(Message::VoiceState(VoiceState::default()))]);
    assert!(link.messages.borrow().is_empty());
    // no audio backend on the phone
    let (mut r, _link) = voice_rig(&["voice"], false);
    r.l.on_world(Some("w"));
    assert!(r.p.logged("no audio backend"));
    // no key could be made
    let (mut r, link) = voice_rig(&["voice"], true);
    link.no_key.set(true);
    r.l.on_world(Some("w"));
    assert!(r.p.logged("no key could be made"));
    // without a voice factory nothing starts either
    let mut r = rig();
    r.l.on_contact("h:1");
    r.l.on_join_request();
    r.l.on_packet(&hello_with([0; 32], 0, &["voice"]));
    r.l.on_world(Some("w"));
    r.l.on_world(None);
}

#[test]
fn denial_failures_udp_trouble_and_disconnect() {
    // no mod runs: voice still works for the player (events have nowhere to go)
    let (mut r, link) = voice_rig_mods(&["voice", "settings"], true, false);
    r.l.on_world(Some("w"));
    r.l.on_world(Some("w"));
    link.want.set(true);
    r.l.on_frame();
    assert_eq!(r.p.asked.get(), 1);
    r.l.on_ui(UiEvent::MicPermission(false));
    assert_eq!(ui_of(&r, "voice-denied").len(), 1);
    assert_eq!(*link.mic.borrow(), [MicState::Denied]);
    // the server's voice port cannot be reached
    r.l.server = "no such host.invalid:1".into();
    link.udp_port.set(Some(1));
    link.send.borrow_mut().push(VoiceOut::Udp(vec![1]));
    r.l.on_frame();
    assert!(r.p.logged("unreachable"));
    // playback that cannot open is logged once
    let (mut r2, _l2) = voice_rig(&["voice"], true);
    r2.p.backend.borrow().as_ref().unwrap().0.lock().unwrap().fail = true;
    r2.l.on_world(Some("w"));
    r2.l.on_frame();
    assert!(r2.p.logged("voice: playback"));
    // unchanged preferences are not written again
    r.p.now.set(1500);
    r.l.on_frame();
    assert!(!r.dirs.device.join("voice-prefs.txt").exists());
    // preferences that cannot be saved are logged
    std::fs::create_dir_all(r.dirs.device.parent().unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&r.dirs.device);
    std::fs::write(&r.dirs.device, b"file").unwrap();
    link.set_master(77).unwrap();
    // the connection drops: voice stops at once
    r.l.on_disconnect();
    assert!(r.p.logged("preferences not saved"));
    assert!(r.l.slot.get().is_none());
    r.l.on_frame();
}

#[test]
fn skipped_optional_mods_are_told_to_the_player() {
    let mut r = rig();
    r.l.on_contact("h:1");
    let m = bhmod_client::session::SkippedMod {
        id: "voice".into(),
        version: "1.0.0".into(),
        name: Default::default(),
        refusal: bhmod_client::Refusal::NeedsNewerApi("^0.4".into()),
    };
    r.l.handle(vec![Output::Skipped(vec![m])]);
    assert!(r.p.logged("cannot run: [\"voice\"]"));
    let s = ui_of(&r, "skipped");
    assert_eq!((s[0]["server"].as_str(), s[0]["mods"][0]["id"].as_str()), (Some("h:1"), Some("voice")));
}
