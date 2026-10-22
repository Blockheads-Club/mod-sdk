//! API 0.2 in Luau: gameplay events, services, custom events, channels, values and handles across mods.

mod common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use bhmod_core::budget::Budgets;
use bhmod_core::types::*;
use bhmod_core::value::Value;
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;

const ECO_TOML: &str = "[permissions]\nchannels = true\n[provides]\nservices = [{ name = \"economy\", version = \"1.2.0\", wit = \"wit/economy.wit\" }]\n";

const ECO: &str = r#"
local balances = {}
local mod = {}
function mod.on_enable()
    bh.services.provide("economy", "1.2.0", {
        balance = function(p) return balances[p:id()] or 0 end,
        give = function(p, n)
            if type(n) ~= "number" or n < 0 then error("bad amount", 0) end
            balances[p:id()] = (balances[p:id()] or 0) + n
            bh.events.fire_custom("eco:changed", { player = p, balance = balances[p:id()] })
            return balances[p:id()]
        end,
        echo = function(...) return { ... } end,
        nothing = function() end,
        spin = function() while true do end end,
        callback = function() return bh.services.call("shopapi", "^1", "ping") end,
        bad = function() return function() end end,
        stale = function(p) return p:id() end,
    })
    assert(not pcall(bh.services.provide, "economy", "9.0.0", {}), "version must match mod.toml")
    assert(not pcall(bh.services.provide, "economy", "x", {}), "semver")
    bh.events.on_custom("eco:changed", function(ev) bh.log.info("eco saw its own event later") end)
    bh.events.on_custom("shop:sale", function(ev)
        bh.log.info("eco saw sale " .. ev.payload.item .. " from " .. ev.source)
        ev.payload = { item = ev.payload.item, taxed = true }
        ev.cancelled = ev.payload.item == "cursed"
    end, { priority = "low" })
    local ch = bh.channels.open("eco:bus", function(origin, data)
        bh.log.info("eco got " .. data .. " from " .. tostring(origin.mod))
    end)
    assert(ch > 0 and bh.channels.open("eco:bus") == ch)
    assert(not pcall(bh.channels.open, "shop:bus"), "not a dependency")
    assert(not pcall(bh.channels.open, "bad name"))
end
return mod
"#;

const SHOP_TOML: &str = "[dependencies]\neco = \"^1\"\n[permissions]\nchannels = true\n[provides]\nservices = [{ name = \"shopapi\", version = \"1.0.0\", wit = \"wit/shop.wit\" }]\n";

const SHOP: &str = r#"
local mod = {}
local got = {}
function mod.on_enable()
    bh.services.provide("shopapi", "1.0.0", { ping = function() return "pong" end })
    local eco = bh.services.get("economy", "^1.1")
    bh.events.on_custom("eco:changed", function(ev)
        got.changed = ev.payload.player:id() .. "=" .. ev.payload.balance .. " by " .. ev.source
    end)
    bh.events.on_custom("shop:sale", function(ev)
        got.sale = tostring(ev.payload.taxed) .. "/" .. tostring(ev.cancelled)
        ev.cancelled = true
    end, { priority = "high" })
    bh.events.on_custom("shop:sale", function(ev) ev.cancelled = false; ev.payload = "monitor" end, { priority = "monitor" })
    local ch = bh.channels.open("eco:bus", function(origin, data) got.msg = data .. "<" .. tostring(origin.mod) .. tostring(origin.server) .. tostring(origin.player and origin.player:id()) end)
    local mine = bh.channels.open("shop:bus", nil)

    bh.commands.register({ name = "shoptest" }, function(sender)
        local p = sender.player
        local out = {}
        table.insert(out, tostring(eco.balance(p)))
        table.insert(out, tostring(eco.give(p, 5)))
        table.insert(out, tostring(bh.services.call("economy", "^1", "balance", p)))
        local e = eco.echo(1, 2.5, "x", true, { a = 1 }, { 1, 2 }, {})
        table.insert(out, #e .. ":" .. e[1] .. ":" .. e[2] .. ":" .. e[3] .. ":" .. tostring(e[4]) .. ":" .. e[5].a .. ":" .. e[6][2] .. ":" .. #e[7])
        table.insert(out, tostring(eco.nothing()))
        local function err(f, ...) local ok, m = pcall(f, ...); assert(not ok); return tostring(m) end
        table.insert(out, err(eco.give, p, -1))
        table.insert(out, err(bh.services.call, "economy", "^2", "balance", p))
        table.insert(out, err(bh.services.call, "nothing", "^1", "x"))
        table.insert(out, err(bh.services.call, "economy", "nonsense", "x"))
        table.insert(out, err(eco.callback))
        table.insert(out, err(eco.missing))
        table.insert(out, err(eco.bad))
        table.insert(out, err(eco.echo, function() end))
        table.insert(out, err(eco.echo, { [true] = 1 }))
        table.insert(out, err(eco.echo, setmetatable({}, { __index = function() end }), coroutine.create(function() end)))
        table.insert(out, tostring(bh.events.fire_custom("shop:sale", { item = "sword" })))
        table.insert(out, tostring(bh.events.fire_custom("shop:sale", { item = "cursed" })))
        table.insert(out, tostring(got.sale) .. "/" .. tostring(got.changed))
        table.insert(out, tostring(bh.events.fire_custom("shop:nobody", nil)))
        bh.channels.send(ch, "local", "hello")
        bh.channels.send(mine, p, "to client")
        bh.channels.send(mine, "all-players", "to all")
        table.insert(out, err(bh.channels.send, mine, "server", "x"))
        table.insert(out, err(bh.channels.send, mine, "moon", "x"))
        table.insert(out, err(bh.channels.send, mine, 5, "x"))
        table.insert(out, err(bh.channels.send, 999, "local", "x"))
        table.insert(out, err(bh.channels.send, ch, "local", string.rep("x", 32 * 1024 + 1)))
        local bin = bh.value.encode({ n = -3, f = 0.5, s = "\255" })
        local back = bh.value.decode(bin)
        table.insert(out, back.n .. ":" .. back.f .. ":" .. #back.s .. ":" .. #bin)
        table.insert(out, err(bh.value.decode, "\255"))
        local nested = {}
        local cur = nested
        for _ = 1, 40 do cur[1] = {}; cur = cur[1] end
        table.insert(out, err(bh.value.encode, nested))
        local pos = p:pos()
        table.insert(out, pos and (pos.x .. "," .. pos.y) or "nowhere")
        return { reply = out }
    end)
    bh.commands.register({ name = "after" }, function()
        assert(bh.channels.close(mine) and not bh.channels.close(mine))
        return { reply = { got.msg or "-", got.sale, got.changed } }
    end)
    bh.commands.register({ name = "spin" }, function(sender)
        local ok, m = pcall(eco.spin)
        return tostring(m)
    end)
    local saved
    bh.commands.register({ name = "stale" }, function(sender)
        saved = saved or bh.value.encode(sender.player)
        local ok, m = pcall(function() return eco.stale(bh.value.decode(saved)) end)
        return tostring(m)
    end)
end
return mod
"#;

const STRANGER: &str = r#"
return { on_enable = function()
    bh.commands.register({ name = "steal" }, function()
        local ok, m = pcall(bh.services.call, "economy", "^1", "balance")
        return tostring(m)
    end)
end }
"#;

fn add_wit(mods: &Path, id: &str, file: &str) {
    fs::create_dir_all(mods.join(id).join("wit")).unwrap();
    fs::write(mods.join(id).join("wit").join(file), "package x:y;\n").unwrap();
}

fn setup() -> (tempfile::TempDir, std::rc::Rc<FakeBridge>, ModHost) {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    lua_mod(&mods, "eco", ECO_TOML, ECO);
    add_wit(&mods, "eco", "economy.wit");
    lua_mod(&mods, "shop", SHOP_TOML, SHOP);
    add_wit(&mods, "shop", "shop.wit");
    lua_mod(&mods, "stranger", "", STRANGER);
    fs::write(mods.join("mods.toml"), "[shop]\ngrant = [\"channels\"]\n[eco]\ngrant = [\"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    b.join("p1", "ALICE", Role::Player);
    b.positions.borrow_mut().push(("p1".into(), Pos { x: 7, y: 9 }));
    let mut opts = HostOptions::new(&mods, Side::Server);
    opts.budgets = Budgets { hard: Duration::from_millis(250), ..Budgets::default() };
    let mut h = ModHost::load(opts, b.clone());
    h.enable("W", "w");
    for m in h.mods() {
        assert_eq!(m.state, ModState::Enabled, "{}", b.log_text());
    }
    (tmp, b, h)
}

fn p1() -> CommandSender {
    CommandSender::Player("p1".into())
}

#[test]
fn services_custom_events_channels_values() {
    let (_tmp, b, mut h) = setup();
    let out = h.command(p1(), "/shoptest").unwrap();
    let want = [
        "0",
        "5",
        "5",
        "7:1:2.5:x:true:1:2:0",
        "nil",
        "service economy.give: provider-error: bad amount",
        "service economy.balance: version-mismatch",
        "service nothing.x: not-found",
        "service economy.x: bad-value: \"nonsense\" is not a semver requirement",
        "service economy.callback: provider-error: service shopapi.ping: reentrant",
        "service economy.missing: provider-error: economy has no method \"missing\"",
        "service economy.bad: provider-error: bad value: cannot convert a function",
        "bad value: cannot convert a function",
        "bad value: a table must be a list (keys 1..n) or have only string keys",
        "bad value: cannot convert a thread",
        "false",
        "true",
        "nil/nil",
        "false",
        "unsupported",
        "invalid argument: channel target \"moon\"",
        "invalid argument: channel target",
        "invalid argument: channel 999 is not open",
        "limit exceeded: channel message size",
        "-3:0.5:1:",
        "bad value: unsupported simple value",
        "bad value: nested too deeply",
        "7,9",
    ];
    assert_eq!(out.len(), want.len(), "{out:#?}\n{}", b.log_text());
    for (got, want) in out.iter().zip(want) {
        assert!(got.contains(want) || got == want, "got {got:?}, want {want:?}\n{out:#?}");
    }
    assert!(b.has_log("eco saw sale sword from shop"), "{}", b.log_text());
    assert!(b.has_log("eco saw its own event later"), "{}", b.log_text());
    let sent = b.sent.borrow().clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], ("shop".into(), "shop:bus".into(), ChannelTarget::Player("p1".into()), b"to client".to_vec()));
    assert_eq!(sent[1].2, ChannelTarget::AllPlayers);
    // Local channel messages arrive with the next poll, not during the send.
    assert!(!b.has_log("eco got hello"));
    h.poll();
    assert!(b.has_log("eco got hello from shop"), "{}", b.log_text());
    // Network input (milestone M5) reaches every mod that opened the channel.
    assert!(h.channel_receive("eco:bus", ChannelSource::Player("p1".into()), b"net".to_vec()));
    assert!(h.channel_receive("eco:bus", ChannelSource::Server, b"srv".to_vec()));
    h.tick(0.2);
    assert!(b.has_log("eco got net from nil"));
    // The shop's own sale handlers ran after /shoptest returned (last one: the cursed sale, which eco cancelled).
    assert_eq!(
        h.command(p1(), "/after"),
        Some(vec!["srv<niltruenil".into(), "true/true".into(), "p1=5 by eco".into()])
    );
    assert!(h.command(p1(), "/steal").unwrap()[0].contains("service economy.balance: not-permitted"));
}

#[test]
fn nested_timeouts_and_stale_handles() {
    let (_tmp, b, mut h) = setup();
    // The nested call shares the caller's deadline: both run out, both get a strike.
    let r = h.command(p1(), "/spin").unwrap();
    assert_eq!(r, vec!["An error occurred while running /spin.".to_string()], "{}", b.log_text());
    assert!(b.has_log("service economy.spin: hard time budget exceeded"), "{}", b.log_text());
    assert!(b.has_log("command /spin: hard time budget exceeded"), "{}", b.log_text());
    // The handle survives a round trip while the player is online, and goes stale when they leave.
    assert_eq!(h.command(p1(), "/stale"), Some(vec!["p1".into()]));
    h.fire(Event::PlayerQuit { player: "p1".into(), was_kick: false });
    let r = h.command(p1(), "/stale").unwrap();
    assert!(r[0].contains("stale handle"), "{r:?}");
}

const EVENTS: &str = r#"
local seen = {}
return { on_enable = function()
    bh.events.on("player-pre-login", function(ev)
        seen.pre = ev.player_id .. "/" .. ev.alias .. "/" .. ev.reason
        if ev.alias == "BAD" then ev.cancelled = true; ev.reason = "banned" end
        if ev.alias == "WEIRD" then ev.reason = "sleepy" end
    end)
    bh.events.on("block-place", function(ev)
        seen.place = ev.player:id() .. "@" .. ev.pos.x .. "," .. ev.pos.y .. ":" .. ev.item .. "/" .. ev.data_a .. "/" .. ev.data_b
        ev.cancelled = ev.item == 13
    end)
    bh.events.on("block-break", function(ev)
        seen["break"] = ev.layer .. ":" .. ev.tile
        ev.cancelled = ev.pos.x < 0
    end)
    bh.events.on("block-changed", function(ev)
        seen.changed = ev.tile .. ":" .. tostring(ev.player and ev.player:id())
        ev.cancelled = true
    end)
    bh.events.on("object-place", function(ev)
        seen.object = ev.object .. ":" .. ev.item
        ev.cancelled = true
    end)
    bh.events.on("damage", function(ev)
        seen.damage = ev.blockhead .. ":" .. ev.amount .. ":" .. tostring(ev.attacker and ev.attacker:id())
        if ev.amount > 1 then ev.cancelled = true else ev.amount = ev.amount / 2 end
    end)
    bh.events.on("death", function(ev) seen.death = tostring(ev.player and ev.player:id()) .. ":" .. ev.blockhead end)
    bh.events.on("player-move", function(ev)
        seen.move = ev.from.x .. "," .. ev.from.y .. ">" .. ev.to.x .. "," .. ev.to.y
    end)
    bh.commands.register({ name = "seen" }, function()
        local keys = {}
        for k, v in pairs(seen) do table.insert(keys, k .. "=" .. v) end
        table.sort(keys)
        return { reply = keys }
    end)
end }
"#;

#[test]
fn gameplay_events_in_luau() {
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(tmp.path(), "ev", "", EVENTS);
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(tmp.path(), Side::Server), b.clone());
    assert!(!h.wants(EventKind::Damage), "not enabled yet");
    h.enable("W", "w");
    assert!(h.wants(EventKind::Damage) && !h.wants(EventKind::Tick));
    let pos = Pos { x: 3, y: 4 };
    let pre = |alias: &str| Event::PlayerPreLogin {
        player: "p9".into(),
        alias: alias.into(),
        cancelled: false,
        reason: RejectReason::Kicked,
    };
    assert_eq!(
        h.fire(pre("BAD")),
        Event::PlayerPreLogin {
            player: "p9".into(),
            alias: "BAD".into(),
            cancelled: true,
            reason: RejectReason::Banned
        }
    );
    assert_eq!(h.fire(pre("OK")), pre("OK"));
    assert!(!h.fire(pre("WEIRD")).cancelled());
    assert!(b.has_log("invalid argument: reject reason \"sleepy\""), "{}", b.log_text());
    let place = |item| Event::BlockPlace { player: "p1".into(), pos, item, data_a: 1, data_b: 2, cancelled: false };
    assert!(h.fire(place(13)).cancelled());
    assert!(!h.fire(place(14)).cancelled());
    let brk = |x| Event::BlockBreak {
        player: "p1".into(),
        pos: Pos { x, y: 1 },
        layer: BreakLayer::BackWall,
        tile: 6,
        cancelled: false,
    };
    assert!(h.fire(brk(-1)).cancelled());
    assert!(!h.fire(brk(1)).cancelled());
    let changed = Event::BlockChanged { pos, tile: 9, player: Some("p1".into()) };
    assert_eq!(h.fire(changed.clone()), changed, "monitor-only kinds ignore cancel");
    h.fire(Event::BlockChanged { pos, tile: 8, player: None });
    let obj = Event::ObjectPlace {
        player: "p1".into(),
        pos,
        object: ObjectKind::InteractionObject,
        item: 1047,
        cancelled: false,
    };
    assert!(h.fire(obj).cancelled());
    let dmg = |amount| Event::Damage {
        player: Some("p1".into()),
        blockhead: 77,
        amount,
        attacker: Some("p2".into()),
        cancelled: false,
    };
    assert!(h.fire(dmg(5.0)).cancelled());
    assert!(matches!(h.fire(dmg(0.5)), Event::Damage { amount, cancelled: false, .. } if amount == 0.25));
    h.fire(Event::Damage { player: None, blockhead: 1, amount: 0.5, attacker: None, cancelled: false });
    h.fire(Event::Death { player: Some("p1".into()), blockhead: 77 });
    h.fire(Event::PlayerMove { player: "p1".into(), from: pos, to: Pos { x: 4, y: 4 } });
    let seen = h.command(CommandSender::Console, "/seen").unwrap();
    assert_eq!(
        seen,
        vec![
            "break=back-wall:6",
            "changed=8:nil",
            "damage=1:0.5:nil",
            "death=p1:77",
            "move=3,4>4,4",
            "object=interaction-object:1047",
            "place=p1@3,4:14/1/2",
            "pre=p9/WEIRD/kicked",
        ],
        "{}",
        b.log_text()
    );
}

#[test]
fn values_round_trip_through_cbor() {
    // Payload mutation with bytes that are not UTF-8 survives as a byte string.
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(
        tmp.path(),
        "vv",
        "",
        r#"return { on_enable = function()
            bh.commands.register({ name = "v" }, function()
                local b = bh.value.encode({ 1, "\0\255", { k = false } })
                local d = bh.value.decode(b)
                return { reply = { #b .. ":" .. #d[2] .. ":" .. tostring(d[3].k), tostring(bh.value.decode(bh.value.encode(nil))) } }
            end)
        end }"#,
    );
    let b = FakeBridge::new();
    let mut h = ModHost::load(HostOptions::new(tmp.path(), Side::Server), b.clone());
    h.enable("W", "w");
    assert_eq!(h.command(CommandSender::Console, "/v"), Some(vec!["9:2:false".into(), "nil".into()]));
    let v = Value::List(vec![
        Value::Int(1),
        Value::Bytes(vec![0, 255]),
        Value::Map(vec![("k".into(), Value::Bool(false))]),
    ]);
    assert_eq!(v.encode().len(), 9);
}

#[test]
fn luau_edge_cases() {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path();
    // Two mods that keep firing at each other from deferred deliveries: the host stops after a bounded number.
    lua_mod(
        mods,
        "ping",
        "[provides]\nservices = [{ name = \"hog\", version = \"1.0.0\", wit = \"wit/economy.wit\" }]\n",
        r#"return { on_enable = function()
            bh.services.provide("hog", "1.0.0", { eat = function()
                local t = {}
                for i = 1, 1e9 do t[i] = string.rep("x", 1048576) .. i end
            end })
            bh.events.on_custom("pong:x", function() bh.events.fire_custom("ping:x", 1) end)
            bh.commands.register({ name = "start" }, function() bh.events.fire_custom("ping:x", 1) end)
        end }"#,
    );
    add_wit(mods, "ping", "economy.wit");
    lua_mod(
        mods,
        "pong",
        "[dependencies]\nping = \"^1\"\n",
        r#"return { on_enable = function()
            bh.events.on_custom("ping:x", function() bh.events.fire_custom("pong:x", 1) end)
            bh.commands.register({ name = "eat" }, function()
                local ok, m = pcall(bh.services.call, "hog", "^1", "eat")
                return tostring(m)
            end)
            bh.commands.register({ name = "where" }, function(s) return tostring(s.player:pos()) end)
            bh.commands.register({ name = "proxy" }, function(s)
                local a = select(2, pcall(bh.value.encode, newproxy()))
                local b = select(2, pcall(bh.channels.send, 1, newproxy(), "x"))
                return { reply = { tostring(a), tostring(b) } }
            end)
        end }"#,
    );
    lua_mod(mods, "broken", "", "this is not luau");
    let b = FakeBridge::new();
    b.join("p1", "A", Role::Player);
    let mut opts = HostOptions::new(mods, Side::Server);
    opts.budgets =
        Budgets { memory_bytes: 8 << 20, strikes: 100, hard: Duration::from_millis(250), ..Budgets::default() };
    let mut h = ModHost::load(opts, b.clone());
    h.enable("W", "w");
    assert!(
        matches!(&h.mods()[0].state, ModState::Disabled(r) if r.contains("syntax error") || r.contains("Incomplete")),
        "{:?}",
        h.mods()
    );
    h.command(p1(), "/start");
    assert!(b.has_log("dropped"), "{}", b.log_text());
    let r = h.command(p1(), "/eat").unwrap();
    assert!(r[0].contains("service hog.eat: trap"), "out of memory is a fault, not a provider error: {r:?}");
    assert!(b.has_log("out of memory"), "{}", b.log_text());
    assert_eq!(h.command(p1(), "/where"), Some(vec!["nil".into()]));
    let r = h.command(p1(), "/proxy").unwrap();
    assert!(r[0].contains("unsupported userdata") && r[1].contains("channel target"), "{r:?}");
}

#[test]
fn interop_limits() {
    let tmp = tempfile::tempdir().unwrap();
    lua_mod(
        tmp.path(),
        "lim",
        "[permissions]\nchannels = true\n",
        r#"return { on_enable = function()
            bh.commands.register({ name = "limits" }, function(s)
                local out = {}
                local ok, m = pcall(function() for i = 1, 2000 do bh.events.on_custom("lim:x", function() end) end end)
                table.insert(out, tostring(m))
                ok, m = pcall(function() for i = 1, 100 do bh.channels.open("lim:c" .. i) end end)
                table.insert(out, tostring(m))
                local ch = bh.channels.open("lim:c1")
                ok, m = pcall(function() for i = 1, 5000 do bh.channels.send(ch, "local", "x") end end)
                table.insert(out, tostring(m))
                local p = bh.players.get("p1")
                table.insert(out, tostring(p ~= nil))
                return { reply = out }
            end)
            bh.commands.register({ name = "pos" }, function(s) return tostring(s.player:pos()) end)
        end }"#,
    );
    fs::write(tmp.path().join("mods.toml"), "[lim]\ngrant = [\"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    b.join("p1", "A", Role::Player);
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.budgets = Budgets { hard: Duration::from_secs(5), ..Budgets::default() };
    let mut h = ModHost::load(opts, b.clone());
    h.enable("W", "w");
    let r = h.command(p1(), "/limits").unwrap();
    assert!(r[0].contains("limit exceeded: subscriptions per mod"), "{r:?}");
    assert!(r[1].contains("limit exceeded: channels per mod"), "{r:?}");
    assert!(r[2].contains("limit exceeded: queued channel messages"), "{r:?}");
    b.leave("p1");
    assert_eq!(h.command(p1(), "/pos"), Some(vec!["nil".into()]), "a player who left has no position");
}
