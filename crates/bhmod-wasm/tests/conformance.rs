//! The conformance suite: the same scenarios run against the Luau and the WASM (Rust) twin of each example mod
//! (`hello`, `chat-filter`, `scheduler`) through the real mod host and a fake game. Both runs must produce the same
//! transcript (command replies, event results, chat, broadcasts, kicks and log lines with the mod id masked), and
//! the transcript must contain the expected key results.
//!
//! The WASM examples are built by `scripts/build-wasm.sh`.

#[path = "../../bhmod-core/tests/common/mod.rs"]
mod common;

use std::fs;
use std::rc::Rc;

use bhmod_core::types::{CommandSender, Event, Role, SavePhase, Side};
use bhmod_core::{HostOptions, ModHost, ModState};
use bhmod_wasm::{WasmConfig, WasmRuntime};
use common::{copy_dir, examples, FakeBridge};

/// One example mod running in a fresh host.
struct Run {
    _tmp: tempfile::TempDir,
    b: Rc<FakeBridge>,
    h: ModHost,
    id: String,
    out: Vec<String>,
}

impl Run {
    fn new(example: &str, grants: &[&str]) -> Run {
        let dir = examples().join(example);
        assert!(
            dir.join("server").read_dir().unwrap().next().is_some(),
            "{example} has no server code (run scripts/build-wasm.sh)"
        );
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(&dir, &tmp.path().join(example));
        let grants: Vec<String> = grants.iter().map(|g| format!("\"{g}\"")).collect();
        fs::write(tmp.path().join("mods.toml"), format!("[{example}]\ngrant = [{}]\n", grants.join(", "))).unwrap();
        let b = FakeBridge::new();
        let mut opts = HostOptions::new(tmp.path(), Side::Server);
        opts.runtimes.push(Box::new(WasmRuntime::new(WasmConfig::default()).unwrap()));
        let h = ModHost::load(opts, b.clone());
        Run { _tmp: tmp, b, h, id: example.to_string(), out: Vec::new() }
    }

    fn note(&mut self, what: &str, v: impl std::fmt::Debug) {
        self.out.push(format!("{what}: {v:?}"));
    }

    fn command(&mut self, sender: CommandSender, line: &str) {
        let r = self.h.command(sender, line);
        self.note(line, r);
    }

    fn chat(&mut self, player: &str, text: &str) {
        let ev = self.h.fire(Event::Chat { player: player.into(), text: text.into(), cancelled: false });
        self.note("chat", ev);
    }

    /// Records and clears what the game saw since the last call.
    fn flush(&mut self) {
        let chats = self.b.take_chats();
        let casts = self.b.take_broadcasts();
        let kicks = std::mem::take(&mut *self.b.kicks.borrow_mut());
        let dir = self._tmp.path().display().to_string();
        let logs: Vec<String> = std::mem::take(&mut *self.b.logs.borrow_mut())
            .into_iter()
            .filter(|(_, _, m)| !m.contains("(budget"))
            .map(|(m, l, s)| {
                format!("[{}] {}: {}", l.name(), m.is_some(), s.replace(&dir, "<dir>").replace(&self.id, "<mod>"))
            })
            .collect();
        self.note("chats", chats);
        self.note("broadcasts", casts);
        self.note("kicks", kicks);
        self.note("logs", logs);
    }

    fn state(&self) -> ModState {
        self.h.mods()[0].state.clone()
    }
}

fn player(id: &str) -> CommandSender {
    CommandSender::Player(id.into())
}

/// Runs `scenario` on both twins and compares the transcripts; returns one of them.
fn conform(lua: &str, wasm: &str, grants: &[&str], scenario: fn(&mut Run)) -> Vec<String> {
    let mut runs = [Run::new(lua, grants), Run::new(wasm, grants)];
    for r in &mut runs {
        scenario(r);
        r.flush();
    }
    let [a, b] = runs;
    assert_eq!(a.out.len(), b.out.len(), "{lua} vs {wasm}:\n{:#?}\n{:#?}", a.out, b.out);
    for (x, y) in a.out.iter().zip(&b.out) {
        assert_eq!(x, y, "{lua} vs {wasm}");
    }
    a.out
}

fn has(out: &[String], needle: &str) -> bool {
    out.iter().any(|l| l.contains(needle))
}

fn hello(r: &mut Run) {
    r.note("loaded", r.h.mods().len());
    r.h.enable("World", "wid");
    r.note("state", r.state());
    r.flush();
    r.b.join("p1", "ALICE", Role::Player);
    r.h.fire(Event::PlayerJoin { player: "p1".into() });
    r.h.fire(Event::PlayerJoin { player: "p1".into() });
    r.flush();
    for line in ["/hello", "/HI there", "/help", "hello"] {
        let owns = r.h.owns_command(line);
        r.note(&format!("owns {line}"), owns);
    }
    r.command(player("p1"), "/hello");
    r.command(CommandSender::Console, "/hi\n");
    r.command(player("p1"), "/help");
    r.flush();
    r.chat("p1", "Lua is BAD");
    r.chat("p1", "hi");
    r.h.disable_all();
    r.note("state", r.state());
}

#[test]
fn hello_twins() {
    let out = conform("hello-lua", "hello-rust", &["players:chat"], hello);
    assert!(has(&out, "state: Enabled"));
    assert!(has(&out, "Hello, ALICE! You are greeting number 2."));
    assert!(has(&out, "/hello: Some([\"Hello, ALICE!\"])"));
    assert!(has(&out, "\"console says hello to everyone!\""));
    assert!(has(&out, "text: \"Lua is BAD\", cancelled: true"));
    assert!(has(&out, "[info] true: <mod> disabled"));
}

#[test]
fn hello_twins_need_the_grant() {
    let out = conform("hello-lua", "hello-rust", &[], |r| r.note("loaded", r.h.mods().len()));
    assert!(has(&out, "loaded: 0"));
    assert!(has(&out, "permissions not granted in mods.toml: players:chat"));
}

fn chat_filter(r: &mut Run) {
    r.h.enable("World", "wid");
    r.b.join("p1", "BOB", Role::Player);
    r.b.join("m1", "MOD", Role::Moderator);
    r.chat("p1", "get stuff at SPAM.example now");
    r.chat("p1", "darn, what the heck");
    r.chat("p1", "free-diamonds.example darn");
    r.flush();
    r.command(player("p1"), "/chatfilter off");
    r.command(player("m1"), "/chatfilter");
    r.command(player("m1"), "/chatfilter maybe");
    r.command(player("m1"), "/chatfilter off");
    r.chat("p1", "darn spam.example");
    r.h.world_save(SavePhase::Pre);
    r.h.world_save(SavePhase::Post);
    r.command(CommandSender::Console, "/chatfilter on");
    r.chat("p1", "heck");
}

#[test]
fn chat_filter_twins() {
    let out = conform("chat-filter", "chat-filter-rust", &[], chat_filter);
    assert!(has(&out, "Your message was blocked: links to spam.example are not allowed."));
    assert!(has(&out, "blocked a message from BOB"));
    assert!(has(&out, "text: \"****, what the ****\", cancelled: false"));
    assert!(has(&out, "You do not have permission to use /chatfilter."));
    assert!(has(&out, "[\"The filter is on.\", \"Usage: /chatfilter on|off\"]"));
    assert!(has(&out, "text: \"darn spam.example\", cancelled: false"));
    assert!(has(&out, "text: \"****\", cancelled: false"));
}

fn ticks(r: &mut Run, n: usize) {
    for _ in 0..n {
        r.h.tick(0.2);
    }
}

fn scheduler(r: &mut Run) {
    r.h.enable("World", "wid");
    r.note("state", r.state());
    r.b.join("p1", "CAROL", Role::Player);
    r.command(player("p1"), "/countdown 2");
    r.command(player("p1"), "/countdown");
    ticks(r, 15);
    r.flush();
    ticks(r, 10);
    r.command(player("p1"), "/countdown 99");
    ticks(r, 60);
    r.flush();
    r.command(player("p1"), "/remind 1 drink water");
    r.flush();
    r.h.poll();
    r.flush();
    ticks(r, 5);
    r.flush();
    r.command(player("p1"), "/remind soon");
    r.command(CommandSender::Console, "/remind 1 x");
    r.command(player("p1"), "/remind 0.2 bye");
    r.b.leave("p1");
    r.h.poll();
    ticks(r, 1);
    r.flush();
    ticks(r, 1500);
}

#[test]
fn scheduler_twins() {
    let out = conform("scheduler-lua", "scheduler-rust", &["players:chat"], scheduler);
    assert!(has(&out, "state: Enabled"));
    assert!(has(&out, "/countdown: Some([\"A countdown is already running.\"])"));
    assert!(has(&out, "broadcasts: [\"2\", \"1\", \"Go!\"]"));
    assert!(has(&out, "\"10\", \"9\""));
    assert!(has(&out, "OK, I will remind you in 1 s."));
    assert!(has(&out, "Reminder: drink water"));
    assert!(has(&out, "Usage: /remind <seconds> <text>"));
    assert!(has(&out, "Tip: type /remind 30 drink water"));
}
