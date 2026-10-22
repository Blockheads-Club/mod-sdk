//! The WASM runtime against the real mod host and the test guest (`tests/guest`, built by `scripts/build-wasm.sh`).

#[path = "../../bhmod-core/tests/common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bhmod_core::runtime::{InstanceLimits, ModFiles, Runtime};
use bhmod_core::types::{CommandSender, Event, Role, SavePhase, Side};
use bhmod_core::{HostOptions, ModHost, ModState};
use bhmod_wasm::{precompile_for, Backend, WasmConfig, WasmRuntime, HOST_PULLEY};
use common::FakeBridge;

fn guest_wasm() -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/guest/out/guest.wasm");
    fs::read(&p).unwrap_or_else(|e| panic!("{}: {e} (run scripts/build-wasm.sh first)", p.display()))
}

const WASM_TOML: &str = "schema = 1\nid = \"ID\"\nversion = \"1.2.3\"\nname = { en = \"N\" }\nauthors = [\"a\"]\nlicense = \"MIT\"\napi = \"^0.1\"\n[sides]\nserver = true\nclient = false\n[entry]\nserver = { runtime = \"wasm\", path = \"server/mod.wasm\" }\n";

/// Writes an unpacked WASM mod `id` with `wasm` as its entry.
fn wasm_mod(mods: &Path, id: &str, extra: &str, wasm: &[u8]) {
    let d = mods.join(id);
    fs::create_dir_all(d.join("server")).unwrap();
    fs::write(d.join("mod.toml"), format!("{}{extra}", WASM_TOML.replace("ID", id))).unwrap();
    fs::write(d.join("server/mod.wasm"), wasm).unwrap();
}

fn host(dir: &Path, config: WasmConfig) -> (Rc<FakeBridge>, ModHost) {
    let bridge = FakeBridge::new();
    let mut opts = HostOptions::new(dir, Side::Server);
    opts.runtimes = vec![Box::new(WasmRuntime::new(config).unwrap())];
    let h = ModHost::load(opts, bridge.clone());
    (bridge, h)
}

fn one(id: &str, extra: &str, grants: &str) -> (tempfile::TempDir, Rc<FakeBridge>, ModHost) {
    let tmp = tempfile::tempdir().unwrap();
    wasm_mod(tmp.path(), id, extra, &guest_wasm());
    fs::write(tmp.path().join("mods.toml"), grants).unwrap();
    let (b, h) = host(tmp.path(), WasmConfig::default());
    (tmp, b, h)
}

#[test]
fn tour_calls_every_import() {
    let (_t, b, mut h) = one(
        "tour",
        "[permissions]\nplayers = [\"chat\", \"kick\"]\n",
        "[tour]\ngrant = [\"players:chat\", \"players:kick\"]\n",
    );
    b.join("p1", "ALICE", Role::Admin);
    b.join("p2", "BOB", Role::Player);
    h.enable("World", "wid");
    let log = b.log_text();
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{log}");
    for line in [
        "api = \"0.1.0\"",
        "id = \"tour\"",
        "version = \"1.2.3\"",
        "side = Side::Server",
        "host = \"fake-host 1.0\"",
        "[trace] tour: t",
        "[debug] tour: d",
        "[warn] tour: w",
        "[error] tour: e",
        "[info] tour: at",
        "[info] tour: to stdout",
        "[warn] tour: to stderr",
        "get-missing = true",
        "p.id = \"p1\"",
        "p.alias = \"ALICE\"",
        "p.online = true",
        "p.role = Role::Admin",
        "p.chat = Ok(())",
        "by-alias = Some(\"p1\")",
        "online = [\"p1\", \"p2\"]",
        "broadcast = Ok(())",
        "kick = Ok(())",
        "off = true",
        "off-missing = false",
        "custom = Err(ApiError::Unsupported)",
        "fire = Err(ApiError::Unsupported)",
        "unregister = true",
        "bad-command = Err(\"invalid argument",
        "tasks = (",
        "cancel-poll = true",
        "bad-every = Err(\"invalid argument",
        "set = Ok(())",
        "set-string = Ok(())",
        "get = Some([118, 49])",
        "get-string = Some(\"v2\")",
        "keys = [\"k1\", \"k2\"]",
        "delete = true",
        "bad-key = Err(\"invalid argument",
        "provide = Err(ApiError::Unsupported)",
        "call = Err(CallError::NotFound)",
        "open = Err(ApiError::Unsupported)",
        "send = Err(ApiError::Unsupported)",
        "close = false",
        "errors = [\"not permitted: p\", \"invalid argument: a\", \"limit exceeded: l\", \"stale handle\", \"unsupported\", \"internal error: i\"]",
    ] {
        assert!(log.contains(line), "missing {line:?} in\n{log}");
    }
    assert_eq!(b.take_chats(), vec![("p1".to_string(), "hi p1".to_string())]);
    assert_eq!(b.take_broadcasts(), vec!["to all"]);
    assert_eq!(b.kicks.borrow().clone(), vec![("p2".to_string(), "bye".to_string())]);

    // Events: players are lent for the call; an offline player reads as a default.
    let ev = h.fire(Event::Chat { player: "p1".into(), text: "x".into(), cancelled: false });
    assert_eq!(ev, Event::Chat { player: "p1".into(), text: "changed".into(), cancelled: false });
    h.fire(Event::PlayerQuit { player: "gone".into(), was_kick: false });
    assert!(b.has_log("(Some(EventKind::PlayerQuit), Some((\"gone\", \"\", Role::Player)), None, false)"));
    h.fire(Event::PlayerJoin { player: "p2".into() });
    h.fire(Event::WorldSave { phase: SavePhase::Pre });
    h.fire(Event::WorldSave { phase: SavePhase::Post });
    h.fire(Event::Tick { tick: 1, dt: 0.2 });
    let ev = h.fire(Event::Command {
        sender: CommandSender::Player("p1".into()),
        name: "x".into(),
        args: vec!["a".into()],
        raw: "/x a".into(),
        cancelled: false,
    });
    assert!(ev.cancelled());
    assert!(b.has_log("command-event = (\"x\", [\"a\"], \"/x a\")"));
    h.fire(Event::Command {
        sender: CommandSender::Console,
        name: "y".into(),
        args: vec![],
        raw: "/y".into(),
        cancelled: true,
    });
    assert!(b.has_log("(Some(EventKind::WorldLoaded), None, None, false)"), "{}", b.log_text());

    // Commands: reply, usage, console refused by the spec.
    assert_eq!(
        h.command(CommandSender::Player("p1".into()), "/t a b"),
        Some(vec!["ALICE".into(), "t".into(), "a,b".into()])
    );
    assert_eq!(
        h.command(CommandSender::Player("p1".into()), "/tour"),
        Some(vec!["ALICE".into(), "tour".into(), "".into(), "Usage: /tour x".into()])
    );
    assert!(h.command(CommandSender::Console, "/tour a").is_some());

    // Tasks.
    h.poll();
    h.tick(0.2);
    h.tick(0.2);
    assert!(b.has_log("later ran"));
    assert!(b.has_log("every ran"));
    assert!(!b.has_log("poll ran"));

    h.disable_all();
    assert!(b.log_text().contains("[info] tour: disabled\n"));
}

#[test]
fn on_enable_error_disables_without_strike() {
    let (_t, b, mut h) = one("fail", "", "");
    h.enable("World", "wid");
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(r) if r.contains("nope")), "{}", b.log_text());
}

#[test]
fn trap_poisons_the_instance() {
    let (_t, b, mut h) = one("trap", "", "");
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled);
    assert!(h.command(CommandSender::Console, "/boom").is_some());
    assert!(b.has_log("boom"), "{}", b.log_text());
    assert!(b.has_log("command /boom: trap: wasm trap: wasm `unreachable` instruction executed"), "{}", b.log_text());
    h.tick(0.2);
    assert!(b.has_log("the instance trapped earlier"), "{}", b.log_text());
    h.tick(0.2);
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(r) if r.contains("3 faults")), "{}", b.log_text());
}

#[test]
fn deadline_stops_a_spinning_guest() {
    let (_t, b, mut h) = one("spin", "", "");
    h.enable("World", "wid");
    let start = std::time::Instant::now();
    h.tick(0.2);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(b.has_log("hard time budget exceeded"), "{}", b.log_text());
}

#[test]
fn memory_limit_traps() {
    let (_t, b, mut h) = one("oom", "", "");
    h.enable("World", "wid");
    h.tick(0.2);
    assert!(!b.has_log("allocated"), "{}", b.log_text());
    assert!(b.has_log("trap"), "{}", b.log_text());
}

#[test]
fn unknown_behaviour_fails() {
    let (_t, b, mut h) = one("other", "", "");
    h.enable("World", "wid");
    assert!(b.has_log("unknown test mod other"), "{}", b.log_text());
}

fn files(entry: &str, extra: &[(&str, Vec<u8>)]) -> ModFiles {
    let mut f = ModFiles { entry: entry.into(), files: Default::default() };
    f.files.insert(entry.into(), guest_wasm());
    for (k, v) in extra {
        f.files.insert(k.to_string(), v.clone());
    }
    f
}

const LIMITS: InstanceLimits = InstanceLimits { memory_bytes: 1 << 24 };

#[test]
fn bad_components_are_refused() {
    let rt = WasmRuntime::new(WasmConfig::default()).unwrap();
    let mut f = files("server/mod.wasm", &[]);
    f.files.insert("server/mod.wasm".into(), b"junk".to_vec());
    let e = rt.instantiate("m", &f, LIMITS).err().unwrap();
    assert!(e.starts_with("m: "), "{e}");
    let cached =
        WasmRuntime::new(WasmConfig { cache_dir: Some(tempfile::tempdir().unwrap().keep()), ..Default::default() });
    assert!(cached.unwrap().instantiate("m", &f, LIMITS).is_err());
    f.entry = "server/missing.wasm".into();
    assert!(rt.instantiate("m", &f, LIMITS).err().unwrap().contains("entry point server/missing.wasm is missing"));
    // A component without the guest export.
    f.files.insert("server/missing.wasm".into(), b"(component)".to_vec());
    let e = rt.instantiate("m", &f, LIMITS).err().unwrap();
    assert!(e.contains("no exported instance named `blockheads:api/guest@0.1.0`"), "{e}");
}

#[test]
fn compile_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let rt = WasmRuntime::new(WasmConfig { cache_dir: Some(cache.clone()), ..Default::default() }).unwrap();
    let f = files("server/mod.wasm", &[]);
    let path = rt.cache_path(&f.files["server/mod.wasm"]).unwrap();
    assert!(path.starts_with(&cache));
    assert!(path.to_string_lossy().ends_with(&format!("-{}.cwasm", rt.engine_hash())));
    rt.instantiate("tour", &f, LIMITS).unwrap();
    assert!(path.is_file());
    rt.instantiate("tour", &f, LIMITS).unwrap();
    // A broken cache file is replaced.
    fs::write(&path, b"junk").unwrap();
    rt.instantiate("tour", &f, LIMITS).unwrap();
    assert_ne!(fs::read(&path).unwrap(), b"junk");
    // A cache that cannot be written only costs a compile.
    let blocked = tmp.path().join("file");
    fs::write(&blocked, b"").unwrap();
    let rt = WasmRuntime::new(WasmConfig { cache_dir: Some(blocked.join("sub")), ..Default::default() }).unwrap();
    rt.instantiate("tour", &f, LIMITS).unwrap();
    let rt = WasmRuntime::new(WasmConfig { cache_dir: Some(PathBuf::from("/")), ..Default::default() }).unwrap();
    rt.instantiate("tour", &f, LIMITS).unwrap();
}

#[test]
fn backends_and_precompiled_artifacts() {
    let native = WasmRuntime::new(WasmConfig { backend: Backend::Native, ..Default::default() }).unwrap();
    assert_eq!(native.target(), std::env::consts::ARCH);
    let auto = WasmRuntime::new(WasmConfig::default()).unwrap();
    assert_eq!(auto.target(), native.target());
    assert_eq!(auto.engine_hash(), native.engine_hash());

    let cfg = WasmConfig { backend: Backend::Pulley, trust_precompiled: true, ..Default::default() };
    let pulley = WasmRuntime::new(cfg).unwrap();
    assert_eq!(pulley.target(), HOST_PULLEY);
    assert_ne!(pulley.engine_hash(), native.engine_hash());
    let wasm = guest_wasm();
    let art = precompile_for(HOST_PULLEY, &wasm).unwrap();
    assert_eq!(pulley.precompile(&wasm).unwrap().len(), art.len());
    let name = format!("server/mod.{HOST_PULLEY}.cwasm");
    pulley.instantiate("tour", &files("server/mod.wasm", &[]), LIMITS).unwrap();
    pulley.instantiate("tour", &files("server/mod.wasm", &[(&name, art)]), LIMITS).unwrap();
    // A broken artifact falls back to compiling the component.
    pulley.instantiate("tour", &files("server/mod.wasm", &[(&name, b"junk".to_vec())]), LIMITS).unwrap();
    assert!(precompile_for("no-such-target", &wasm).is_err());
    assert!(precompile_for(HOST_PULLEY, b"junk").is_err());
    assert!(native.precompile(b"junk").is_err());

    // The Pulley interpreter runs the same mod.
    let tmp = tempfile::tempdir().unwrap();
    wasm_mod(tmp.path(), "trap", "", &wasm);
    let (b, mut h) = host(tmp.path(), WasmConfig { backend: Backend::Pulley, ..Default::default() });
    h.enable("World", "wid");
    h.tick(0.2);
    assert!(b.has_log("tick"), "{}", b.log_text());
    assert_eq!(pulley.kind(), bhmod_manifest::manifest::Runtime::Wasm);
}

#[test]
fn conversions() {
    use bhmod_core::types as core;
    use bhmod_wasm::bindings::{self as b, events, services, types};
    let errors = [
        core::ApiError::NotPermitted("p".into()),
        core::ApiError::InvalidArgument("a".into()),
        core::ApiError::LimitExceeded("l".into()),
        core::ApiError::StaleHandle,
        core::ApiError::Unsupported,
        core::ApiError::Internal("i".into()),
    ];
    let texts: Vec<String> = errors.into_iter().map(|e| format!("{:?}", b::api_error(e))).collect();
    assert_eq!(texts[0], "ApiError::NotPermitted(\"p\")");
    assert_eq!(texts[5], "ApiError::Internal(\"i\")");
    for r in core::Role::ALL {
        assert_eq!(b::role_from(b::role_to(r)), r);
    }
    assert_eq!(b::side_to(core::Side::Client), types::Side::Client);
    assert!(types::ApiError::StaleHandle.to_string().contains("StaleHandle"));
    assert!(services::CallError::NotFound.to_string().contains("NotFound"));
    assert_eq!(b::side_to(core::Side::Server), types::Side::Server);
    let o =
        b::outcome_from(events::Outcome { cancelled: true, mutation: Some(events::Mutation::CustomPayload(vec![1])) });
    assert_eq!(o, core::Outcome { cancelled: true, mutation: Some(core::Mutation::CustomPayload(vec![1])) });
}
