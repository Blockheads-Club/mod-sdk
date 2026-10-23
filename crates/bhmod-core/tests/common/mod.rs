//! Test doubles shared by the integration tests.
#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bhmod_core::budget::Clock;
use bhmod_core::types::{ApiError, ChannelTarget, ContentKind, LogLevel, Pos, Role, Side};
use bhmod_core::ui::UiCommand;
use bhmod_core::GameBridge;

/// A channel message the bridge was asked to send: (mod, channel, target, data).
pub type Sent = (String, String, ChannelTarget, Vec<u8>);

#[derive(Default)]
pub struct FakeBridge {
    pub players: RefCell<Vec<(String, String, Role)>>,
    pub logs: RefCell<Vec<(Option<String>, LogLevel, String)>>,
    pub chats: RefCell<Vec<(String, String)>>,
    pub broadcasts: RefCell<Vec<String>>,
    pub kicks: RefCell<Vec<(String, String)>>,
    pub positions: RefCell<Vec<(String, Pos)>>,
    pub sent: RefCell<Vec<Sent>>,
    /// The cosmetics registry: (key, id).
    pub cosmetic_ids: RefCell<Vec<(String, u64)>>,
    /// Worn cosmetics: (player, slot, id).
    pub worn: RefCell<Vec<(String, String, u64)>>,
    /// The UI commands the bridge drew: (mod id, command).
    pub ui: RefCell<Vec<(String, UiCommand)>>,
    /// True: the bridge has no UI (the server), `ui` answers `unsupported`.
    pub no_ui: Cell<bool>,
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
    pub fn take_ui(&self) -> Vec<(String, UiCommand)> {
        std::mem::take(&mut self.ui.borrow_mut())
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
    fn player_pos(&self, id: &str) -> Option<Pos> {
        self.positions.borrow().iter().find(|p| p.0 == id).map(|p| p.1)
    }
    fn content_id(&self, kind: ContentKind, key: &str) -> Option<u64> {
        if kind != ContentKind::Cosmetic {
            return None;
        }
        self.cosmetic_ids.borrow().iter().find(|c| c.0 == key).map(|c| c.1)
    }
    fn content_key(&self, kind: ContentKind, id: u64) -> Option<String> {
        if kind != ContentKind::Cosmetic {
            return None;
        }
        self.cosmetic_ids.borrow().iter().find(|c| c.1 == id).map(|c| c.0.clone())
    }
    fn set_cosmetic(&self, player: &str, slot: &str, id: Option<u64>) -> Result<(), ApiError> {
        let mut worn = self.worn.borrow_mut();
        worn.retain(|w| !(w.0 == player && w.1 == slot));
        if let Some(id) = id {
            worn.push((player.into(), slot.into(), id));
        }
        Ok(())
    }
    fn cosmetics_of(&self, player: &str) -> Vec<(String, u64)> {
        self.worn.borrow().iter().filter(|w| w.0 == player).map(|w| (w.1.clone(), w.2)).collect()
    }
    fn ui(&self, mod_id: &str, command: &UiCommand) -> Result<(), ApiError> {
        if self.no_ui.get() {
            return Err(ApiError::Unsupported);
        }
        self.ui.borrow_mut().push((mod_id.into(), command.clone()));
        Ok(())
    }
    fn channel_send(&self, mod_id: &str, name: &str, target: &ChannelTarget, data: &[u8]) -> Result<(), ApiError> {
        if *target == ChannelTarget::Server {
            return Err(ApiError::Unsupported);
        }
        self.sent.borrow_mut().push((mod_id.into(), name.into(), target.clone(), data.to_vec()));
        Ok(())
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
