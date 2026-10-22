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
