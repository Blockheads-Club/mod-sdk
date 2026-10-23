//! The JVM runtime through the real mod host: events, the whole API, mutations, services between Java mods,
//! channels, and every way a mod can fail (exceptions, errors, timeouts). The fixtures are in `jvm-fixtures`.
//!
//! Needs a JDK (javac) and a JRE at run time.
#![cfg(feature = "jvm")]

mod common;
#[path = "common/java.rs"]
mod java;

use std::rc::Rc;
use std::time::Duration;

use bhmod_core::jvm::{JvmConfig, JvmRuntime};
use bhmod_core::runtime::{InstanceLimits, ModFiles, Runtime};
use bhmod_core::types::*;
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;
use java::*;

struct T {
    _tmp: tempfile::TempDir,
    b: Rc<FakeBridge>,
    h: ModHost,
}

fn runtime(grace_ms: u64) -> Box<dyn Runtime> {
    let cfg = JvmConfig { grace: Duration::from_millis(grace_ms), ..JvmConfig::default() };
    Box::new(JvmRuntime::new(cfg).unwrap())
}

/// A host with the given JVM mods: (id, main class, extra mod.toml).
fn host(mods: &[(&str, &str, &str)], grants: &str, tune: impl FnOnce(&mut HostOptions)) -> T {
    let tmp = tempfile::tempdir().unwrap();
    let mut toml = String::new();
    for (id, main, extra) in mods {
        jvm_mod(tmp.path(), id, main, extra);
        toml.push_str(&format!("[{id}]\ngrant = [\"native\", \"channels\"{grants}]\n"));
    }
    std::fs::write(tmp.path().join("mods.toml"), toml).unwrap();
    let b = FakeBridge::new();
    b.join("p1", "ALICE", Role::Player);
    b.join("p2", "BOB", Role::Moderator);
    b.positions.borrow_mut().push(("p1".into(), Pos { x: 3, y: -4 }));
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.runtimes.push(runtime(2000));
    tune(&mut opts);
    let mut h = ModHost::load(opts, b.clone());
    h.enable("World", "wid");
    T { _tmp: tmp, b, h }
}

fn state(t: &T, i: usize) -> ModState {
    t.h.mods()[i].state.clone()
}

fn sink(grants: &str) -> T {
    host(&[("sink", "fx.Sink", "")], grants, |_| {})
}

#[test]
fn every_event_reaches_the_mod() {
    let mut t = sink("");
    assert_eq!(state(&t, 0), ModState::Enabled, "{}", t.b.log_text());
    let pos = Pos { x: 1, y: 2 };
    let events = [
        Event::PlayerJoin { player: "p1".into() },
        Event::PlayerQuit { player: "p1".into(), was_kick: true },
        Event::Chat { player: "p1".into(), text: "hello".into(), cancelled: false },
        Event::Command {
            sender: CommandSender::Console,
            name: "x".into(),
            args: vec!["a".into(), "b".into()],
            raw: "/x a b".into(),
            cancelled: false,
        },
        Event::Tick { tick: 5, dt: 0.2 },
        Event::WorldLoaded { world_name: "W".into(), world_id: "w".into() },
        Event::WorldSave { phase: SavePhase::Post },
        Event::PlayerPreLogin {
            player: "p9".into(),
            alias: "NEW".into(),
            cancelled: false,
            reason: RejectReason::Kicked,
        },
        Event::BlockPlace { player: "p1".into(), pos, item: 7, data_a: 8, data_b: 9, cancelled: false },
        Event::BlockBreak { player: "p1".into(), pos, layer: BreakLayer::BackWall, tile: 4, cancelled: false },
        Event::BlockChanged { pos, tile: 5, player: None },
        Event::ObjectPlace { player: "p1".into(), pos, object: ObjectKind::Workbench, item: 3, cancelled: false },
        Event::Damage { player: Some("p1".into()), blockhead: 12, amount: 0.5, attacker: None, cancelled: false },
        Event::Death { player: None, blockhead: 13 },
        Event::PlayerMove { player: "p1".into(), from: pos, to: Pos { x: 2, y: 2 } },
    ];
    for ev in events {
        t.h.fire(ev);
    }
    let log = t.b.log_text();
    for want in [
        "join ALICE PLAYER",
        "quit p1 true",
        "chat p1 hello false",
        "command x [a, b] /x a b console false",
        "tick 5 0.2",
        "loaded W w",
        "save POST",
        "prelogin p9 NEW KICKED",
        "place p1 1,2 7 8 9",
        "break p1 1,2 BACK_WALL 4",
        "changed 1,2 5 -",
        "object p1 WORKBENCH 3",
        "damage p1 12 0.5 -",
        "death - 13",
        "move p1 1,2->2,2",
    ] {
        assert!(log.contains(want), "missing {want:?} in\n{log}");
    }
}

#[test]
fn the_whole_api_from_a_command() {
    let mut t = sink("");
    let r = t.h.command(CommandSender::Player("p1".into()), "/api").unwrap();
    assert_eq!(r, vec!["done".to_string()], "{}", t.b.log_text());
    let log = t.b.log_text();
    for want in [
        "facts 0.2.0 sink 1.0.0 SERVER fake-host 1.0",
        "get p1 -",
        "alias p1 -",
        "online 2",
        "me ALICE true PLAYER p1",
        "pos 3,-4",
        "handle p1 -",
        "broadcast not-permitted not-permitted: players:chat",
        "storage [1, 2] null [a, ab] true false",
        "cancel true false",
        "off true false",
        "fire false",
        "custom sink:evt sink {n=1, p=",
        "close true",
        "unregister true false",
    ] {
        assert!(log.contains(want), "missing {want:?} in\n{log}");
    }
    assert_eq!(t.b.take_chats(), vec![("p1".to_string(), "hi from java".to_string())]);
    // Levels: warn/error/debug reach the host log with their level.
    assert!(log.contains("[warn] sink: warn"), "{log}");
    assert!(log.contains("[error] sink: error"), "{log}");
    assert!(log.contains("[debug] sink: debug"), "{log}");
    // Tasks the command scheduled: `every` was cancelled before it ran, `later` too; the poll task runs.
    t.h.poll();
    t.h.tick(0.2);
    t.h.tick(0.2);
    let log = t.b.log_text();
    assert!(log.contains("poll"), "{log}");
    assert!(!log.contains("every"), "{log}");
    assert!(!log.contains("later"), "{log}");
}

#[test]
fn channels_reach_the_mod() {
    let mut t = sink("");
    assert!(t.h.channel_receive("sink:chan", ChannelSource::Server, b"ping".to_vec()));
    assert!(t.h.channel_receive("sink:chan", ChannelSource::Local("other".into()), b"x".to_vec()));
    assert!(t.h.channel_receive("sink:chan", ChannelSource::Player("p1".into()), b"y".to_vec()));
    t.h.poll();
    let log = t.b.log_text();
    assert!(log.contains("channel server ping"), "{log}");
    assert!(log.contains("channel local x"), "{log}");
    assert!(log.contains("channel player y"), "{log}");
    assert_eq!(t.h.channel_names(), vec!["sink:chan".to_string()]);
}

#[test]
fn handlers_change_events() {
    let mut t = host(&[("mutator", "fx.Mutator", "")], "", |_| {});
    assert_eq!(state(&t, 0), ModState::Enabled, "{}", t.b.log_text());
    let ev = t.h.fire(Event::Chat { player: "p1".into(), text: "quiet".into(), cancelled: false });
    assert_eq!(ev, Event::Chat { player: "p1".into(), text: "QUIET".into(), cancelled: false });
    let ev = t.h.fire(Event::PlayerPreLogin {
        player: "p9".into(),
        alias: "N".into(),
        cancelled: false,
        reason: RejectReason::Kicked,
    });
    assert_eq!(
        ev,
        Event::PlayerPreLogin { player: "p9".into(), alias: "N".into(), cancelled: true, reason: RejectReason::Banned }
    );
    let ev = t.h.fire(Event::Damage { player: None, blockhead: 1, amount: 0.25, attacker: None, cancelled: false });
    assert_eq!(ev, Event::Damage { player: None, blockhead: 1, amount: 0.5, attacker: None, cancelled: false });
    let ev = t.h.fire(Event::BlockPlace {
        player: "p1".into(),
        pos: Pos { x: 0, y: 0 },
        item: 1,
        data_a: 0,
        data_b: 0,
        cancelled: false,
    });
    assert!(ev.cancelled());
    // A tick has no cancelled state: whatever the handler does, it stays uncancelled.
    assert!(!t.h.fire(Event::Tick { tick: 1, dt: 0.2 }).cancelled());
}

#[test]
fn services_between_java_and_java() {
    let mut t = host(
        &[
            (
                "provider",
                "fx.Provider",
                "[provides]\nservices = [{ name = \"calc\", version = \"1.0.0\", wit = \"wit/calc.wit\" }]\n",
            ),
            ("consumer", "fx.Consumer", "[dependencies]\nprovider = \"^1\"\n"),
        ],
        "",
        |_| {},
    );
    for m in t.h.mods() {
        assert_eq!(m.state, ModState::Enabled, "{}", t.b.log_text());
    }
    let p = || CommandSender::Player("p1".into());
    assert_eq!(t.h.command(p(), "/calc add 2 40"), Some(vec!["ok 42".to_string()]), "{}", t.b.log_text());
    assert_eq!(t.h.command(p(), "/calc name"), Some(vec!["ok ALICE".to_string()]));
    let echo = t.h.command(p(), "/calc echo").unwrap();
    assert!(echo[0].starts_with("ok [1, two, player(p1), [3, null]]"), "{echo:?}");
    let fail = t.h.command(p(), "/calc fail").unwrap();
    assert!(fail[0].contains("provider-error: calc refused"), "{fail:?}");
    let crash = t.h.command(p(), "/calc crash").unwrap();
    assert!(crash[0].contains("provider-error") && crash[0].contains("calc crashed"), "{crash:?}");
    let bad = t.h.command(p(), "/calc bad").unwrap();
    assert!(bad[0].contains("provider-error"), "{bad:?}");
    let missing = t.h.command(p(), "/calc nothing-here").unwrap();
    assert!(missing[0].contains("no method nothing-here"), "{missing:?}");
    // An Error (not an Exception) is a fault of the provider.
    let err = t.h.command(p(), "/calc error").unwrap();
    assert!(err[0].contains("trap"), "{err:?}");
}

#[test]
fn failures_are_contained() {
    let mut t = host(&[("faulty", "fx.Faulty", "")], "", |_| {});
    assert_eq!(state(&t, 0), ModState::Enabled, "{}", t.b.log_text());
    let chat =
        |t: &mut T, text: &str| t.h.fire(Event::Chat { player: "p1".into(), text: text.into(), cancelled: false });
    // A command without a handler.
    assert_eq!(
        t.h.command(CommandSender::Console, "/nohandler"),
        Some(vec!["An error occurred while running /nohandler.".to_string()])
    );
    chat(&mut t, "boom");
    assert!(t.b.has_log("handler boom"), "{}", t.b.log_text());
    chat(&mut t, "fine");
    assert_eq!(state(&t, 0), ModState::Enabled);
    // The third fault in a minute disables the mod; the game keeps going.
    chat(&mut t, "error");
    assert!(t.b.has_log("java.lang.AssertionError: fatal"), "{}", t.b.log_text());
    t.h.tick(0.2);
    assert!(matches!(state(&t, 0), ModState::Disabled(_)), "{}", t.b.log_text());
    t.h.disable_all();
}

#[test]
fn a_blocked_call_is_interrupted_at_the_deadline() {
    let mut t = host(&[("faulty", "fx.Faulty", "")], "", |o| {
        o.budgets.hard = Duration::from_millis(50);
        o.runtimes.clear();
        o.runtimes.push(runtime(10));
    });
    let start = std::time::Instant::now();
    t.h.fire(Event::Chat { player: "p1".into(), text: "sleep".into(), cancelled: false });
    assert!(start.elapsed() < Duration::from_secs(5), "the sleep was not interrupted");
    assert!(t.b.has_log("hard time budget exceeded"), "{}", t.b.log_text());
    // A call that ends inside deadline + grace is fine.
    let mut t = host(&[("faulty", "fx.Faulty", "")], "", |o| {
        o.budgets.hard = Duration::from_millis(5);
        o.budgets.soft = Duration::from_millis(1);
        o.runtimes.clear();
        o.runtimes.push(runtime(2000));
    });
    t.h.fire(Event::Chat { player: "p1".into(), text: "slow".into(), cancelled: false });
    assert!(!t.b.has_log("hard time budget exceeded"), "{}", t.b.log_text());
    assert!(t.b.has_log("took"), "{}", t.b.log_text());
}

#[test]
fn enabling_can_fail_in_every_way() {
    let t = host(&[("fails", "fx.EnableFails", "")], "", |_| {});
    assert!(matches!(state(&t, 0), ModState::Disabled(_)), "{:?}", state(&t, 0));
    assert!(t.b.has_log("enable failed"), "{}", t.b.log_text());
    let t = host(&[("error", "fx.EnableError", "")], "", |_| {});
    assert!(!matches!(state(&t, 0), ModState::Enabled));
    assert!(t.b.has_log("StackOverflowError"), "{}", t.b.log_text());
    let t = host(&[("notamod", "fx.NotAMod", "")], "", |_| {});
    assert!(!matches!(state(&t, 0), ModState::Enabled));
    assert!(t.b.has_log("ClassCastException"), "{}", t.b.log_text());
    let t = host(&[("missing", "fx.DoesNotExist", "")], "", |_| {});
    assert!(!matches!(state(&t, 0), ModState::Enabled));
    assert!(t.b.has_log("ClassNotFoundException"), "{}", t.b.log_text());
}

#[test]
fn bad_packages_do_not_load() {
    let rt = runtime(100);
    let limits = InstanceLimits { memory_bytes: 1 << 20 };
    let files = |bytes: Vec<u8>| ModFiles {
        entry: "server/mod.jar".into(),
        files: [("server/mod.jar".to_string(), bytes)].into(),
    };
    let err = |f: ModFiles| rt.instantiate("m", &f, limits).err().expect("must not load");
    assert!(err(ModFiles { entry: "server/mod.jar".into(), files: Default::default() }).contains("is missing"));
    assert!(err(files(b"not a zip".to_vec())).starts_with("m: "));
    assert!(err(files(jar(None))).contains("no Bhmod-Main"));
    // `Main-Class` works as well and an unrelated manifest line is skipped.
    let ok = ModFiles {
        entry: "server/mod.jar".into(),
        files: [("server/mod.jar".to_string(), jar_with_manifest(&[], "Created-By: x\nMain-Class: fx.Sink\n"))].into(),
    };
    assert!(rt.instantiate("m", &ok, limits).is_ok());
    assert_eq!(rt.kind(), bhmod_manifest::manifest::Runtime::Jvm);
}

#[test]
fn a_panic_in_the_host_is_a_java_exception_not_a_crash() {
    let mut t = sink("");
    t.b.explode.set(true);
    // `/api` asks for the online players; the host panics there and the mod sees an exception.
    let r = t.h.command(CommandSender::Player("p1".into()), "/api").unwrap();
    assert_eq!(r, vec!["An error occurred while running /api.".to_string()]);
    let log = t.b.log_text();
    assert!(log.contains("the host panicked: the fake game exploded"), "{log}");
    t.b.explode.set(false);
    // The mod is still there.
    assert_eq!(t.h.command(CommandSender::Player("p1".into()), "/api").unwrap(), vec!["done".to_string()]);
}
