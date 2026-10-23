//! The conformance suite: the same scenarios run against the Luau and the WASM (Rust) twin of each example mod
//! (`hello`, `chat-filter`, `scheduler`) through the real mod host and a fake game. Both runs must produce the same
//! transcript (command replies, event results, chat, broadcasts, kicks and log lines with the mod id masked), and
//! the transcript must contain the expected key results.
//!
//! The WASM examples are built by `scripts/build-wasm.sh`.

#[path = "../../bhmod-core/tests/common/mod.rs"]
mod common;

use bhmod_core::runtime::Runtime;
use bhmod_wasm::{WasmConfig, WasmRuntime};
use common::conformance::*;

fn wasm() -> Option<Box<dyn Runtime>> {
    Some(Box::new(WasmRuntime::new(WasmConfig::default()).unwrap()))
}

#[test]
fn hello_twins() {
    let out = conform("hello-lua", "hello-rust", &["players:chat"], wasm, hello);
    assert!(has(&out, "state: Enabled"));
    assert!(has(&out, "Hello, ALICE! You are greeting number 2."));
    assert!(has(&out, "/hello: Some([\"Hello, ALICE!\"])"));
    assert!(has(&out, "\"console says hello to everyone!\""));
    assert!(has(&out, "text: \"Lua is BAD\", cancelled: true"));
    assert!(has(&out, "[info] true: <mod> disabled"));
}

#[test]
fn hello_twins_need_the_grant() {
    let out = conform("hello-lua", "hello-rust", &[], wasm, |r| r.note("loaded", r.h.mods().len()));
    assert!(has(&out, "loaded: 0"));
    assert!(has(&out, "permissions not granted in mods.toml: players:chat"));
}

#[test]
fn chat_filter_twins() {
    let out = conform("chat-filter", "chat-filter-rust", &[], wasm, chat_filter);
    assert!(has(&out, "Your message was blocked: links to spam.example are not allowed."));
    assert!(has(&out, "blocked a message from BOB"));
    assert!(has(&out, "text: \"****, what the ****\", cancelled: false"));
    assert!(has(&out, "You do not have permission to use /chatfilter."));
    assert!(has(&out, "[\"The filter is on.\", \"Usage: /chatfilter on|off\"]"));
    assert!(has(&out, "text: \"darn spam.example\", cancelled: false"));
    assert!(has(&out, "text: \"****\", cancelled: false"));
}

#[test]
fn scheduler_twins() {
    let out = conform("scheduler-lua", "scheduler-rust", &["players:chat"], wasm, scheduler);
    assert!(has(&out, "state: Enabled"));
    assert!(has(&out, "/countdown: Some([\"A countdown is already running.\"])"));
    assert!(has(&out, "broadcasts: [\"2\", \"1\", \"Go!\"]"));
    assert!(has(&out, "\"10\", \"9\""));
    assert!(has(&out, "OK, I will remind you in 1 s."));
    assert!(has(&out, "Reminder: drink water"));
    assert!(has(&out, "Usage: /remind <seconds> <text>"));
    assert!(has(&out, "Tip: type /remind 30 drink water"));
}
