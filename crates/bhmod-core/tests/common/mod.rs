//! Test doubles shared by the integration tests.
#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bhmod_core::budget::Clock;
use bhmod_core::types::{LogLevel, Role, Side};
use bhmod_core::GameBridge;

#[derive(Default)]
pub struct FakeBridge {
    pub players: RefCell<Vec<(String, String, Role)>>,
    pub logs: RefCell<Vec<(Option<String>, LogLevel, String)>>,
    pub chats: RefCell<Vec<(String, String)>>,
    pub broadcasts: RefCell<Vec<String>>,
    pub kicks: RefCell<Vec<(String, String)>>,
}

impl FakeBridge {
    pub fn new() -> Rc<FakeBridge> {
        Rc::new(FakeBridge::default())
    }
    pub fn join(&self, id: &str, alias: &str, role: Role) {
        self.players.borrow_mut().push((id.into(), alias.into(), role));
    }
    pub fn leave(&self, id: &str) {
        self.players.borrow_mut().retain(|p| p.0 != id);
    }
    pub fn take_chats(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.chats.borrow_mut())
    }
    pub fn take_broadcasts(&self) -> Vec<String> {
        std::mem::take(&mut self.broadcasts.borrow_mut())
    }
    pub fn log_text(&self) -> String {
        self.logs
            .borrow()
            .iter()
            .map(|(m, l, s)| format!("[{}] {}: {}\n", l.name(), m.as_deref().unwrap_or("-"), s))
            .collect()
    }
    pub fn has_log(&self, needle: &str) -> bool {
        self.logs.borrow().iter().any(|(_, _, s)| s.contains(needle))
    }
}

impl GameBridge for FakeBridge {
    fn side(&self) -> Side {
        Side::Server
    }
    fn host_name(&self) -> String {
        "fake-host 1.0".into()
    }
    fn log(&self, mod_id: Option<&str>, level: LogLevel, message: &str) {
        self.logs.borrow_mut().push((mod_id.map(str::to_string), level, message.to_string()));
    }
    fn online_players(&self) -> Vec<String> {
        self.players.borrow().iter().map(|p| p.0.clone()).collect()
    }
    fn player_alias(&self, id: &str) -> Option<String> {
        self.players.borrow().iter().find(|p| p.0 == id).map(|p| p.1.clone())
    }
    fn player_role(&self, id: &str) -> Role {
        self.players.borrow().iter().find(|p| p.0 == id).map_or(Role::Player, |p| p.2)
    }
    fn send_chat(&self, id: &str, text: &str) -> bool {
        if self.player_alias(id).is_none() {
            return false;
        }
        self.chats.borrow_mut().push((id.into(), text.into()));
        true
    }
    fn broadcast(&self, text: &str) {
        self.broadcasts.borrow_mut().push(text.into());
    }
    fn kick(&self, id: &str, reason: &str) -> bool {
        if self.player_alias(id).is_none() {
            return false;
        }
        self.kicks.borrow_mut().push((id.into(), reason.into()));
        true
    }
    fn is_builtin_command(&self, name: &str) -> bool {
        name == "help" || name == "kick"
    }
}

/// A clock that only moves when told to (or by `auto` on every reading).
#[derive(Clone)]
pub struct FakeClock {
    pub now: Rc<Cell<Instant>>,
    pub auto: Rc<Cell<Duration>>,
}

impl FakeClock {
    pub fn new() -> FakeClock {
        FakeClock { now: Rc::new(Cell::new(Instant::now())), auto: Rc::new(Cell::new(Duration::ZERO)) }
    }
    pub fn advance(&self, d: Duration) {
        self.now.set(self.now.get() + d);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        let n = self.now.get();
        self.now.set(n + self.auto.get());
        n
    }
}

pub const LUA_TOML: &str = "schema = 1\nid = \"ID\"\nversion = \"1.0.0\"\nname = { en = \"N\" }\nauthors = [\"a\"]\nlicense = \"MIT\"\napi = \"^0.1\"\n[sides]\nserver = true\nclient = false\n[entry]\nserver = { runtime = \"lua\", path = \"server/main.luau\" }\n";

/// Writes an unpacked Luau mod `id` into `mods` with `main.luau` and extra manifest text.
pub fn lua_mod(mods: &Path, id: &str, extra: &str, main: &str) {
    let d = mods.join(id);
    fs::create_dir_all(d.join("server")).unwrap();
    fs::write(d.join("mod.toml"), format!("{}{extra}", LUA_TOML.replace("ID", id))).unwrap();
    fs::write(d.join("server/main.luau"), main).unwrap();
}

/// Copies a directory tree.
pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// The SDK examples directory.
pub fn examples() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}
