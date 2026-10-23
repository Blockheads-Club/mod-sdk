//! API 0.3.1 cross-side service calls through the Luau mapping, on both hosts: a server mod provides services that
//! a client may call (opt-in, per method, rate limited, under the provider's budget), and a client mod calls them
//! asynchronously (callback, timeout, cancel, disconnect).

mod common;

use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use bhmod_core::budget::Budgets;
use bhmod_core::remote::{RemoteError, RemoteLimits, RemoteTarget, MAX_PAYLOAD};
use bhmod_core::types::{CommandSender, Event, Role, Side};
use bhmod_core::value::Value;
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;

const ECO_TOML: &str = "[dependencies]\nledger = \"^1\"\n[permissions]\nchannels = true\n[provides]\nservices = [\n\
{ name = \"economy\", version = \"1.2.0\", wit = \"wit/e.wit\", remote = true, remote-methods = [\"balance\", \"whoami\", \"fail\", \"huge\"] },\n\
{ name = \"bank\", version = \"1.0.0\", wit = \"wit/e.wit\", remote = true },\n\
{ name = \"other\", version = \"1.0.0\", wit = \"wit/e.wit\" },\n]\n";

const LEDGER_TOML: &str = "[provides]\nservices = [{ name = \"vault\", version = \"1.0.0\", wit = \"wit/e.wit\" }]\n";

const LEDGER: &str = r#"
local mod = {}
function mod.on_enable()
    bh.services.provide("vault", "1.0.0", {
        peek = function()
            local c = bh.services.caller()
            return c and c.mod or "local"
        end,
    })
end
return mod
"#;

const ECO: &str = r#"
local mod = {}
function mod.on_enable()
    bh.services.provide("economy", "1.2.0", {
        balance = function(who) return 100 + #who end,
        give = function() return "never reachable from a client" end,
        whoami = function()
            local c = bh.services.caller()
            return { mod = c.mod, player = c.player and c.player:id() or false }
        end,
        fail = function() error("no such account", 0) end,
        huge = function() return string.rep("x", 40000) end,
    })
    bh.services.provide("bank", "1.0.0", {
        echo = function(...) return { ... } end,
        spin = function() while true do end end,
        crash = function() return function() end end,
        longfail = function() error(string.rep("e", 300), 0) end,
        local_caller = function() return bh.services.caller() == nil end,
        chain = function() return bh.services.call("vault", "^1", "peek") end,
    })
    bh.services.provide("other", "1.0.0", {})
    -- a local call has no remote caller
    assert(bh.services.caller() == nil)
end
return mod
"#;

/// A client mod that calls the server and prints every answer.
const CLIENT: &str = r#"
local mod = {}
local function show(tag)
    return function(ok, v)
        if ok then
            print(tag .. " ok " .. tostring(type(v) == "table" and (v[1] or v.mod or "table") or v))
        else
            print(tag .. " err " .. v.kind .. " [" .. v.message .. "]")
        end
    end
end
function mod.on_enable()
    bh.commands.register({ name = "ask" }, function() end)
    local id = bh.services.call_remote("server", "economy", "^1", "balance", { "abc" }, show("balance"))
    assert(id == 1, id)
    bh.services.call_remote("server", "economy", "^1", "fail", nil, show("fail"), 1000)
    bh.services.call_remote("server", "bank", "*", "echo", { 1, "two" }, show("echo"))
    bh.services.call_remote("server", "bank", "*", "echo", {}, show("echo0"))
    local slow = bh.services.call_remote("server", "economy", "^1", "balance", { "x" }, show("slow"), 2000)
    local gone = bh.services.call_remote("server", "economy", "^1", "balance", { "x" }, show("gone"))
    assert(bh.services.cancel_remote(gone) == true)
    assert(bh.services.cancel_remote(gone) == false)
    -- invalid calls fail at once, nothing is sent
    local function bad(...) local ok, e = pcall(bh.services.call_remote, ...); assert(not ok); return tostring(e) end
    local f = function() end
    print(bad("moon", "a", "^1", "m", {}, f))
    print(bad("server", "", "^1", "m", {}, f))
    print(bad("server", "a", "nonsense", "m", {}, f))
    print(bad("server", "a", "^1", "m", "text", f))
    print(bad("server", "a", "^1", "m", { 1 }, f, 30001))
    print(bad("server", "a", "^1", "m", { string.rep("x", 40000) }, f))
    print(bad("server", "a", "^1", string.rep("m", 65), {}, f))
    print(bad("server", "a", "^1", "m", { f }, f))
end
return mod
"#;

fn add_wit(mods: &Path, id: &str) {
    fs::create_dir_all(mods.join(id).join("wit")).unwrap();
    fs::write(mods.join(id).join("wit/e.wit"), "package x:y;\n").unwrap();
}

fn client_mod(mods: &Path, id: &str, extra: &str, main: &str) {
    let d = mods.join(id);
    fs::create_dir_all(d.join("client")).unwrap();
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = {{ en = \"N\" }}\nauthors = [\"a\"]\nlicense = \"MIT\"\n\
         api = \"^0.3\"\n[sides]\nserver = false\nclient = true\n[entry]\nclient = {{ runtime = \"lua\", path = \"client/main.luau\" }}\n{extra}"
    );
    fs::write(d.join("mod.toml"), toml).unwrap();
    fs::write(d.join("client/main.luau"), main).unwrap();
}

struct World {
    _tmp: tempfile::TempDir,
    b: Rc<FakeBridge>,
    h: ModHost,
    clock: FakeClock,
}

/// A server with the `eco` and `ledger` mods. Budgets are wall-clock, so the manual clock is only installed on
/// request (`manual_clock`).
fn server(limits: RemoteLimits, manual_clock: bool) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    lua_mod(&mods, "eco", ECO_TOML, ECO);
    add_wit(&mods, "eco");
    lua_mod(&mods, "ledger", LEDGER_TOML, LEDGER);
    add_wit(&mods, "ledger");
    fs::write(mods.join("mods.toml"), "[eco]\ngrant = [\"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    b.join("p1", "ALICE", Role::Player);
    let clock = FakeClock::ahead();
    let mut opts = HostOptions::new(&mods, Side::Server);
    opts.budgets = Budgets { hard: Duration::from_millis(250), strikes: 100, ..Budgets::default() };
    if manual_clock {
        opts.clock = Box::new(clock.clone());
    }
    opts.remote = limits;
    let mut h = ModHost::load(opts, b.clone());
    h.enable("W", "w");
    assert!(h.mods().iter().all(|m| m.state == ModState::Enabled), "{}", b.log_text());
    World { _tmp: tmp, b, h, clock }
}

fn client(main: &str, extra: &str, grant: bool, limits: RemoteLimits) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    client_mod(&mods, "shopui", extra, main);
    add_wit(&mods, "shopui");
    if grant {
        fs::write(mods.join("mods.toml"), "[shopui]\ngrant = [\"channels\"]\n").unwrap();
    }
    let b = FakeBridge::new();
    let clock = FakeClock::ahead();
    let mut opts = HostOptions::new(&mods, Side::Client);
    opts.clock = Box::new(clock.clone());
    opts.remote = limits;
    let mut h = ModHost::load(opts, b.clone());
    h.enable("W", "w");
    World { _tmp: tmp, b, h, clock }
}

fn args(items: Vec<Value>) -> Vec<u8> {
    Value::List(items).encode()
}

fn call(
    w: &mut World,
    from: &RemoteTarget,
    svc: &str,
    req: &str,
    method: &str,
    a: &[u8],
) -> Result<Value, RemoteError> {
    w.h.remote_call_in(from, "shopui", svc, req, method, a).map(|b| Value::decode(&b).unwrap())
}

fn p1() -> RemoteTarget {
    RemoteTarget::Player("p1".into())
}

#[test]
fn server_answers_exposed_services_only() {
    let mut w = server(RemoteLimits { burst: 1000, ..RemoteLimits::default() }, false);
    let t = |s: &str| Value::Text(s.into());
    assert_eq!(call(&mut w, &p1(), "economy", "^1", "balance", &args(vec![t("abc")])), Ok(Value::Int(103)));
    // the provider sees who called
    let Ok(Value::Map(who)) = call(&mut w, &p1(), "economy", "^1.1", "whoami", &args(vec![])) else { panic!() };
    assert!(who.contains(&("mod".into(), t("shopui"))) && who.contains(&("player".into(), t("p1"))));
}

#[test]
fn rules_of_the_answering_side() {
    let mut w = server(RemoteLimits { burst: 1000, ..RemoteLimits::default() }, false);
    let t = |s: &str| Value::Text(s.into());
    let none = args(vec![]);
    // not exposed at all, exposed but not this method
    assert_eq!(call(&mut w, &p1(), "vault", "*", "peek", &none), Err(RemoteError::NotPermitted));
    assert_eq!(call(&mut w, &p1(), "economy", "^1", "give", &none), Err(RemoteError::NotPermitted));
    assert_eq!(call(&mut w, &p1(), "nothing", "*", "x", &none), Err(RemoteError::NotFound));
    assert_eq!(call(&mut w, &p1(), "economy", "^2", "balance", &none), Err(RemoteError::VersionMismatch));
    assert!(matches!(call(&mut w, &p1(), "economy", "nonsense", "balance", &none), Err(RemoteError::BadValue(_))));
    // arguments
    assert!(matches!(
        call(&mut w, &p1(), "bank", "*", "echo", &Value::Int(1).encode()),
        Err(RemoteError::BadValue(m)) if m.contains("must be a list")
    ));
    assert!(matches!(
        call(&mut w, &p1(), "bank", "*", "echo", &[0xff]),
        Err(RemoteError::BadValue(m)) if m.starts_with("arguments")
    ));
    assert_eq!(call(&mut w, &p1(), "bank", "*", "echo", &vec![0x80; MAX_PAYLOAD + 1]), Err(RemoteError::TooLarge));
    assert_eq!(
        w.h.remote_call_in(&p1(), "Not A Mod", "bank", "*", "echo", &none),
        Err(RemoteError::BadValue("caller mod or method name".into()))
    );
    assert!(matches!(
        w.h.remote_call_in(&p1(), "shopui", "bank", "*", &"m".repeat(65), &none),
        Err(RemoteError::BadValue(_))
    ));
    // the provider's own results
    assert_eq!(
        call(&mut w, &p1(), "economy", "^1", "fail", &none),
        Err(RemoteError::ProviderError("no such account".into()))
    );
    assert_eq!(call(&mut w, &p1(), "economy", "^1", "huge", &none), Err(RemoteError::TooLarge));
    assert_eq!(
        call(&mut w, &p1(), "bank", "*", "crash", &none),
        Err(RemoteError::ProviderError("bad value: cannot convert a function".into()))
    );
    // error texts leave the machine without a traceback and cut at 256 bytes
    assert_eq!(call(&mut w, &p1(), "bank", "*", "longfail", &none), Err(RemoteError::ProviderError("e".repeat(256))));
    assert_eq!(call(&mut w, &p1(), "bank", "*", "echo", &args(vec![t("a"), Value::Int(2)])).map(|v| v.encode()), {
        Ok(Value::List(vec![t("a"), Value::Int(2)]).encode())
    });
    // a time budget overrun is a trap (and a strike) of the provider
    assert_eq!(call(&mut w, &p1(), "bank", "*", "spin", &none), Err(RemoteError::Trap));
    assert!(w.b.has_log("remote service bank.spin: hard time budget exceeded"), "{}", w.b.log_text());
    // the remote caller reaches services the provider calls itself
    assert_eq!(call(&mut w, &p1(), "bank", "*", "chain", &none), Ok(t("shopui")));
    assert_eq!(call(&mut w, &p1(), "other", "*", "x", &none), Err(RemoteError::NotPermitted));
    assert_eq!(call(&mut w, &p1(), "bank", "*", "local_caller", &none), Ok(Value::Bool(false)));
}

#[test]
fn rate_limit_per_peer() {
    let mut w = server(RemoteLimits { burst: 3, per_second: 2, max_pending: 4 }, true);
    let none = args(vec![]);
    for _ in 0..3 {
        assert!(call(&mut w, &p1(), "bank", "*", "echo", &none).is_ok());
    }
    assert_eq!(call(&mut w, &p1(), "bank", "*", "echo", &none), Err(RemoteError::Busy));
    // another player has a bucket of their own
    assert!(call(&mut w, &RemoteTarget::Player("p2".into()), "bank", "*", "echo", &none).is_ok());
    w.clock.advance(Duration::from_millis(500));
    assert!(call(&mut w, &p1(), "bank", "*", "echo", &none).is_ok());
    assert_eq!(call(&mut w, &p1(), "bank", "*", "echo", &none), Err(RemoteError::Busy));
}

#[test]
fn nothing_is_answered_before_the_world_loads_or_after_the_mods_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    lua_mod(&mods, "eco", ECO_TOML, ECO);
    add_wit(&mods, "eco");
    lua_mod(&mods, "ledger", LEDGER_TOML, LEDGER);
    add_wit(&mods, "ledger");
    fs::write(mods.join("mods.toml"), "[eco]\ngrant = [\"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(&mods, Side::Server), b);
    let none = args(vec![]);
    assert_eq!(h.remote_call_in(&p1(), "shopui", "bank", "*", "echo", &none), Err(RemoteError::Busy));
    h.enable("W", "w");
    assert!(h.remote_call_in(&p1(), "shopui", "bank", "*", "echo", &none).is_ok());
    h.disable_all();
    assert_eq!(h.remote_call_in(&p1(), "shopui", "bank", "*", "echo", &none), Err(RemoteError::Busy));
}

#[test]
fn a_client_mod_calls_the_server_asynchronously() {
    let mut w = client(CLIENT, "[permissions]\nchannels = true\n", true, RemoteLimits::default());
    assert_eq!(w.h.mods()[0].state, ModState::Enabled, "{}", w.b.log_text());
    // the eight bad calls were refused at once and nothing was sent for them
    let sent = w.b.remote_calls.borrow().clone();
    assert_eq!(sent.len(), 6, "{}", w.b.log_text());
    assert_eq!(sent[0].id, 1);
    assert_eq!(
        (sent[0].mod_id.as_str(), sent[0].service.as_str(), sent[0].method.as_str()),
        ("shopui", "economy", "balance")
    );
    assert_eq!(sent[0].version_req, "^1");
    assert_eq!(sent[0].target, RemoteTarget::Server);
    assert_eq!(Value::decode(&sent[0].args).unwrap(), Value::List(vec![Value::Text("abc".into())]));
    let log = w.b.log_text();
    for want in [
        "invalid argument: remote target is \"server\" or a player",
        "service name must be 1 to 64 bytes",
        "\"nonsense\" is not a semver requirement",
        "invalid argument: remote call arguments must be a list",
        "timeout is at most 30000 ms",
        "limit exceeded: remote call arguments",
        "method name must be 1 to 64 bytes",
        "bad value: cannot convert a function",
    ] {
        assert!(log.contains(want), "{want}\n{log}");
    }
    assert!(!log.contains("balance ok"), "no answer yet");

    let ok = |v: Value| Ok(v.encode());
    // out of order, each reaches its own callback
    assert!(w.h.remote_result(
        &RemoteTarget::Server,
        3,
        ok(Value::List(vec![Value::Int(1), Value::Text("two".into())]))
    ));
    assert!(w.h.remote_result(&RemoteTarget::Server, 1, ok(Value::Int(103))));
    assert!(w.h.remote_result(&RemoteTarget::Server, 2, Err(RemoteError::ProviderError("no such account".into()))));
    assert!(w.h.remote_result(&RemoteTarget::Server, 4, ok(Value::List(vec![]))));
    let log = w.b.log_text();
    for want in ["balance ok 103", "fail err provider-error [no such account]", "echo ok 1", "echo0 ok table"] {
        assert!(log.contains(want), "{want}\n{log}");
    }
    // answered calls, cancelled calls and strangers' answers are ignored
    assert!(!w.h.remote_result(&RemoteTarget::Server, 1, ok(Value::Null)));
    assert!(!w.h.remote_result(&RemoteTarget::Server, 6, ok(Value::Null)), "cancelled");
    assert!(!w.h.remote_result(&RemoteTarget::Player("p1".into()), 5, ok(Value::Null)), "not the peer asked");
    assert!(!w.h.remote_result(&RemoteTarget::Server, 99, ok(Value::Null)));
    // the slow call (2 s) times out, once; a late answer is dropped
    w.clock.advance(Duration::from_millis(1999));
    w.h.poll();
    assert!(!w.b.has_log("slow err"), "{}", w.b.log_text());
    w.clock.advance(Duration::from_millis(2));
    w.h.tick(0.2);
    assert_eq!(w.b.log_text().matches("slow err timeout []").count(), 1);
    assert!(!w.h.remote_result(&RemoteTarget::Server, 5, ok(Value::Null)));
    w.clock.advance(Duration::from_secs(100));
    w.h.poll();
    assert_eq!(w.b.log_text().matches("slow err").count(), 1);
}

#[test]
fn bad_answers_are_failures_not_values() {
    const MAIN: &str = r#"
local mod = {}
function mod.on_enable()
    for i = 1, 3 do
        bh.services.call_remote("server", "s", "*", "m", {}, function(ok, v) print("got " .. i .. " " .. tostring(ok) .. " " .. (ok and "value" or v.kind .. " " .. v.message)) end)
    end
end
return mod
"#;
    let mut w = client(MAIN, "[permissions]\nchannels = true\n", true, RemoteLimits::default());
    let s = RemoteTarget::Server;
    assert!(w.h.remote_result(&s, 1, Ok(vec![0xff])));
    assert!(w.h.remote_result(&s, 2, Ok(vec![0x80; MAX_PAYLOAD + 1])));
    assert!(w.h.remote_result(&s, 3, Err(RemoteError::from_wire(77, "odd".into()))));
    let log = w.b.log_text();
    assert!(log.contains("got 1 false bad-value result:"), "{log}");
    assert!(log.contains("got 2 false too-large "), "{log}");
    assert!(log.contains("got 3 false internal odd"), "{log}");
}

#[test]
fn failures_known_at_once_arrive_as_events_after_the_call() {
    const MAIN: &str = r#"
local mod = {}
local order = {}
function mod.on_enable()
    local id = bh.services.call_remote("server", "s", "*", "m", {}, function(ok, v)
        print("callback " .. tostring(ok) .. " " .. v.kind)
    end)
    print("returned " .. id)
end
return mod
"#;
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    client_mod(&mods, "shopui", "[permissions]\nchannels = true\n", MAIN);
    fs::write(mods.join("mods.toml"), "[shopui]\ngrant = [\"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    *b.remote_fail.borrow_mut() = Some(RemoteError::Unavailable);
    let mut h = ModHost::load(HostOptions::new(&mods, Side::Client), b.clone());
    h.enable("W", "w");
    let log = b.log_text();
    let returned = log.find("returned 1").expect(&log);
    let callback = log.find("callback false unavailable").expect(&log);
    assert!(returned < callback, "{log}");
    assert!(b.remote_calls.borrow().is_empty());
    // the call is not open any more
    assert!(!h.remote_result(&RemoteTarget::Server, 1, Ok(Value::Null.encode())));
}

#[test]
fn rules_of_the_calling_side() {
    // no channels permission
    let w = client(
        r#"local mod = {} function mod.on_enable() local ok, e = pcall(bh.services.call_remote, "server", "s", "*", "m", {}, function() end); print("denied " .. tostring(e)) end return mod"#,
        "",
        false,
        RemoteLimits::default(),
    );
    assert!(w.b.has_log("not permitted: channels"), "{}", w.b.log_text());
    // too many open calls
    let w = client(
        r#"local mod = {} function mod.on_enable()
            for i = 1, 3 do print(select(2, pcall(bh.services.call_remote, "server", "s", "*", "m", {}, function() end))) end
        end return mod"#,
        "[permissions]\nchannels = true\n",
        true,
        RemoteLimits { max_pending: 2, ..RemoteLimits::default() },
    );
    assert!(w.b.has_log("limit exceeded: open remote calls"), "{}", w.b.log_text());
    assert_eq!(w.b.remote_calls.borrow().len(), 2);
    // a client does not call players, a server does not call itself
    let w = client(
        r#"local mod = {} function mod.on_enable()
            bh.commands.register({ name = "x" }, function(s) end)
            print(select(2, pcall(bh.services.call_remote, bh.players.get("p1") or 5, "s", "*", "m", {}, function() end)))
        end return mod"#,
        "[permissions]\nchannels = true\n",
        true,
        RemoteLimits::default(),
    );
    assert!(w.b.has_log("remote target is"), "{}", w.b.log_text());
}

const SERVER_CALLER: &str = r#"
local mod = {}
local alice
function mod.on_enable()
    bh.events.on("player-join", function(ev)
        alice = ev.player
        bh.services.call_remote(ev.player, "hud", "^1", "title", { 7 }, function(ok, v)
            print("title " .. tostring(ok) .. " " .. tostring(ok and v or v.kind))
        end)
    end)
    local function try(to)
        local ok, e = pcall(bh.services.call_remote, to, "hud", "^1", "title", {}, function() end)
        print("try " .. tostring(ok) .. " " .. tostring(e))
    end
    bh.commands.register({ name = "poke" }, function() try("server") end)
    bh.commands.register({ name = "poke2" }, function() try(alice) end)
end
return mod
"#;

#[test]
fn a_server_mod_calls_a_player_and_the_client_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    lua_mod(&mods, "srv", "[permissions]\nchannels = true\n", SERVER_CALLER);
    fs::write(mods.join("mods.toml"), "[srv]\ngrant = [\"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(&mods, Side::Server), b.clone());
    h.enable("W", "w");
    // the server cannot call itself
    b.join("p1", "ALICE", Role::Player);
    assert!(h.command(CommandSender::Player("p1".into()), "/poke").is_some());
    assert!(b.has_log("try false runtime error: invalid argument: a server mod calls players"), "{}", b.log_text());
    h.fire(Event::PlayerJoin { player: "p1".into() });
    let sent = b.remote_calls.borrow().clone();
    assert_eq!(sent.len(), 1, "{}", b.log_text());
    assert_eq!(sent[0].target, p1());
    assert_eq!(sent[0].service, "hud");
    // the client answers
    assert!(h.remote_result(&p1(), sent[0].id, Ok(Value::Text("Hi".into()).encode())));
    assert!(b.has_log("title true Hi"), "{}", b.log_text());
    // a player who leaves with a call open: disconnected
    h.fire(Event::PlayerJoin { player: "p1".into() });
    let id = b.remote_calls.borrow()[1].id;
    b.leave("p1");
    h.fire(Event::PlayerQuit { player: "p1".into(), was_kick: false });
    assert!(b.has_log("title false disconnected"), "{}", b.log_text());
    assert!(!h.remote_result(&p1(), id, Ok(Value::Null.encode())));
    // the player is gone: stale handle at once
    assert!(h.command(CommandSender::Player("p1".into()), "/poke2").is_some());
    assert!(b.has_log("try false runtime error: stale handle"), "{}", b.log_text());
}

#[test]
fn a_client_provides_a_remote_service_to_the_server() {
    const MAIN: &str = r#"
local mod = {}
function mod.on_enable()
    bh.services.provide("hud", "1.0.0", {
        title = function(n)
            local c = bh.services.caller()
            return "title " .. n .. " from " .. c.mod .. " " .. tostring(c.player)
        end,
    })
end
return mod
"#;
    let mut w = client(
        MAIN,
        "[permissions]\nchannels = true\n[provides]\nservices = [{ name = \"hud\", version = \"1.0.0\", wit = \"wit/e.wit\", remote = true }]\n",
        true,
        RemoteLimits::default(),
    );
    assert_eq!(w.h.mods()[0].state, ModState::Enabled, "{}", w.b.log_text());
    let r = w.h.remote_call_in(&RemoteTarget::Server, "srv", "hud", "^1", "title", &args(vec![Value::Int(7)]));
    assert_eq!(r.map(|b| Value::decode(&b).unwrap()), Ok(Value::Text("title 7 from srv nil".into())));
}

#[test]
fn a_disabled_mod_forgets_its_calls() {
    const MAIN: &str = r#"
local mod = {}
function mod.on_enable()
    bh.services.call_remote("server", "s", "*", "m", {}, function() print("must not run") end)
end
function mod.on_disable() end
return mod
"#;
    let mut w = client(MAIN, "[permissions]\nchannels = true\n", true, RemoteLimits::default());
    w.h.disable_all();
    assert!(!w.h.remote_result(&RemoteTarget::Server, 1, Ok(Value::Null.encode())));
    assert!(!w.b.has_log("must not run"));
}

#[test]
fn peer_gone_fails_open_calls() {
    let mut w = client(CLIENT, "[permissions]\nchannels = true\n", true, RemoteLimits::default());
    w.h.remote_peer_gone(&RemoteTarget::Server);
    let log = w.b.log_text();
    assert_eq!(log.matches("err disconnected []").count(), 5, "{log}");
    // nothing is open any more: the clock changes nothing
    w.clock.advance(Duration::from_secs(60));
    w.h.poll();
    assert_eq!(w.b.log_text().matches("err timeout").count(), 0);
}
