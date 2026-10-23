use std::cell::Cell;

use super::*;
use crate::testutil::{content_package, package, package_files};
use bhmod_client::cache::sha256;
use bhmod_proto::{
    mods_hash, Channel, ChannelInfo, FileChunk, Message, ModInfo, ModList, ModPermissions, RejectText, ServerHello,
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
    Message::ServerHello(ServerHello { proto: 1, mods_hash: hash, client_required: true, count }).encode()
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
    let b = Bridge { p: p.clone(), outbox: Rc::new(RefCell::new(Vec::new())) };
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
    PackageCache::new(&r.dirs.cache).put(&pkg).unwrap();
    flow(&mut r);
    assert_eq!(r.p.natives.borrow().len(), 1, "{:?}", r.p.logs.borrow());
}
