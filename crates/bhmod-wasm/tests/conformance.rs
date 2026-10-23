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
    fn with_side(example: &str, grants: &[&str], side: Side) -> Run {
        let dir = examples().join(example);
        let code = if side == Side::Client { "client" } else { "server" };
        assert!(
            dir.join(code).read_dir().unwrap().next().is_some(),
            "{example} has no {code} code (run scripts/build-wasm.sh)"
        );
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(&dir, &tmp.path().join(example));
        let grants: Vec<String> = grants.iter().map(|g| format!("\"{g}\"")).collect();
        fs::write(tmp.path().join("mods.toml"), format!("[{example}]\ngrant = [{}]\n", grants.join(", "))).unwrap();
        let b = FakeBridge::new();
        let mut opts = HostOptions::new(tmp.path(), side);
        opts.runtimes.push(Box::new(WasmRuntime::new(WasmConfig::default()).unwrap()));
        // Behaviour, not speed: emulated 32-bit runs (qemu) and coverage builds are slow.
        opts.budgets.hard = std::time::Duration::from_secs(5);
        opts.budgets.soft = opts.budgets.hard;
        opts.budgets.step = opts.budgets.hard;
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
        let ui: Vec<String> = self.b.take_ui().into_iter().map(|(m, c)| format!("{}: {c:?}", m == self.id)).collect();
        self.note("ui", ui);
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
    conform_on(lua, wasm, grants, Side::Server, scenario)
}

fn conform_on(lua: &str, wasm: &str, grants: &[&str], side: Side, scenario: fn(&mut Run)) -> Vec<String> {
    let mut runs = [Run::with_side(lua, grants, side), Run::with_side(wasm, grants, side)];
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

/// Like [`has`], for transcript lines that hold Debug text of strings (quotes escaped twice).
fn has_ui(out: &[String], needle: &str) -> bool {
    out.iter().any(|l| l.replace("\\\"", "\"").contains(needle))
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

fn hud(r: &mut Run) {
    r.h.enable("World", "wid");
    r.note("state", r.state());
    r.flush();
    // The tick handler updates the position text every fifth step.
    ticks(r, 10);
    r.flush();
    // The player answers the dialog: send 25 coins.
    let v = r.h.ui_result(1, 1, Some("25".into()));
    r.note("answer", v);
    r.flush();
    // A second answer to the same dialog is refused; the loader never saw one.
    let v = r.h.ui_result(1, 0, None);
    r.note("again", v);
    r.flush();
    r.h.disable_all();
    r.note("state", r.state());
    r.flush();
}

fn hud_cancel(r: &mut Run) {
    r.h.enable("World", "wid");
    r.flush();
    let v = r.h.ui_result(1, 0, None);
    r.note("answer", v);
    let v = r.h.ui_result(2, bhmod_core::ui::CLOSED, None);
    r.note("closed", v);
    r.flush();
}

#[test]
fn hud_twins() {
    let out = conform_on("hud-lua", "hud-rust", &[], Side::Client, hud);
    assert!(has_ui(&out, "state: Enabled"));
    assert!(has_ui(&out, "SetWidget { id: \"coins\", widget: Widget { slot: TopRight, kind: Counter, icon: Some(\"coin\"), title: None, text: \"1250\""));
    assert!(has_ui(&out, "Notice { text: \"HUD demo is on.\", level: Success }"));
    assert!(has_ui(&out, "ShowDialog { id: 1"));
    assert!(has_ui(&out, "text: \"tick 5\"") && has_ui(&out, "text: \"tick 10\""));
    assert!(has_ui(&out, "answer: true"));
    assert!(has_ui(&out, "text: \"1225\""), "the counter follows the answer: {out:#?}");
    assert!(has_ui(&out, "Toast { text: \"You sent 25 coins.\" }"));
    assert!(has_ui(&out, "CloseDialog { id: 2 }"));
    assert!(has_ui(&out, "again: false"));
    assert!(has_ui(&out, "ClearMod"));
}

#[test]
fn hud_twins_cancelled() {
    let out = conform_on("hud-lua", "hud-rust", &[], Side::Client, hud_cancel);
    assert!(has_ui(&out, "Toast { text: \"Nothing was sent.\" }"));
    assert!(has_ui(&out, "RemoveWidget { id: \"where\" }"));
    assert!(has_ui(&out, "closed: false"));
}
