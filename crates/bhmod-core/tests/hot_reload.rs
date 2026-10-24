//! Hot reload (`ModHost::reload`) and the per-mod time accounting (`ModHost::timings`) with real Luau mods.

mod common;

use std::fs;
use std::time::Duration;

use bhmod_core::budget::Budgets;
use bhmod_core::timings::{format_lines, HandlerKind};
use bhmod_core::types::*;
use bhmod_core::{HostOptions, ModHost, ModState, ReloadOutcome};
use common::*;

fn v1(tag: &str) -> String {
    format!(
        r#"
local mod = {{}}
function mod.on_enable()
    local n = tonumber(bh.storage.get("runs") or "0") + 1
    bh.storage.set("runs", tostring(n))
    bh.log.info("{tag} enabled, run " .. n)
    bh.commands.register({{ name = "ver" }}, function() return "{tag} runs=" .. bh.storage.get("runs") end)
    bh.events.on("chat", function(ev) ev.text = "{tag}:" .. ev.text end)
    bh.events.on("world-loaded", function(ev) bh.log.info("{tag} world " .. ev.world_name) end)
    bh.scheduler.run_every(1, 1, function() bh.log.info("{tag} tick task") end)
    bh.services.provide("greeter", "1.0.0", {{ hello = function() return "{tag}" end }})
end
function mod.on_disable() bh.log.info("{tag} disabled") end
return mod
"#
    )
}

const PROVIDES: &str = "[provides]\nservices = [{ name = \"greeter\", version = \"1.0.0\", wit = \"wit/g.wit\" }]\n";
const USER_TOML: &str = "[dependencies]\ngreet = \"^1\"\n";
const USER: &str = r#"
local mod = {}
local proxy
function mod.on_enable()
    proxy = bh.services.get("greeter", "^1")
    bh.commands.register({ name = "old" }, function() return proxy.hello() end)
    bh.commands.register({ name = "fresh" }, function() return bh.services.get("greeter", "^1").hello() end)
    bh.commands.register({ name = "byname" }, function() return bh.services.call("greeter", "^1", "hello") end)
end
return mod
"#;

fn add_wit(mods: &std::path::Path, id: &str) {
    fs::create_dir_all(mods.join(id).join("wit")).unwrap();
    fs::write(mods.join(id).join("wit/g.wit"), "package x:y;\n").unwrap();
}

fn setup(mods: &std::path::Path) -> (std::rc::Rc<FakeBridge>, ModHost) {
    lua_mod(mods, "greet", PROVIDES, &v1("one"));
    add_wit(mods, "greet");
    lua_mod(mods, "user", USER_TOML, USER);
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(mods, Side::Server), b.clone());
    h.enable("World", "w1");
    for m in h.mods() {
        assert_eq!(m.state, ModState::Enabled, "{}", b.log_text());
    }
    (b, h)
}

fn run(h: &mut ModHost, line: &str) -> Vec<String> {
    h.command(CommandSender::Console, line).expect("a mod command")
}

#[test]
fn reload_one_mod_restarts_it_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let (b, mut h) = setup(tmp.path());
    assert_eq!(run(&mut h, "/ver"), vec!["one runs=1"]);
    assert_eq!(run(&mut h, "/old"), vec!["one"]);
    h.tick(0.05);
    assert!(b.has_log("one tick task"));
    // New code: a different task text, a version bump in the manifest.
    fs::write(tmp.path().join("greet/server/main.luau"), v1("two")).unwrap();
    let manifest =
        fs::read_to_string(tmp.path().join("greet/mod.toml")).unwrap().replace("1.0.0\"\nname", "1.1.0\"\nname");
    fs::write(tmp.path().join("greet/mod.toml"), manifest).unwrap();
    b.logs.borrow_mut().clear();
    let r = h.reload(Some("greet"));
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0].outcome, ReloadOutcome::Reloaded("1.0.0".into(), "1.1.0".into()));
    assert_eq!(r[0].to_string(), "greet: reloaded (1.0.0 -> 1.1.0)");
    assert!(b.has_log("one disabled"), "on-disable ran: {}", b.log_text());
    assert!(b.has_log("two enabled, run 2"), "storage kept: {}", b.log_text());
    assert!(b.has_log("two world World"), "world-loaded reaches the reloaded mod: {}", b.log_text());
    assert!(!b.has_log("one world"), "...and only that mod");
    // Commands and handlers are the new ones, once.
    assert_eq!(run(&mut h, "/ver"), vec!["two runs=2"]);
    let chat = h.fire(Event::Chat { player: "p".into(), text: "hi".into(), cancelled: false });
    assert!(matches!(chat, Event::Chat { text, .. } if text == "two:hi"));
    // The old repeating task is gone; the new one runs.
    b.logs.borrow_mut().clear();
    h.tick(0.05);
    h.tick(0.05);
    assert!(!b.has_log("one tick task") && b.has_log("two tick task"), "{}", b.log_text());
    assert_eq!(h.mods().iter().find(|m| m.id == "greet").unwrap().version, "1.1.0");
    // A proxy fetched before the reload is stale; one fetched after, and calls by name, work.
    b.logs.borrow_mut().clear();
    assert_eq!(run(&mut h, "/old"), vec!["An error occurred while running /old."]);
    assert!(b.has_log("stale service handle") && b.has_log("services.get again"), "{}", b.log_text());
    assert_eq!(run(&mut h, "/fresh"), vec!["two"]);
    assert_eq!(run(&mut h, "/byname"), vec!["two"]);
}

#[test]
fn reload_without_id_only_touches_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let (b, mut h) = setup(tmp.path());
    let r = h.reload(None);
    assert!(r.iter().all(|x| x.outcome == ReloadOutcome::Unchanged), "{r:?}");
    assert_eq!(r[0].to_string(), "greet: unchanged");
    assert!(!b.has_log("reload:"));
    // Change one mod, add one, remove one.
    fs::write(tmp.path().join("greet/server/main.luau"), v1("three")).unwrap();
    lua_mod(tmp.path(), "extra", "", "return { on_enable = function() bh.log.info('extra up') end }");
    fs::remove_dir_all(tmp.path().join("user")).unwrap();
    let r = h.reload(None);
    let text: Vec<String> = r.iter().map(ToString::to_string).collect();
    assert!(text.contains(&"greet: reloaded (1.0.0)".to_string()), "{text:?}");
    assert!(text.contains(&"extra: loaded (1.0.0)".to_string()), "{text:?}");
    assert!(text.contains(&"user: stopped (its package is gone or no longer loadable)".to_string()), "{text:?}");
    assert!(b.has_log("extra up") && b.has_log("reload: 3 change(s)"), "{}", b.log_text());
    assert_eq!(h.mods().len(), 3);
    // The gone mod is not reported again; bringing it back starts it.
    let r = h.reload(None);
    assert!(r.iter().all(|x| x.outcome == ReloadOutcome::Unchanged), "{r:?}");
    lua_mod(tmp.path(), "user", USER_TOML, USER);
    let r = h.reload(Some("user"));
    assert_eq!(r[0].outcome, ReloadOutcome::Reloaded("1.0.0".into(), "1.0.0".into()));
    assert_eq!(h.mods().iter().find(|m| m.id == "user").unwrap().state, ModState::Enabled);
    assert_eq!(run(&mut h, "/fresh"), vec!["three"]);
}

#[test]
fn reload_problems_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let (b, mut h) = setup(tmp.path());
    // Unknown id.
    let r = h.reload(Some("nope"));
    assert_eq!(r[0].outcome, ReloadOutcome::Failed("no package for nope in the mods directory".into()));
    assert_eq!(r[0].to_string(), "nope: not reloaded: no package for nope in the mods directory");
    // A broken package in the directory shows up in a full reload and for its id.
    let broken = tmp.path().join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("mod.toml"), "not toml [[[").unwrap();
    let r = h.reload(None);
    assert!(r.iter().any(|x| matches!(&x.outcome, ReloadOutcome::Failed(m) if m.contains("broken"))), "{r:?}");
    let r = h.reload(Some("broken"));
    assert!(matches!(&r[0].outcome, ReloadOutcome::Failed(m) if m.contains("broken")), "{r:?}");
    fs::remove_dir_all(&broken).unwrap();
    // A mod disabled in mods.toml is stopped, with the reason.
    fs::write(tmp.path().join("mods.toml"), "[user]\nenabled = false\n").unwrap();
    let r = h.reload(Some("user"));
    assert_eq!(r[0].outcome, ReloadOutcome::Removed("disabled in mods.toml".into()));
    assert_eq!(
        h.mods().iter().find(|m| m.id == "user").unwrap().state,
        ModState::Disabled("its package is gone or no longer loadable".into())
    );
    // A new version that fails in on-enable leaves the mod disabled and says so.
    fs::write(tmp.path().join("greet/server/main.luau"), "this is not luau").unwrap();
    let r = h.reload(Some("greet"));
    assert!(matches!(&r[0].outcome, ReloadOutcome::Failed(m) if m.contains("disabled")), "{r:?}");
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(_)));
    // Fixing it and reloading again recovers (storage still there).
    fs::write(tmp.path().join("greet/server/main.luau"), v1("fixed")).unwrap();
    let r = h.reload(Some("greet"));
    assert!(matches!(r[0].outcome, ReloadOutcome::Reloaded(..)), "{r:?}");
    assert_eq!(run(&mut h, "/ver"), vec!["fixed runs=2"]);
    drop(b);
}

#[test]
fn reload_before_the_world_loads_only_replaces_the_mod() {
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(tmp.path(), "early", PROVIDES, &v1("a"));
    add_wit(tmp.path(), "early");
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(tmp.path(), Side::Server), b.clone());
    fs::write(tmp.path().join("early/server/main.luau"), v1("b")).unwrap();
    let r = h.reload(Some("early"));
    assert!(matches!(r[0].outcome, ReloadOutcome::Reloaded(..)));
    assert_eq!(h.mods()[0].state, ModState::Loaded);
    lua_mod(tmp.path(), "late", "", "return { on_enable = function() bh.log.info('c enabled') end }");
    let r = h.reload(None);
    assert!(r.iter().any(|x| x.outcome == ReloadOutcome::Added("1.0.0".into())), "{r:?}");
    h.enable("W", "w");
    assert!(b.has_log("b enabled") && b.has_log("c enabled") && !b.has_log("a enabled"), "{}", b.log_text());
    assert_eq!(run(&mut h, "/ver"), vec!["b runs=1"]);
}

#[test]
fn reload_resets_strikes_and_counters() {
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(
        tmp.path(),
        "flaky",
        "",
        r#"local mod = {}
function mod.on_enable()
    bh.events.on("chat", function(ev) if ev.text == "boom" then error("boom") end end)
    bh.events.on_custom("flaky:ping", function(ev) bh.log.info("ping") end)
    bh.commands.register({ name = "fire" }, function() bh.events.fire_custom("flaky:ping", 1); return "fired" end)
end
return mod"#,
    );
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(tmp.path(), Side::Server), b.clone());
    h.enable("W", "w");
    let boom = || Event::Chat { player: "p".into(), text: "boom".into(), cancelled: false };
    h.fire(boom());
    h.fire(boom());
    let t = &h.timings(None)[0];
    assert_eq!((t.strikes, t.strikes_total), (2, 2));
    h.reload(Some("flaky"));
    let t = &h.timings(None)[0];
    assert_eq!((t.strikes, t.strikes_total, t.kinds.len()), (0, 0, 1), "fresh counters (on-enable only): {t:?}");
    // Two more faults do not disable it: the counter started again.
    h.fire(boom());
    h.fire(boom());
    assert_eq!(h.mods()[0].state, ModState::Enabled);
    // A self-fired custom event is queued while the command runs; reloading afterwards is clean.
    assert_eq!(run(&mut h, "/fire"), vec!["fired"]);
    h.reload(Some("flaky"));
    assert_eq!(h.mods()[0].state, ModState::Enabled);
}

#[test]
fn timings_per_kind_over_a_rolling_window() {
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(
        tmp.path(),
        "busy",
        "",
        r#"local mod = {}
function mod.on_enable()
    bh.commands.register({ name = "work" }, function() return "ok" end)
    bh.events.on("chat", function(ev) end)
    bh.scheduler.run_every(1, 1, function() end)
end
return mod"#,
    );
    let clock = FakeClock::ahead();
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.clock = Box::new(clock.clone());
    opts.budgets = Budgets { window: Duration::from_secs(10), ..Budgets::default() };
    let mut h = ModHost::load(opts, FakeBridge::new());
    assert_eq!(h.timings(None)[0].state, "loaded");
    h.enable("W", "w");
    clock.auto.set(Duration::from_millis(8));
    run(&mut h, "/work");
    run(&mut h, "/work");
    h.fire(Event::Chat { player: "p".into(), text: "x".into(), cancelled: false });
    h.tick(0.05);
    clock.auto.set(Duration::ZERO);
    let t = h.timings(Some("busy")).remove(0);
    assert_eq!((t.id.as_str(), t.state.as_str()), ("busy", "enabled"));
    let kinds: Vec<_> = t.kinds.iter().map(|k| (k.kind.unwrap(), k.calls)).collect();
    assert!(kinds.contains(&(HandlerKind::Command, 2)), "{kinds:?}");
    assert!(kinds.contains(&(HandlerKind::Event, 1)), "{kinds:?}");
    assert!(kinds.contains(&(HandlerKind::Task, 1)), "{kinds:?}");
    assert!(kinds.contains(&(HandlerKind::Lifecycle, 1)), "{kinds:?}");
    let cmd = t.kinds.iter().find(|k| k.kind == Some(HandlerKind::Command)).unwrap();
    assert_eq!(cmd.total, Duration::from_millis(16));
    assert_eq!(cmd.avg(), Duration::from_millis(8));
    assert_eq!(cmd.max, Duration::from_millis(8));
    assert_eq!(cmd.soft_hits, 2, "8 ms is over the 5 ms soft budget");
    assert!(t.budget_hits() >= 2 && t.calls() >= 5 && t.total() >= Duration::from_millis(16));
    let lines = format_lines(&h.timings(None));
    assert!(lines[0].contains("last 10 s") && lines[1].starts_with("busy [enabled]: "), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("  command: 2 calls, total 16.00, avg 8.00, max 8.00, over soft 2")));
    // The numbers roll out of the window.
    clock.now.set(clock.now.get() + Duration::from_secs(30));
    assert!(h.timings(None)[0].kinds.is_empty());
    // Reset keeps nothing; an unknown id is reported.
    clock.auto.set(Duration::from_millis(1));
    run(&mut h, "/work");
    clock.auto.set(Duration::ZERO);
    assert_eq!(h.timings(None)[0].calls(), 1);
    assert!(!h.reset_timings(Some("nope")));
    assert!(h.reset_timings(Some("busy")));
    assert_eq!(h.timings(None)[0].calls(), 0);
    run(&mut h, "/work");
    assert!(h.reset_timings(None));
    assert_eq!(h.timings(None)[0].calls(), 0);
    assert!(h.timings(Some("nope")).is_empty());
}

#[test]
fn timings_count_faults_and_timeouts() {
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(
        tmp.path(),
        "bad",
        "",
        r#"local mod = {}
function mod.on_enable()
    bh.commands.register({ name = "err" }, function() error("x") end)
    bh.commands.register({ name = "spin" }, function() while true do end end)
end
return mod"#,
    );
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.budgets = Budgets { hard: Duration::from_millis(100), strikes: 10, ..Budgets::default() };
    let mut h = ModHost::load(opts, FakeBridge::new());
    h.enable("W", "w");
    run(&mut h, "/err");
    run(&mut h, "/spin");
    let t = h.timings(None).remove(0);
    let cmd = t.kinds.iter().find(|k| k.kind == Some(HandlerKind::Command)).unwrap();
    assert_eq!((cmd.calls, cmd.errors, cmd.timeouts), (2, 1, 1), "{cmd:?}");
    assert_eq!((t.strikes, t.strikes_total), (2, 2));
    assert!(cmd.soft_hits >= 1);
    h.disable_all();
    assert!(h.timings(None)[0].state.starts_with("disabled: "));
}
