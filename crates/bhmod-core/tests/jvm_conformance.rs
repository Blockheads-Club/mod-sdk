//! The conformance suite for the JVM runtime: the Java twin of each example behaves exactly like the Luau one
//! (same scenarios and transcript comparison as the WASM suite), and a Java mod calls a Luau service.
//!
//! Needs a JRE at run time and the example jars: run `scripts/build-java.sh` first (CI does).
#![cfg(feature = "jvm")]

mod common;

use bhmod_core::jvm::{JvmConfig, JvmRuntime};
use bhmod_core::runtime::Runtime;
use bhmod_core::types::{CommandSender, Role, Side};
use bhmod_core::{HostOptions, ModHost, ModState};
use common::conformance::*;
use common::*;

fn jvm() -> Option<Box<dyn Runtime>> {
    Some(Box::new(JvmRuntime::new(JvmConfig::default()).unwrap()))
}

const GRANTS: &[&str] = &["native", "players:chat"];

#[test]
fn hello_twins() {
    let out = conform("hello-lua", "hello-java", GRANTS, jvm, hello);
    assert!(has(&out, "state: Enabled"));
    assert!(has(&out, "Hello, ALICE! You are greeting number 2."));
    assert!(has(&out, "/hello: Some([\"Hello, ALICE!\"])"));
    assert!(has(&out, "\"console says hello to everyone!\""));
    assert!(has(&out, "text: \"Lua is BAD\", cancelled: true"));
    assert!(has(&out, "[info] true: <mod> disabled"));
}

#[test]
fn chat_filter_twins() {
    let out = conform("chat-filter", "chat-filter-java", &["native"], jvm, chat_filter);
    assert!(has(&out, "Your message was blocked: links to spam.example are not allowed."));
    assert!(has(&out, "blocked a message from BOB"));
    assert!(has(&out, "text: \"****, what the ****\", cancelled: false"));
    assert!(has(&out, "You do not have permission to use /chatfilter."));
    assert!(has(&out, "[\"The filter is on.\", \"Usage: /chatfilter on|off\"]"));
    assert!(has(&out, "text: \"****\", cancelled: false"));
}

#[test]
fn scheduler_twins() {
    let out = conform("scheduler-lua", "scheduler-java", GRANTS, jvm, scheduler);
    assert!(has(&out, "state: Enabled"));
    assert!(has(&out, "broadcasts: [\"2\", \"1\", \"Go!\"]"));
    assert!(has(&out, "OK, I will remind you in 1 s."));
    assert!(has(&out, "Reminder: drink water"));
    assert!(has(&out, "Usage: /remind <seconds> <text>"));
    assert!(has(&out, "Tip: type /remind 30 drink water"));
}

#[test]
fn scheduler_reload_twins() {
    let out = conform("scheduler-lua", "scheduler-java", GRANTS, jvm, scheduler_reload);
    assert!(has(&out, "reload: [Reloaded(\"0.1.0\", \"0.1.0\")]"), "{out:#?}");
    assert!(has(&out, "broadcasts: [\"2\", \"1\", \"Go!\"]"), "{out:#?}");
}

#[test]
fn a_jvm_mod_needs_the_native_grant() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&examples().join("hello-java"), &tmp.path().join("hello-java"));
    std::fs::write(tmp.path().join("mods.toml"), "[hello-java]\ngrant = [\"players:chat\"]\n").unwrap();
    let b = FakeBridge::new();
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.runtimes.extend(jvm());
    let h = ModHost::load(opts, b.clone());
    assert!(h.mods().is_empty());
    assert!(b.has_log("permissions not granted in mods.toml: native"), "{}", b.log_text());
}

#[test]
fn java_shop_pays_through_the_luau_economy() {
    let tmp = tempfile::tempdir().unwrap();
    for e in ["economy", "shop-java"] {
        copy_dir(&examples().join(e), &tmp.path().join(e));
    }
    std::fs::write(tmp.path().join("mods.toml"), "[shop-java]\ngrant = [\"native\"]\n").unwrap();
    let b = FakeBridge::new();
    b.join("p1", "ALICE", Role::Player);
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.runtimes.extend(jvm());
    let mut h = ModHost::load(opts, b.clone());
    h.enable("World", "wid");
    for m in h.mods() {
        assert_eq!(m.state, ModState::Enabled, "{}", b.log_text());
    }
    let p = || CommandSender::Player("p1".into());
    for _ in 0..3 {
        assert_eq!(h.command(p(), "/buy sword"), Some(vec!["You bought a sword for 30 coins.".to_string()]));
    }
    assert_eq!(h.command(p(), "/buy sword"), Some(vec!["A sword costs 30 coins; you do not have enough.".to_string()]));
    assert_eq!(h.command(p(), "/balance"), Some(vec!["ALICE has 10 coins.".to_string()]));
    assert_eq!(h.command(p(), "/buy"), Some(vec!["Usage: /buy <apple|sword>".to_string()]));
}
