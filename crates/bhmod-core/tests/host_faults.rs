//! Host behaviour with a scripted fake runtime: loading problems, panics, budgets, limits and storage failures.

mod common;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use bhmod_core::runtime::{CommandId, InstanceLimits, ModFiles, SubscriptionId, TaskId};
use bhmod_core::storage::{KvBackend, StorageProvider};
use bhmod_core::types::*;
use bhmod_core::{CallError, HostApi, HostOptions, Instance, ModHost, ModState, Runtime};
use common::*;

type EnableFn = Box<dyn FnMut(&mut dyn HostApi) -> Result<(), CallError>>;
type EventFn = Box<dyn FnMut(&mut dyn HostApi, SubscriptionId, &Event) -> Result<Outcome, CallError>>;
type TaskFn = Box<dyn FnMut(&mut dyn HostApi, TaskId) -> Result<(), CallError>>;
type CmdFn = Box<dyn FnMut(&mut dyn HostApi, CommandId) -> Result<CommandOutcome, CallError>>;

#[derive(Default)]
struct Fake {
    enable: Option<EnableFn>,
    event: Option<EventFn>,
    task: Option<TaskFn>,
    command: Option<CmdFn>,
}

impl Instance for Fake {
    fn on_enable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        self.enable.as_mut().map_or(Ok(()), |f| f(host))
    }
    fn on_disable(&mut self, _: &mut dyn HostApi) -> Result<(), CallError> {
        Ok(())
    }
    fn handle_event(&mut self, host: &mut dyn HostApi, sub: SubscriptionId, ev: &Event) -> Result<Outcome, CallError> {
        self.event.as_mut().map_or(Ok(Outcome::default()), |f| f(host, sub, ev))
    }
    fn run_command(
        &mut self,
        host: &mut dyn HostApi,
        id: CommandId,
        _: &CommandSender,
        _: &str,
        _: &[String],
    ) -> Result<CommandOutcome, CallError> {
        self.command.as_mut().map_or(Ok(CommandOutcome::default()), |f| f(host, id))
    }
    fn run_task(&mut self, host: &mut dyn HostApi, id: TaskId) -> Result<(), CallError> {
        self.task.as_mut().map_or(Ok(()), |f| f(host, id))
    }
}

#[derive(Default, Clone)]
struct FakeRuntime {
    fakes: Rc<RefCell<HashMap<String, Fake>>>,
}

impl FakeRuntime {
    fn add(&self, id: &str, f: Fake) {
        self.fakes.borrow_mut().insert(id.into(), f);
    }
}

impl Runtime for FakeRuntime {
    fn kind(&self) -> bhmod_manifest::manifest::Runtime {
        bhmod_manifest::manifest::Runtime::Wasm
    }
    fn instantiate(&self, mod_id: &str, files: &ModFiles, _: InstanceLimits) -> Result<Box<dyn Instance>, String> {
        assert!(files.files.contains_key(&files.entry));
        let f = self.fakes.borrow_mut().remove(mod_id).ok_or_else(|| "no fake".to_string())?;
        Ok(Box::new(f))
    }
}

const WASM_MAGIC: &[u8] = b"\0asm\x0d\x00\x01\x00";

fn wasm_mod(mods: &Path, id: &str, side: &str, extra: &str) {
    let d = mods.join(id);
    fs::create_dir_all(d.join(side)).unwrap();
    let (srv, cli) = if side == "server" { ("true", "false") } else { ("false", "true") };
    fs::write(
        d.join("mod.toml"),
        format!(
            "schema = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = {{ en = \"N\" }}\nauthors = [\"a\"]\nlicense = \"MIT\"\napi = \"^0.1\"\n[sides]\nserver = {srv}\nclient = {cli}\n[entry]\n{side} = {{ runtime = \"wasm\", path = \"{side}/mod.wasm\" }}\n{extra}"
        ),
    )
    .unwrap();
    fs::write(d.join(format!("{side}/mod.wasm")), WASM_MAGIC).unwrap();
}

struct Setup {
    _tmp: tempfile::TempDir,
    bridge: Rc<FakeBridge>,
    rt: FakeRuntime,
    clock: FakeClock,
    mods: std::path::PathBuf,
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    fs::create_dir_all(&mods).unwrap();
    Setup { bridge: FakeBridge::new(), rt: FakeRuntime::default(), clock: FakeClock::new(), mods, _tmp: tmp }
}

impl Setup {
    fn host(&self, side: Side, storage: Option<Box<dyn StorageProvider>>) -> ModHost {
        let mut o = HostOptions::new(&self.mods, side);
        o.runtimes = vec![Box::new(self.rt.clone())];
        o.clock = Box::new(self.clock.clone());
        if let Some(s) = storage {
            o.storage = s;
        }
        ModHost::load(o, self.bridge.clone())
    }
}

#[test]
fn loading_problems_are_logged() {
    let s = setup();
    fs::write(s.mods.join("mods.toml"), "[x]\nwhat = 1\n").unwrap();
    fs::write(s.mods.join("bad.bhmod"), "zip?").unwrap();
    wasm_mod(&s.mods, "nofake", "server", "");
    wasm_mod(&s.mods, "needs", "server", "[permissions]\nplayers = [\"kick\"]\n");
    wasm_mod(&s.mods, "clientonly", "client", "");
    let h = s.host(Side::Server, None);
    assert!(h.is_empty());
    let b = &s.bridge;
    assert!(b.has_log("mods.toml is invalid, no permissions are granted"), "{}", b.log_text());
    assert!(b.has_log("cannot load"));
    assert!(b.has_log("cannot start nofake: no fake"));
    assert!(b.has_log("not loading"));
    assert!(b
        .logs
        .borrow()
        .iter()
        .any(|(_, l, m)| *l == LogLevel::Debug && m == "skipping clientonly: no server entry"));

    // The same mods directory on a client loads the client mod.
    s.rt.add("clientonly", Fake::default());
    let h = s.host(Side::Client, None);
    assert_eq!(h.mods().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["clientonly"]);
    assert_eq!(h.mods()[0].sha256, None);
    assert!(h.mods()[0].origin.ends_with("clientonly"));
}

#[test]
fn not_enabled_is_inert() {
    let s = setup();
    wasm_mod(&s.mods, "aa", "server", "");
    s.rt.add(
        "aa",
        Fake {
            enable: Some(Box::new(|h| {
                h.register_command(CommandSpec::new("x")).unwrap();
                h.subscribe(EventKind::Tick, Priority::Normal, false).unwrap();
                Ok(())
            })),
            event: Some(Box::new(|_, _, _| panic!("no events before enable"))),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, None);
    assert_eq!(h.mods()[0].state, ModState::Loaded);
    assert!(!h.owns_command("/x"));
    assert_eq!(h.command(CommandSender::Console, "/x"), None);
    h.tick(0.2);
    h.poll();
    h.world_save(SavePhase::Post);
    h.enable("W", "w");
    h.enable("W", "w");
    assert!(h.owns_command("/x"));
    h.disable_all();
    h.enable("W", "w");
    assert!(matches!(h.mods()[0].state, ModState::Disabled(_)));
}

#[test]
fn panics_are_strikes() {
    let s = setup();
    wasm_mod(&s.mods, "pp", "server", "");
    s.rt.add(
        "pp",
        Fake {
            enable: Some(Box::new(|h| {
                h.subscribe(EventKind::Chat, Priority::Normal, false).unwrap();
                Ok(())
            })),
            event: Some(Box::new(|_, _, ev| match ev {
                Event::Chat { text, .. } if text == "str" => panic!("static text"),
                Event::Chat { text, .. } if text == "string" => panic!("{}", String::from("owned text")),
                _ => std::panic::panic_any(5u8),
            })),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, None);
    h.enable("W", "w");
    for t in ["str", "string", "other"] {
        let ev = h.fire(Event::Chat { player: "p".into(), text: t.into(), cancelled: false });
        assert!(!ev.cancelled(), "a trapped handler changes nothing");
    }
    let b = &s.bridge;
    assert!(b.has_log("panic: static text") && b.has_log("panic: owned text") && b.has_log("panic: unknown panic"));
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(_)));
}

#[test]
fn outcomes_monitor_and_mutations() {
    let s = setup();
    wasm_mod(&s.mods, "mm", "server", "");
    s.rt.add(
        "mm",
        Fake {
            enable: Some(Box::new(|h| {
                h.subscribe(EventKind::Tick, Priority::Normal, false).unwrap();
                h.subscribe(EventKind::Chat, Priority::Monitor, false).unwrap();
                h.subscribe(EventKind::Chat, Priority::Low, false).unwrap();
                Ok(())
            })),
            event: Some(Box::new(|_, _, ev| {
                Ok(match ev {
                    Event::Tick { .. } => Outcome { cancelled: false, mutation: Some(Mutation::ChatText("x".into())) },
                    _ => Outcome { cancelled: true, mutation: Some(Mutation::ChatText("changed".into())) },
                })
            })),
            command: Some(Box::new(|_, _| Err(CallError::Failed("no".into())))),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, None);
    h.enable("W", "w");
    h.tick(0.2);
    assert!(s.bridge.has_log("ignored a change that does not fit tick"));
    // Only the low handler changes the event; the monitor's identical outcome is ignored either way.
    let ev = h.fire(Event::Chat { player: "p".into(), text: "t".into(), cancelled: false });
    assert_eq!(ev, Event::Chat { player: "p".into(), text: "changed".into(), cancelled: true });
}

#[test]
fn step_budget_postpones_tasks_and_soft_budget_warns() {
    let s = setup();
    wasm_mod(&s.mods, "tt", "server", "");
    let clock = s.clock.clone();
    let ran = Rc::new(RefCell::new(Vec::new()));
    let r2 = ran.clone();
    s.rt.add(
        "tt",
        Fake {
            enable: Some(Box::new(|h| {
                for _ in 0..3 {
                    h.run_later(1).unwrap();
                }
                h.run_next_poll().unwrap();
                Ok(())
            })),
            task: Some(Box::new(move |_, id| {
                clock.advance(Duration::from_millis(15));
                r2.borrow_mut().push(id);
                Ok(())
            })),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, None);
    h.enable("W", "w");
    h.tick(0.2);
    assert_eq!(*ran.borrow(), vec![1, 2]);
    assert!(s.bridge.has_log("scheduled task took 15.0 ms (budget 5 ms)"), "{}", s.bridge.log_text());
    h.tick(0.2);
    assert_eq!(*ran.borrow(), vec![1, 2, 3]);
    h.poll();
    assert_eq!(*ran.borrow(), vec![1, 2, 3, 4]);
}

#[test]
fn host_api_limits_and_permissions() {
    let s = setup();
    wasm_mod(&s.mods, "ll", "server", "[permissions]\nplayers = [\"kick\"]\n");
    fs::write(s.mods.join("mods.toml"), "[ll]\ngrant = [\"players:kick\"]\n").unwrap();
    s.bridge.join("p", "P", Role::Player);
    s.rt.add(
        "ll",
        Fake {
            enable: Some(Box::new(|h| {
                for _ in 0..1024 {
                    h.subscribe(EventKind::Tick, Priority::Normal, false).unwrap();
                }
                assert!(matches!(
                    h.subscribe(EventKind::Tick, Priority::Normal, false),
                    Err(ApiError::LimitExceeded(_))
                ));
                for _ in 0..10_000 {
                    h.run_next_poll().unwrap();
                }
                assert!(matches!(h.run_later(1), Err(ApiError::LimitExceeded(_))));
                assert!(matches!(h.run_every(1, 1), Err(ApiError::LimitExceeded(_))));
                assert!(matches!(h.run_next_poll(), Err(ApiError::LimitExceeded(_))));
                assert_eq!(h.player_kick("p", "bye"), Ok(()));
                assert_eq!(h.player_kick("gone", "bye"), Err(ApiError::StaleHandle));
                assert_eq!(h.player_send_chat("gone", "x"), Err(ApiError::StaleHandle));
                assert_eq!(h.player_role("gone"), Err(ApiError::StaleHandle));
                assert_eq!(h.player_role("p"), Ok(Role::Player));
                assert_eq!(h.players_broadcast("x"), Err(ApiError::NotPermitted("players:chat".into())));
                h.log(LogLevel::Info, &"é".repeat(3000));
                assert_eq!(h.current_side(), Side::Server);
                Ok(())
            })),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, None);
    h.enable("W", "w");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", s.bridge.log_text());
    assert_eq!(*s.bridge.kicks.borrow(), vec![("p".to_string(), "bye".to_string())]);
    let long = s.bridge.logs.borrow().iter().find(|(_, _, m)| m.starts_with('é')).unwrap().2.clone();
    assert_eq!(long.len(), 4096);
}

struct BrokenStorage {
    open: bool,
}

struct HalfBroken;

impl KvBackend for HalfBroken {
    fn load(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
        Ok(vec![("k".into(), b"v".to_vec())])
    }
    fn put(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn delete(&mut self, _: &str) -> io::Result<()> {
        Err(io::Error::other("delete broke"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("flush broke"))
    }
}

impl StorageProvider for BrokenStorage {
    fn open(&self, _: &str) -> io::Result<Box<dyn KvBackend>> {
        if self.open {
            Err(io::Error::other("no disk"))
        } else {
            Ok(Box::new(HalfBroken))
        }
    }
}

#[test]
fn storage_failures() {
    let s = setup();
    wasm_mod(&s.mods, "ss", "server", "");
    s.rt.add(
        "ss",
        Fake {
            enable: Some(Box::new(|h| {
                assert_eq!(h.storage_get("k"), None);
                assert!(matches!(h.storage_set("k", b"v"), Err(ApiError::Internal(_))));
                assert!(!h.storage_delete("k"));
                assert!(h.storage_keys("").is_empty());
                Ok(())
            })),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, Some(Box::new(BrokenStorage { open: true })));
    h.enable("W", "w");
    assert!(s.bridge.has_log("storage unavailable: no disk"));
    assert_eq!(h.mods()[0].state, ModState::Enabled);
    h.world_save(SavePhase::Post);
    h.disable_all();

    wasm_mod(&s.mods, "ss", "server", "");
    s.rt.add(
        "ss",
        Fake {
            enable: Some(Box::new(|h| {
                assert_eq!(h.storage_get("k"), Some(b"v".to_vec()));
                assert!(!h.storage_delete("k"));
                Ok(())
            })),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, Some(Box::new(BrokenStorage { open: false })));
    h.enable("W", "w");
    assert!(s.bridge.has_log("storage delete failed: delete broke"));
    h.world_save(SavePhase::Post);
    h.disable_all();
    let flushes = s.bridge.logs.borrow().iter().filter(|(_, _, m)| m == "storage flush failed: flush broke").count();
    assert_eq!(flushes, 2);
}

#[test]
fn split_command_and_truncate() {
    use bhmod_core::host::{split_command, truncate};
    assert_eq!(split_command("/Hi  a b\r\n"), Some(("hi".to_string(), vec!["a".to_string(), "b".to_string()])));
    assert_eq!(split_command("/"), None);
    assert_eq!(split_command("x"), None);
    assert_eq!(truncate("abc", 5), "abc");
    assert_eq!(truncate("aé", 2), "a");
}

#[test]
fn tasks_of_a_mod_disabled_mid_step_are_skipped() {
    let s = setup();
    wasm_mod(&s.mods, "tr", "server", "");
    let calls = Rc::new(RefCell::new(0));
    let c2 = calls.clone();
    s.rt.add(
        "tr",
        Fake {
            enable: Some(Box::new(|h| {
                for _ in 0..4 {
                    h.run_later(1).unwrap();
                }
                Ok(())
            })),
            task: Some(Box::new(move |_, _| {
                *c2.borrow_mut() += 1;
                Err(CallError::Trap("task broke".into()))
            })),
            ..Fake::default()
        },
    );
    let mut h = s.host(Side::Server, None);
    h.enable("W", "w");
    h.tick(0.2);
    assert_eq!(*calls.borrow(), 3);
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(_)));
}
