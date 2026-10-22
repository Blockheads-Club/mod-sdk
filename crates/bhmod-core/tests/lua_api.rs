//! Every `bh` function of the Luau mapping, plus the Luau runtime's limits.

mod common;

use std::fs;
use std::time::Duration;

use bhmod_core::budget::Budgets;
use bhmod_core::storage::FileStorage;
use bhmod_core::types::{CommandSender, Event, LogLevel, Role, SavePhase, Side};
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;

const API: &str = r#"
local util = require("./lib/util")
assert(require("./lib/util") == util, "require caches")
assert(require("./lib/nothing.luau") == true, "a module returning nil gives true")
assert(not pcall(require, "lib/util"), "bare names are refused")
assert(not pcall(require, "../../escape"), "no escape from the side directory")
assert(not pcall(require, "./missing"), "missing module")
assert(require("./lib/../lib/util") == util, "../ inside the side directory")
assert(debug == nil and loadstring == nil and getfenv == nil and setfenv == nil)
assert(not pcall(function() bh.log = nil end), "bh is read-only")
assert(not pcall(function() bh.log.info = nil end), "bh tables are read-only")

local mod = {}
local seen = {}

function mod.on_enable()
    assert(util.double(2) == 4)
    assert(bh.host.api_version() == "0.1.0")
    assert(bh.host.mod_id() == "api")
    assert(bh.host.mod_version() == "1.0.0")
    assert(bh.host.current_side() == "server")
    assert(bh.host.host_name() == "fake-host 1.0")
    bh.log.log("warn", "via log")
    assert(not pcall(bh.log.log, "loud", "x"))
    bh.log.trace("t"); bh.log.debug("d"); bh.log.info("i"); bh.log.warn("w"); bh.log.error("e")
    print("printed", 1, true)

    -- players
    local p = bh.players.get("p1")
    assert(p and p:id() == "p1" and p:alias() == "ALICE" and p:is_online() and p:role() == "admin")
    assert(tostring(p) == "player(p1)")
    assert(p == bh.players.find_by_alias("alice"))
    assert(p ~= bh.players.get("p2"))
    assert(bh.players.get("nobody") == nil and bh.players.find_by_alias("nobody") == nil)
    assert(#bh.players.online() == 2)
    p:send_chat("hi alice")
    bh.players.broadcast("hello all")
    local ok, err = pcall(function() p:kick("bye") end)
    assert(not ok and string.find(tostring(err), "not permitted: players:kick", 1, true), tostring(err))

    -- events
    local id = bh.events.on("tick", function(ev) seen.tick = ev.tick end)
    assert(bh.events.off(id) == true and bh.events.off(id) == false)
    assert(not pcall(bh.events.on, "explode", function() end))
    assert(not pcall(bh.events.on, "chat", function() end, { priority = "urgent" }))
    assert(not pcall(bh.events.on_custom, "api:x", function() end))
    assert(not pcall(bh.events.fire_custom, "api:x", ""))
    assert(not pcall(bh.services.provide, "x"))
    assert(not pcall(bh.channels.open, "x"))
    bh.events.on("chat", function(ev)
        ev.text = string.upper(ev.text)
        if ev.text == "STOP" then ev.cancelled = true end
    end, { priority = "high", ignore_cancelled = false })
    bh.events.on("chat", function(ev) ev.text = "monitor cannot change this"; ev.cancelled = false end, { priority = "monitor" })
    bh.events.on("command", function(ev)
        seen.command = ev.name .. ":" .. table.concat(ev.args, ",") .. ":" .. ev.raw .. ":" .. tostring(ev.sender.console)
        if ev.name == "blocked" then ev.cancelled = true end
    end)
    bh.events.on("player-join", function(ev) seen.join = ev.player:alias() end)
    bh.events.on("player-quit", function(ev) seen.quit = tostring(ev.was_kick) .. ev.player:id() .. ev.player:alias() end)
    bh.events.on("tick", function(ev) seen.tick = ev.tick .. "/" .. string.format("%.1f", ev.dt) end)
    bh.events.on("world-loaded", function(ev) seen.world = ev.world_name .. "/" .. ev.world_id end)
    bh.events.on("world-save", function(ev) seen.save = (seen.save or "") .. ev.phase end)
    local quitter = bh.events.on("player-quit", function(ev) error("not reached") end)
    bh.events.off(quitter)

    -- commands
    local cid = bh.commands.register({ name = "x", aliases = { "y", "z" }, description = "d", usage = "/x n",
        min_role = "admin", console = false }, function(sender, label, args)
        return { handled = args[1] ~= nil, reply = "label " .. label .. " by " .. sender.player:alias() }
    end)
    assert(not pcall(bh.commands.register, { name = "x" }, function() end), "taken")
    assert(not pcall(bh.commands.register, { name = "help" }, function() end), "built-in")
    assert(not pcall(bh.commands.register, { aliases = {} }, function() end), "name required")
    assert(not pcall(bh.commands.register, { name = "w", min_role = "king" }, function() end))
    assert(not pcall(bh.commands.register, { name = "w", aliases = 5 }, function() end))
    bh.commands.register({ name = "nilreply" }, function() return nil end)
    bh.commands.register({ name = "strreply" }, function() return "just text" end)
    bh.commands.register({ name = "badreply" }, function() return 42 end)
    bh.commands.register({ name = "listreply" }, function() return { reply = { "a", "b" } } end)
    bh.commands.register({ name = "fails" }, function() error("boom") end)
    bh.commands.register({ name = "spin" }, function() while true do end end)
    bh.commands.register({ name = "sneaky" }, function()
        for _ = 1, 3 do pcall(function() while true do end end) end
        return "escaped?"
    end)
    bh.commands.register({ name = "hog" }, function()
        local t = {}
        for i = 1, 1e9 do t[i] = string.rep("x", 1048576) .. i end
    end)
    bh.commands.register({ name = "seen" }, function()
        return { reply = { seen.tick or "-", seen.command or "-", seen.join or "-", seen.quit or "-", seen.world or "-", seen.save or "-" } }
    end)
    local gone = bh.commands.register({ name = "gone" }, function() end)
    assert(bh.commands.unregister(gone) == true and bh.commands.unregister(gone) == false)
    assert(cid > 0)

    -- scheduler
    assert(not pcall(bh.scheduler.run_every, 1, 0, function() end), "period 0")
    local t = bh.scheduler.run_later(1, function() seen.never = true end)
    assert(bh.scheduler.cancel(t) == true and bh.scheduler.cancel(t) == false)
    bh.scheduler.run_later(0, function() bh.log.info("later ran") end)
    bh.scheduler.run_next_poll(function() bh.log.info("poll ran") end)

    -- storage
    assert(bh.storage.get("missing") == nil)
    bh.storage.set("bin", "\0\1\2")
    assert(bh.storage.get("bin") == "\0\1\2")
    bh.storage.set("a:1", "x"); bh.storage.set("a:2", "y")
    local keys = bh.storage.keys("a:")
    assert(#keys == 2 and keys[1] == "a:1")
    assert(#bh.storage.keys() == 3)
    assert(bh.storage.delete("a:1") == true and bh.storage.delete("a:1") == false)
    assert(not pcall(bh.storage.set, "", "x"))
end

function mod.on_disable()
    bh.log.info("api off")
end

return mod
"#;

fn api_host(budgets: Budgets) -> (tempfile::TempDir, std::rc::Rc<FakeBridge>, ModHost) {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    lua_mod(&mods, "api", "[permissions]\nplayers = [\"chat\"]\n", API);
    fs::write(mods.join("mods.toml"), "[api]\ngrant = [\"players:chat\"]\n").unwrap();
    fs::create_dir_all(mods.join("api/server/lib")).unwrap();
    fs::write(mods.join("api/server/lib/util.luau"), "return { double = function(x) return x * 2 end }").unwrap();
    fs::write(mods.join("api/server/lib/nothing.luau"), "local x = 1").unwrap();
    let b = FakeBridge::new();
    b.join("p1", "ALICE", Role::Admin);
    b.join("p2", "BOB", Role::Player);
    let mut opts = HostOptions::new(&mods, Side::Server);
    // A generous hard budget: instrumented debug builds are slow, and the timeout tests only need a finite one.
    opts.budgets = Budgets { hard: Duration::from_millis(250), ..budgets };
    opts.storage = Box::new(FileStorage::new(tmp.path().join("world-storage")));
    let h = ModHost::load(opts, b.clone());
    (tmp, b, h)
}

fn p(id: &str) -> CommandSender {
    CommandSender::Player(id.into())
}

#[test]
fn whole_api() {
    let (tmp, b, mut h) = api_host(Budgets::default());
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    let logs = b.logs.borrow().clone();
    for (lvl, msg) in [
        (LogLevel::Warn, "via log"),
        (LogLevel::Trace, "t"),
        (LogLevel::Debug, "d"),
        (LogLevel::Info, "i"),
        (LogLevel::Warn, "w"),
        (LogLevel::Error, "e"),
        (LogLevel::Info, "printed\t1\ttrue"),
    ] {
        assert!(logs.contains(&(Some("api".into()), lvl, msg.into())), "{lvl:?} {msg}\n{}", b.log_text());
    }
    assert_eq!(b.take_chats(), vec![("p1".into(), "hi alice".into())]);
    assert_eq!(b.take_broadcasts(), vec!["hello all"]);

    // Events.
    let ev = h.fire(Event::Chat { player: "p2".into(), text: "stop".into(), cancelled: false });
    assert_eq!(ev, Event::Chat { player: "p2".into(), text: "STOP".into(), cancelled: true });
    h.fire(Event::PlayerJoin { player: "p2".into() });
    h.fire(Event::PlayerQuit { player: "p2".into(), was_kick: true });
    h.tick(0.2);
    h.poll();
    assert!(b.has_log("later ran") && b.has_log("poll ran"));
    h.world_save(SavePhase::Pre);
    h.world_save(SavePhase::Post);
    assert!(tmp.path().join("world-storage/api.kv").is_file());
    assert_eq!(h.command(p("p1"), "/blocked a b"), Some(vec![]));
    assert_eq!(
        h.command(p("p1"), "/seen"),
        Some(vec![
            "1/0.2".to_string(),
            "seen::/seen:nil".to_string(),
            "BOB".to_string(),
            "truep2BOB".to_string(),
            "World/wid".to_string(),
            "prepost".to_string()
        ])
    );
    h.command(CommandSender::Console, "/seen");

    // Commands.
    assert_eq!(h.command(p("p2"), "/x 1"), Some(vec!["You do not have permission to use /x.".into()]));
    assert_eq!(h.command(CommandSender::Console, "/x 1"), Some(vec!["/x cannot be used from the console.".into()]));
    assert_eq!(h.command(p("p1"), "/Z 1"), Some(vec!["label z by ALICE".into()]));
    assert_eq!(h.command(p("p1"), "/y"), Some(vec!["label y by ALICE".into(), "Usage: /x n".into()]));
    assert_eq!(h.command(p("p1"), "/nilreply"), Some(vec![]));
    assert_eq!(h.command(p("p1"), "/strreply"), Some(vec!["just text".into()]));
    assert_eq!(h.command(p("p1"), "/listreply"), Some(vec!["a".into(), "b".into()]));
    assert_eq!(h.command(p("p1"), "/gone"), None);
    assert_eq!(h.command(p("p1"), "/"), None);
    assert_eq!(h.command(p("p1"), "plain"), None);

    // Faults: three strikes within the window disable the mod (on_disable still runs).
    let err = Some(vec!["An error occurred while running /badreply.".to_string()]);
    assert_eq!(h.command(p("p1"), "/badreply"), err);
    assert_eq!(h.command(p("p1"), "/fails"), Some(vec!["An error occurred while running /fails.".to_string()]));
    assert!(b.has_log("boom"), "{}", b.log_text());
    assert_eq!(h.mods()[0].state, ModState::Enabled);
    let start = std::time::Instant::now();
    h.command(p("p1"), "/spin");
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(b.has_log("hard time budget exceeded"));
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(r) if r == "3 faults within 60 s"), "{:?}", h.mods());
    assert!(b.has_log("api off"));
    assert_eq!(h.command(p("p1"), "/seen"), None, "commands are gone");
    h.tick(0.2);
    h.disable_all();
}

#[test]
fn timeouts_cannot_be_caught_and_memory_is_limited() {
    let budgets = Budgets { strikes: 100, memory_bytes: 8 * 1024 * 1024, ..Budgets::default() };
    let (_tmp, b, mut h) = api_host(budgets);
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    assert_eq!(h.command(p("p1"), "/sneaky"), Some(vec!["An error occurred while running /sneaky.".to_string()]));
    assert!(b.has_log("hard time budget exceeded"));
    assert!(b.has_log("budget 5 ms"), "soft budget warning\n{}", b.log_text());
    assert_eq!(h.command(p("p1"), "/hog"), Some(vec!["An error occurred while running /hog.".to_string()]));
    assert!(b.has_log("out of memory"), "{}", b.log_text());
    // Still alive afterwards.
    assert_eq!(h.command(p("p1"), "/strreply"), Some(vec!["just text".into()]));
}

fn single(main: &str) -> (tempfile::TempDir, std::rc::Rc<FakeBridge>, ModHost) {
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(tmp.path(), "solo", "", main);
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(tmp.path(), Side::Server), b.clone());
    h.enable("W", "w");
    (tmp, b, h)
}

#[test]
fn entry_point_outcomes() {
    let (_t, b, h) = single("return 5");
    assert!(
        matches!(&h.mods()[0].state, ModState::Disabled(r) if r.contains("must return a table or nil")),
        "{}",
        b.log_text()
    );
    let (_t, _b, h) = single("error('broken main')");
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(r) if r.contains("broken main")));
    let (_t, _b, h) = single("return { on_enable = function() error('nope') end }");
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(r) if r.starts_with("on-enable failed: failed:")));
    let (_t, _b, h) = single("while true do end");
    assert!(matches!(&h.mods()[0].state, ModState::Disabled(r) if r.contains("hard time budget")));
    // nil module: no callbacks at all.
    let (_t, _b, mut h) = single("bh.log.info('top level')");
    assert_eq!(h.mods()[0].state, ModState::Enabled);
    h.disable_all();
    // on_disable that faults.
    let (_t, b, mut h) = single("return { on_disable = function() error('bad bye') end }");
    h.disable_all();
    assert!(b.has_log("on-disable failed"), "{}", b.log_text());
    // A module table without on_disable.
    let (_t, _b, mut h) = single("return {}");
    h.disable_all();
    // A missing entry file cannot be instantiated.
    let tmp = tempfile::tempdir().unwrap();
    let files = bhmod_core::runtime::ModFiles { entry: "server/main.luau".into(), files: Default::default() };
    let rt = bhmod_core::lua::LuauRuntime;
    use bhmod_core::Runtime;
    let err = rt.instantiate("x", &files, bhmod_core::runtime::InstanceLimits { memory_bytes: 1 << 20 }).err().unwrap();
    assert!(err.contains("entry point server/main.luau is missing"));
    drop(tmp);
}

#[test]
fn events_for_unknown_subscriptions_and_tasks() {
    // A handler unsubscribed during dispatch of the same event still gets the snapshot call: the runtime answers
    // with the current state. Same for a task cancelled after it was taken.
    let (_t, b, mut h) = single(
        r#"
        local mod = {}
        local second
        function mod.on_enable()
            bh.events.on("chat", function(ev) bh.events.off(second); ev.cancelled = true end, { priority = "low" })
            second = bh.events.on("chat", function(ev) ev.text = "never" end)
            bh.events.on("chat", function(ev) ev.text = "late" end, { priority = "high", ignore_cancelled = true })
            local a, c
            a = bh.scheduler.run_later(1, function() bh.scheduler.cancel(c) end)
            c = bh.scheduler.run_later(1, function() bh.log.info("c ran") end)
            bh.commands.register({ name = "kickme" }, function(s) s.player:kick("bye") end)
        end
        return mod
        "#,
    );
    let ev = h.fire(Event::Chat { player: "p".into(), text: "t".into(), cancelled: false });
    assert_eq!(ev, Event::Chat { player: "p".into(), text: "t".into(), cancelled: true });
    h.tick(0.2);
    assert!(!b.has_log("c ran"), "{}", b.log_text());
    // A player handle that went stale: alias / role give defaults, send_chat raises.
    b.join("q", "Q", Role::Owner);
    assert_eq!(
        h.command(CommandSender::Player("q".into()), "/kickme"),
        Some(vec!["An error occurred while running /kickme.".to_string()])
    );
}
