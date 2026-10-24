//! The SDK's Luau examples, run by the real host against a fake game.

mod common;

use std::fs;

use bhmod_core::types::{CommandSender, Event, Role, SavePhase, Side};
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;

fn host_with(examples_: &[&str], grants: &str) -> (tempfile::TempDir, std::rc::Rc<FakeBridge>, ModHost) {
    let tmp = tempfile::tempdir().unwrap();
    for e in examples_ {
        copy_dir(&examples().join(e), &tmp.path().join(e));
    }
    fs::write(tmp.path().join("mods.toml"), grants).unwrap();
    let bridge = FakeBridge::new();
    let host = ModHost::load(HostOptions::new(tmp.path(), Side::Server), bridge.clone());
    (tmp, bridge, host)
}

fn player(id: &str) -> CommandSender {
    CommandSender::Player(id.into())
}

#[test]
fn hello_lua() {
    let (_t, b, mut h) = host_with(&["hello-lua"], "[hello-lua]\ngrant = [\"players:chat\"]\n");
    assert_eq!(h.mods().len(), 1, "{}", b.log_text());
    assert!(b.has_log("loaded 1 mod(s): hello-lua 0.1.0"));
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    assert!(b.has_log("hello-lua enabled"));

    b.join("p1", "ALICE", Role::Player);
    h.fire(Event::PlayerJoin { player: "p1".into() });
    h.fire(Event::PlayerJoin { player: "p1".into() });
    assert_eq!(
        b.take_chats(),
        vec![
            ("p1".to_string(), "Hello, ALICE! You are greeting number 1.".to_string()),
            ("p1".to_string(), "Hello, ALICE! You are greeting number 2.".to_string()),
        ]
    );

    assert!(h.owns_command("/hello"));
    assert!(h.owns_command("/HI there"));
    assert!(!h.owns_command("/help"));
    assert!(!h.owns_command("hello"));
    assert_eq!(h.command(player("p1"), "/hello"), Some(vec!["Hello, ALICE!".to_string()]));
    assert_eq!(h.command(CommandSender::Console, "/hi\n"), Some(vec!["Hello, console!".to_string()]));
    assert_eq!(b.take_broadcasts(), vec!["ALICE says hello to everyone!", "console says hello to everyone!"]);
    assert_eq!(h.command(player("p1"), "/help"), None);

    let ev = h.fire(Event::Chat { player: "p1".into(), text: "Lua is BAD".into(), cancelled: false });
    assert!(ev.cancelled());
    let ev = h.fire(Event::Chat { player: "p1".into(), text: "hi".into(), cancelled: false });
    assert!(!ev.cancelled());

    h.disable_all();
    assert!(b.has_log("hello-lua disabled"));
    assert!(matches!(h.mods()[0].state, ModState::Disabled(_)));
}

#[test]
fn chat_filter() {
    let (_t, b, mut h) = host_with(&["chat-filter"], "");
    h.enable("World", "wid");
    b.join("p1", "BOB", Role::Player);
    b.join("m1", "MOD", Role::Moderator);
    let ev =
        h.fire(Event::Chat { player: "p1".into(), text: "get stuff at SPAM.example now".into(), cancelled: false });
    assert!(ev.cancelled());
    assert_eq!(
        b.take_chats(),
        vec![("p1".into(), "Your message was blocked: links to spam.example are not allowed.".into())]
    );
    assert!(b.has_log("blocked a message from BOB"));
    let ev = h.fire(Event::Chat { player: "p1".into(), text: "darn, what the heck".into(), cancelled: false });
    assert_eq!(ev, Event::Chat { player: "p1".into(), text: "****, what the ****".into(), cancelled: false });

    assert_eq!(
        h.command(player("p1"), "/chatfilter off"),
        Some(vec!["You do not have permission to use /chatfilter.".into()])
    );
    assert_eq!(
        h.command(player("m1"), "/chatfilter"),
        Some(vec!["The filter is on.".into(), "Usage: /chatfilter on|off".into()])
    );
    assert_eq!(h.command(player("m1"), "/chatfilter off"), Some(vec!["Chat filter off.".into()]));
    let ev = h.fire(Event::Chat { player: "p1".into(), text: "darn spam.example".into(), cancelled: false });
    assert!(!ev.cancelled());
    assert!(matches!(ev, Event::Chat { text, .. } if text == "darn spam.example"));
    // The setting survives in storage across a save.
    h.world_save(SavePhase::Pre);
    h.world_save(SavePhase::Post);
    assert_eq!(h.command(CommandSender::Console, "/chatfilter on"), Some(vec!["Chat filter on.".into()]));
}

#[test]
fn scheduler_lua() {
    let (_t, b, mut h) = host_with(&["scheduler-lua"], "[scheduler-lua]\ngrant = [\"players:chat\"]\n");
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    b.join("p1", "CAROL", Role::Player);

    assert_eq!(h.command(player("p1"), "/countdown 2"), Some(vec![]));
    assert_eq!(h.command(player("p1"), "/countdown"), Some(vec!["A countdown is already running.".into()]));
    for _ in 0..15 {
        h.tick(0.2);
    }
    assert_eq!(b.take_broadcasts(), vec!["2", "1", "Go!"]);
    for _ in 0..10 {
        h.tick(0.2);
    }
    assert!(b.take_broadcasts().is_empty());

    assert_eq!(h.command(player("p1"), "/remind 1 drink water"), Some(vec![]));
    assert!(b.take_chats().is_empty());
    h.poll();
    assert_eq!(b.take_chats(), vec![("p1".into(), "OK, I will remind you in 1 s.".into())]);
    for _ in 0..5 {
        h.tick(0.2);
    }
    assert_eq!(b.take_chats(), vec![("p1".into(), "Reminder: drink water".into())]);
    // A fractional countdown still ends, and a negative or huge delay is clamped instead of faulting the mod.
    assert_eq!(h.command(player("p1"), "/countdown 1.5"), Some(vec![]));
    for _ in 0..15 {
        h.tick(0.2);
    }
    assert_eq!(b.take_broadcasts(), vec!["1", "Go!"]);
    assert_eq!(h.command(player("p1"), "/remind -5 now"), Some(vec![]));
    assert_eq!(h.command(player("p1"), "/remind 1e12 never"), Some(vec![]));
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    h.poll();
    b.take_chats();
    assert_eq!(h.command(player("p1"), "/remind soon"), Some(vec!["Usage: /remind <seconds> <text>".into()]));
    assert_eq!(h.command(CommandSender::Console, "/remind 1 x"), Some(vec!["Usage: /remind <seconds> <text>".into()]));
    // A reminder for a player who left is dropped.
    h.command(player("p1"), "/remind 0.2 bye");
    b.leave("p1");
    h.poll();
    h.tick(0.2);
    assert!(b.take_chats().is_empty());
    // The 5 minute tip.
    for _ in 0..1500 {
        h.tick(0.2);
    }
    assert_eq!(b.take_broadcasts(), vec!["Tip: type /remind 30 drink water"]);
}

#[test]
fn missing_grant_keeps_mod_off() {
    let (_t, b, h) = host_with(&["hello-lua", "scheduler-lua"], "");
    assert!(h.is_empty());
    assert!(b.has_log("permissions not granted in mods.toml: players:chat"), "{}", b.log_text());
}

#[test]
fn economy_landclaim_perms() {
    use bhmod_core::types::{BreakLayer, ObjectKind, Pos};
    let (_t, b, mut h) = host_with(&["economy", "landclaim", "perms"], "");
    assert!(b.has_log("loaded 3 mod(s): economy 1.0.0, perms 1.0.0, landclaim 1.0.0"), "{}", b.log_text());
    h.enable("World", "wid");
    for m in h.mods() {
        assert_eq!(m.state, ModState::Enabled, "{}", b.log_text());
    }
    b.join("a", "ALICE", Role::Player);
    b.join("b", "BOB", Role::Player);
    b.join("c", "CAROL", Role::Admin);
    let at = |id: &str, x, y| {
        b.positions.borrow_mut().retain(|p| p.0 != id);
        b.positions.borrow_mut().push((id.into(), Pos { x, y }));
    };
    let cmd = |h: &mut ModHost, who: &str, line: &str| h.command(player(who), line).unwrap().join(" | ");

    // Economy.
    assert_eq!(cmd(&mut h, "a", "/balance"), "ALICE has 100 coins.");
    assert_eq!(cmd(&mut h, "a", "/pay BOB 30"), "Paid 30 coins to BOB. You have 70 left.");
    assert_eq!(b.take_chats(), vec![("b".to_string(), "ALICE paid you 30 coins.".to_string())]);
    assert_eq!(cmd(&mut h, "a", "/bal bob"), "BOB has 130 coins.");
    assert_eq!(cmd(&mut h, "a", "/bal nobody"), "No such player online.");
    assert_eq!(cmd(&mut h, "a", "/pay BOB 1000"), "Payment failed: insufficient funds");
    assert_eq!(cmd(&mut h, "a", "/pay ALICE 1"), "You cannot pay yourself.");
    assert!(cmd(&mut h, "a", "/pay BOB -1").contains("Usage: /pay <player> <amount>"));
    assert_eq!(cmd(&mut h, "a", "/eco-give ALICE 5"), "You do not have permission to use /eco-give.");
    assert_eq!(cmd(&mut h, "c", "/eco-give ALICE 5"), "ALICE now has 75 coins.");
    assert!(cmd(&mut h, "c", "/eco-give ALICE x").contains("Usage"));

    // Land claim: Alice buys the plot around (20, 40) = plot (1, 2).
    assert_eq!(cmd(&mut h, "a", "/claim"), "You need to be in the world.");
    at("a", 20, 40);
    assert_eq!(cmd(&mut h, "a", "/claiminfo"), "Nobody owns this plot.");
    assert_eq!(cmd(&mut h, "a", "/claim"), "You bought this plot for 50 coins.");
    assert_eq!(cmd(&mut h, "a", "/claim"), "You already own this plot.");
    assert_eq!(cmd(&mut h, "a", "/balance"), "ALICE has 25 coins.");
    at("b", 31, 47);
    assert_eq!(cmd(&mut h, "b", "/claim"), "This plot belongs to ALICE.");
    assert_eq!(cmd(&mut h, "b", "/claiminfo"), "Owned by ALICE.");
    assert_eq!(cmd(&mut h, "b", "/unclaim"), "You do not own this plot.");
    let brk = |who: &str, x, y| Event::BlockBreak {
        player: who.into(),
        pos: Pos { x, y },
        layer: BreakLayer::Block,
        tile: 1,
        cancelled: false,
    };
    assert!(h.fire(brk("b", 16, 32)).cancelled(), "Bob may not dig in Alice's plot");
    assert_eq!(b.take_chats(), vec![("b".to_string(), "This land belongs to ALICE.".to_string())]);
    assert!(!h.fire(brk("b", 15, 32)).cancelled(), "next plot is free");
    assert!(!h.fire(brk("a", 16, 32)).cancelled(), "the owner may");
    assert!(!h.fire(brk("c", 16, 32)).cancelled(), "admins may");
    let place = Event::BlockPlace {
        player: "b".into(),
        pos: Pos { x: 17, y: 33 },
        item: 1,
        data_a: 0,
        data_b: 0,
        cancelled: false,
    };
    assert!(h.fire(place).cancelled());
    let obj = Event::ObjectPlace {
        player: "b".into(),
        pos: Pos { x: 17, y: 33 },
        object: ObjectKind::Workbench,
        item: 1,
        cancelled: false,
    };
    assert!(h.fire(obj).cancelled());

    // Perms: grant Bob the bypass node (a wildcard) and he may dig.
    assert!(cmd(&mut h, "c", "/perm grant nobody x").contains("Name an online player."));
    assert_eq!(cmd(&mut h, "c", "/perm list BOB"), "BOB: (none)");
    assert_eq!(cmd(&mut h, "c", "/perm grant BOB landclaim.*"), "Granted landclaim.* for BOB.");
    assert_eq!(cmd(&mut h, "c", "/perm grant BOB other"), "Granted other for BOB.");
    assert_eq!(cmd(&mut h, "c", "/perm list BOB"), "BOB: landclaim.*, other");
    assert!(!h.fire(brk("b", 16, 32)).cancelled());
    assert_eq!(cmd(&mut h, "c", "/perm revoke BOB landclaim.*"), "Revoked landclaim.* for BOB.");
    assert!(h.fire(brk("b", 16, 32)).cancelled());
    assert!(cmd(&mut h, "c", "/perm frobnicate BOB").contains("Usage"));

    // Unclaim refunds half.
    assert_eq!(cmd(&mut h, "a", "/unclaim"), "Plot released; 25 coins refunded.");
    assert_eq!(cmd(&mut h, "a", "/balance"), "ALICE has 50 coins.");
    assert!(!h.fire(brk("b", 16, 32)).cancelled());
    // Not enough coins for a plot.
    at("b", 100, 100);
    assert_eq!(cmd(&mut h, "b", "/pay ALICE 130"), "Paid 130 coins to ALICE. You have 0 left.");
    assert_eq!(cmd(&mut h, "b", "/claim"), "A plot costs 50 coins; you have 0.");
    h.world_save(SavePhase::Post);
    h.disable_all();
}

#[test]
fn cosmetics_hats() {
    let (_t, b, mut h) = host_with(&["cosmetics-hats"], "");
    assert_eq!(h.mods().len(), 1, "{}", b.log_text());
    *b.cosmetic_ids.borrow_mut() = vec![
        ("cosmetics-hats:crown".into(), 1),
        ("cosmetics-hats:party".into(), 2),
        ("cosmetics-hats:ember".into(), 3),
    ];
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    b.join("p1", "ALICE", Role::Player);
    let worn = |b: &common::FakeBridge| {
        let mut w = b.worn.borrow().clone();
        w.sort();
        w
    };

    // /hat and /hair equip, replacing what the slot held; the console cannot wear anything.
    assert_eq!(h.command(player("p1"), "/hat crown"), Some(vec!["Your hat is now crown.".to_string()]));
    assert_eq!(h.command(player("p1"), "/hair ember"), Some(vec!["Your hair is now ember.".to_string()]));
    assert_eq!(worn(&b), vec![("p1".into(), "hair".into(), 3), ("p1".into(), "hat".into(), 1)]);
    h.command(player("p1"), "/hat party");
    assert_eq!(worn(&b), vec![("p1".into(), "hair".into(), 3), ("p1".into(), "hat".into(), 2)]);
    assert_eq!(h.command(CommandSender::Console, "/hat crown"), Some(vec!["Only players wear cosmetics.".to_string()]));

    // Help for a missing or unknown word.
    assert_eq!(h.command(player("p1"), "/hat"), Some(vec!["Usage: /hat <crown, party, none>".to_string()]));
    assert_eq!(
        h.command(player("p1"), "/hair purple"),
        Some(vec!["Unknown hair \"purple\". Try: ember, none".to_string()])
    );

    // The choice is remembered: after the player left and came back it is put on again (the server forgot it).
    b.leave("p1");
    b.worn.borrow_mut().clear();
    b.join("p1", "ALICE", Role::Player);
    h.fire(Event::PlayerJoin { player: "p1".into() });
    assert_eq!(worn(&b), vec![("p1".into(), "hair".into(), 3), ("p1".into(), "hat".into(), 2)]);

    // none takes it off and forgets it.
    assert_eq!(h.command(player("p1"), "/hat none"), Some(vec!["Your hat is back to normal.".to_string()]));
    b.worn.borrow_mut().clear();
    h.fire(Event::PlayerJoin { player: "p1".into() });
    assert_eq!(worn(&b), vec![("p1".into(), "hair".into(), 3)]);
    h.disable_all();
}

/// The price check mod end to end without a network: the client half's call goes to the server half through
/// `remote_call_in`, the answer comes back through `remote_result`.
#[test]
fn prices_lua_round_trip() {
    use bhmod_core::remote::RemoteTarget;
    let grants = "[prices-lua]\ngrant = [\"channels\"]\n";
    let load = |side: Side| {
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(&examples().join("prices-lua"), &tmp.path().join("prices-lua"));
        fs::write(tmp.path().join("mods.toml"), grants).unwrap();
        let b = FakeBridge::new();
        let mut h = ModHost::load(HostOptions::new(tmp.path(), side), b.clone());
        h.enable("World", "wid");
        assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
        (tmp, b, h)
    };
    let (_s, sb, mut server) = load(Side::Server);
    let (_c, cb, mut client) = load(Side::Client);
    let p1 = RemoteTarget::Player("p1".into());
    for (item, toast) in
        [("apple", "apple costs 3 coins."), ("ghost", "ghost is not for sale."), ("sword", "sword costs 120 coins.")]
    {
        client.command(CommandSender::Console, &format!("/price {item}")).unwrap();
        let call = cb.remote_calls.borrow_mut().remove(0);
        // What the network layer does: the server answers the call, the client gets the result.
        let out = server.remote_call_in(&p1, &call.mod_id, &call.service, &call.version_req, &call.method, &call.args);
        assert!(client.remote_result(&RemoteTarget::Server, call.id, out));
        let ui = cb.take_ui();
        assert!(
            ui.iter().any(|(_, c)| format!("{c:?}").contains(&format!("Toast {{ text: \"{toast}\" }}"))),
            "{toast}: {ui:?}\n{}",
            cb.log_text()
        );
    }
    // `update` and anything else is not exposed to clients.
    let args =
        bhmod_core::value::Value::List(vec![bhmod_core::value::Value::text("apple"), bhmod_core::value::Value::Int(1)])
            .encode();
    assert_eq!(
        server.remote_call_in(&p1, "prices-lua", "prices", "^1", "update", &args),
        Err(bhmod_core::RemoteError::NotPermitted)
    );
    let _ = sb;
}

#[test]
fn decor_items() {
    let (_t, b, mut h) = host_with(&["decor-items"], "");
    assert_eq!(h.mods().len(), 1, "{}", b.log_text());
    *b.item_ids.borrow_mut() = vec![("decor-items:brass-lamp".into(), 1107), ("decor-items:wall-plaque".into(), 1108)];
    *b.block_ids.borrow_mut() = vec![("decor-items:warm-brick".into(), 1)];
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    b.join("p1", "ALICE", Role::Player);

    assert_eq!(h.command(player("p1"), "/decor"), Some(vec!["You got 1 x lamp.".to_string()]));
    assert_eq!(h.command(player("p1"), "/decor plaque 500"), Some(vec!["You got 64 x plaque.".to_string()]));
    assert_eq!(h.command(player("p1"), "/decor lamp 3"), Some(vec!["You got 3 x lamp.".to_string()]));
    assert_eq!(
        *b.given.borrow(),
        [("p1".to_string(), 1107, 1), ("p1".to_string(), 1108, 64), ("p1".to_string(), 1107, 3)]
    );
    assert_eq!(
        h.command(player("p1"), "/decor sofa"),
        Some(vec!["Unknown item \"sofa\". Try: lamp, plaque".to_string()])
    );
    assert_eq!(h.command(CommandSender::Console, "/decor"), Some(vec!["Only players carry items.".to_string()]));

    // /warm needs a position, then replaces the block under the player.
    assert_eq!(h.command(player("p1"), "/warm"), Some(vec!["Stand somewhere first.".to_string()]));
    assert_eq!(h.command(CommandSender::Console, "/warm"), Some(vec!["Stand somewhere first.".to_string()]));
    b.positions.borrow_mut().push(("p1".into(), bhmod_core::types::Pos { x: 40, y: 80 }));
    assert_eq!(h.command(player("p1"), "/warm"), Some(vec!["The ground under you is a warm brick now.".to_string()]));
    assert_eq!(*b.placed.borrow(), [(40, 79, 1)]);
    h.disable_all();
}
