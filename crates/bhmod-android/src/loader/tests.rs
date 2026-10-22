use std::cell::Cell;

use super::*;
use crate::testutil::package;
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
        Loader::new(p.clone(), dirs.clone(), crate::caps(), crate::options(), Box::new(bhmod_core::default_runtimes));
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
    r.l.on_contact("10.0.2.2:15151");
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
    r.l.on_disconnect();
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
    // the server echoes; the client mod gets it on the next frame
    r.l.on_packet(&Message::Channel(Channel { channel_id: 7, payload: b"echo:ping".to_vec() }).encode());
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
