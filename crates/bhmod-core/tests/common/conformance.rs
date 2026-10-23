//! The conformance suite shared by every runtime: the same scenarios run against the Luau twin and another
//! runtime's twin of each example mod through the real mod host and a fake game. Both runs must produce the same
//! transcript (command replies, event results, chat, broadcasts, kicks and log lines with the mod id masked).
#![allow(dead_code)]

use std::fs;
use std::rc::Rc;

use bhmod_core::runtime::Runtime;
use bhmod_core::types::{CommandSender, Event, Role, SavePhase, Side};
use bhmod_core::{HostOptions, ModHost, ModState};

use super::{copy_dir, examples, FakeBridge};

/// Makes the runtime of the non-Luau twin (`None` for a Luau mod: the host already has Luau).
pub type MakeRuntime = fn() -> Option<Box<dyn Runtime>>;

/// One example mod running in a fresh host.
pub struct Run {
    pub _tmp: tempfile::TempDir,
    pub b: Rc<FakeBridge>,
    pub h: ModHost,
    pub id: String,
    pub out: Vec<String>,
}

impl Run {
    pub fn new(example: &str, grants: &[&str], make: MakeRuntime) -> Run {
        let dir = examples().join(example);
        assert!(
            dir.join("server").read_dir().unwrap().next().is_some(),
            "{example} has no server code (run scripts/build-wasm.sh or scripts/build-java.sh)"
        );
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(&dir, &tmp.path().join(example));
        let grants: Vec<String> = grants.iter().map(|g| format!("\"{g}\"")).collect();
        fs::write(tmp.path().join("mods.toml"), format!("[{example}]\ngrant = [{}]\n", grants.join(", "))).unwrap();
        let b = FakeBridge::new();
        let mut opts = HostOptions::new(tmp.path(), Side::Server);
        opts.runtimes.extend(make());
        // Behaviour, not speed: emulated 32-bit runs (qemu) and coverage builds are slow.
        opts.budgets.hard = std::time::Duration::from_secs(5);
        opts.budgets.soft = opts.budgets.hard;
        opts.budgets.step = opts.budgets.hard;
        let h = ModHost::load(opts, b.clone());
        Run { _tmp: tmp, b, h, id: example.to_string(), out: Vec::new() }
    }

    pub fn note(&mut self, what: &str, v: impl std::fmt::Debug) {
        self.out.push(format!("{what}: {v:?}"));
    }

    pub fn command(&mut self, sender: CommandSender, line: &str) {
        let r = self.h.command(sender, line);
        self.note(line, r);
    }

    pub fn chat(&mut self, player: &str, text: &str) {
        let ev = self.h.fire(Event::Chat { player: player.into(), text: text.into(), cancelled: false });
        self.note("chat", ev);
    }

    /// Records and clears what the game saw since the last call.
    pub fn flush(&mut self) {
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

    pub fn state(&self) -> ModState {
        self.h.mods()[0].state.clone()
    }
}

pub fn player(id: &str) -> CommandSender {
    CommandSender::Player(id.into())
}

/// Runs `scenario` on both twins and compares the transcripts; returns one of them.
pub fn conform(lua: &str, other: &str, grants: &[&str], make: MakeRuntime, scenario: fn(&mut Run)) -> Vec<String> {
    let mut runs = [Run::new(lua, grants, || None), Run::new(other, grants, make)];
    for r in &mut runs {
        scenario(r);
        r.flush();
    }
    let [a, b] = runs;
    assert_eq!(a.out.len(), b.out.len(), "{lua} vs {other}:\n{:#?}\n{:#?}", a.out, b.out);
    for (x, y) in a.out.iter().zip(&b.out) {
        assert_eq!(x, y, "{lua} vs {other}");
    }
    a.out
}

pub fn has(out: &[String], needle: &str) -> bool {
    out.iter().any(|l| l.contains(needle))
}

pub fn hello(r: &mut Run) {
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

pub fn chat_filter(r: &mut Run) {
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

pub fn ticks(r: &mut Run, n: usize) {
    for _ in 0..n {
        r.h.tick(0.2);
    }
}

pub fn scheduler(r: &mut Run) {
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
