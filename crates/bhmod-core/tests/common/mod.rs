//! Test doubles shared by the integration tests.
#![allow(dead_code)]

pub mod conformance;

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bhmod_core::budget::Clock;
use bhmod_core::input::InputCommand;
use bhmod_core::remote::{RemoteCall, RemoteError};
use bhmod_core::types::{ApiError, ChannelTarget, ContentKind, LogLevel, Pos, Role, Side};
use bhmod_core::ui::UiCommand;
use bhmod_core::voice::{Activation, Caps, Icons, Mode, Route, TransportConfig, VoiceHost, VoiceState};
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
    /// The items registry: (key, id).
    pub item_ids: RefCell<Vec<(String, u64)>>,
    /// The variant blocks registry: (key, id).
    pub block_ids: RefCell<Vec<(String, u64)>>,
    /// `give_item` calls: (player, item id, count).
    pub given: RefCell<Vec<(String, u64, u32)>>,
    /// Variant blocks in the world: (x, y, id).
    pub placed: RefCell<Vec<(i32, i32, u64)>>,
    /// Worn cosmetics: (player, slot, id).
    pub worn: RefCell<Vec<(String, String, u64)>>,
    /// Makes `online_players` panic (a host bug that must not cross into the JVM).
    pub explode: Cell<bool>,
    /// The UI commands the bridge drew: (mod id, command).
    pub ui: RefCell<Vec<(String, UiCommand)>>,
    /// True: the bridge has no UI (the server), `ui` answers `unsupported`.
    pub no_ui: Cell<bool>,
    /// The remote service calls the bridge was asked to send.
    pub remote_calls: RefCell<Vec<RemoteCall>>,
    /// When set, `service_send` fails with this error (and sends nothing).
    pub remote_fail: RefCell<Option<RemoteError>>,
    /// The voice host (API 0.3.4), when `has_voice`.
    pub voice: RecVoice,
    pub has_voice: Cell<bool>,
    /// Solid tiles and the world width (API 0.3.4 `terrain`).
    pub solid: RefCell<Vec<(i32, i32)>>,
    pub width: Cell<Option<i32>>,
    /// Key binding commands (API 0.3.4 `input`), and the player's rebinds: (mod, action, key).
    pub input: RefCell<Vec<(String, InputCommand)>>,
    pub rebinds: RefCell<Vec<(String, String, Option<String>)>>,
    /// The side the bridge reports (the server unless set).
    pub client: Cell<bool>,
}

/// A voice host that records every call as text and answers with fixed values.
#[derive(Default)]
pub struct RecVoice {
    pub calls: RefCell<Vec<String>>,
}

impl RecVoice {
    fn rec(&self, s: String) {
        self.calls.borrow_mut().push(s);
    }
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.calls.borrow_mut())
    }
}

impl VoiceHost for RecVoice {
    fn state(&self, p: &str) -> Result<VoiceState, ApiError> {
        self.rec(format!("state {p}"));
        Ok(VoiceState::SelfMuted)
    }
    fn icons(&self) -> Result<Icons, ApiError> {
        Ok(Icons { no_voice: false, ..Icons::default() })
    }
    fn set_routes(&self, s: &str, r: &[Route]) -> Result<(), ApiError> {
        self.rec(format!("routes {s} {r:?}"));
        Ok(())
    }
    fn mute(&self, p: &str, u: Option<u64>) -> Result<(), ApiError> {
        self.rec(format!("mute {p} {u:?}"));
        Ok(())
    }
    fn unmute(&self, p: &str) -> Result<bool, ApiError> {
        self.rec(format!("unmute {p}"));
        Ok(true)
    }
    fn set_speaker_gain(&self, p: &str, g: u16) -> Result<(), ApiError> {
        self.rec(format!("gain {p} {g}"));
        Ok(())
    }
    fn set_caps(&self, c: Caps) -> Result<(), ApiError> {
        self.rec(format!("caps {c:?}"));
        Ok(())
    }
    fn set_transport(&self, t: TransportConfig) -> Result<(), ApiError> {
        self.rec(format!("transport {t:?}"));
        Ok(())
    }
    fn set_allowed(&self, p: &str, m: u8, a: u8) -> Result<(), ApiError> {
        self.rec(format!("allowed {p} {m} {a}"));
        Ok(())
    }
    fn set_icons(&self, i: Icons) -> Result<(), ApiError> {
        self.rec(format!("icons {i:?}"));
        Ok(())
    }
    fn set_broadcast_overrides_deafen(&self, on: bool) -> Result<(), ApiError> {
        self.rec(format!("override {on}"));
        Ok(())
    }
    fn set_default_mode(&self, m: Mode) -> Result<(), ApiError> {
        self.rec(format!("default {}", m.name()));
        Ok(())
    }
    fn set_activation(&self, a: Activation) -> Result<(), ApiError> {
        self.rec(format!("activation {}", a.name()));
        Ok(())
    }
    fn talk(&self, m: Option<Mode>, pressed: bool) -> Result<(), ApiError> {
        self.rec(format!("talk {:?} {pressed}", m.map(Mode::name)));
        Ok(())
    }
    fn set_mic(&self, on: bool) -> Result<(), ApiError> {
        self.rec(format!("mic {on}"));
        Ok(())
    }
    fn set_self_mute(&self, on: bool) -> Result<(), ApiError> {
        self.rec(format!("self-mute {on}"));
        Ok(())
    }
    fn set_deafen(&self, on: bool) -> Result<(), ApiError> {
        self.rec(format!("deafen {on}"));
        Ok(())
    }
    fn set_vad_threshold(&self, db: f32) -> Result<(), ApiError> {
        self.rec(format!("vad {db}"));
        Ok(())
    }
    fn set_master(&self, p: u16) -> Result<(), ApiError> {
        self.rec(format!("master {p}"));
        Ok(())
    }
    fn master(&self) -> Result<u16, ApiError> {
        Ok(75)
    }
    fn set_player_volume(&self, p: &str, v: u16) -> Result<(), ApiError> {
        self.rec(format!("volume {p} {v}"));
        Ok(())
    }
    fn player_volume(&self, _p: &str) -> Result<u16, ApiError> {
        Ok(60)
    }
    fn mute_player(&self, p: &str, m: bool) -> Result<(), ApiError> {
        self.rec(format!("mute-player {p} {m}"));
        Ok(())
    }
    fn player_muted(&self, _p: &str) -> Result<bool, ApiError> {
        Ok(true)
    }
}

impl FakeBridge {
    fn ids(&self, kind: ContentKind) -> &RefCell<Vec<(String, u64)>> {
        match kind {
            ContentKind::Cosmetic => &self.cosmetic_ids,
            ContentKind::Item => &self.item_ids,
            ContentKind::Block => &self.block_ids,
        }
    }
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
        if self.client.get() {
            Side::Client
        } else {
            Side::Server
        }
    }
    fn voice(&self) -> Option<&dyn VoiceHost> {
        self.has_voice.get().then_some(&self.voice as &dyn VoiceHost)
    }
    fn tile_solid(&self, x: i32, y: i32) -> Option<bool> {
        Some(self.solid.borrow().contains(&(x, y)))
    }
    fn world_width(&self) -> Option<i32> {
        self.width.get()
    }
    fn input(&self, mod_id: &str, command: &InputCommand) -> Result<(), ApiError> {
        if self.no_ui.get() {
            return Err(ApiError::Unsupported);
        }
        self.input.borrow_mut().push((mod_id.into(), command.clone()));
        Ok(())
    }
    fn input_key(&self, mod_id: &str, action: &str, default_key: &str) -> Option<String> {
        let r = self.rebinds.borrow();
        match r.iter().find(|(m, a, _)| m == mod_id && a == action) {
            Some((_, _, k)) => k.clone(),
            None => Some(default_key.to_string()),
        }
    }
    fn host_name(&self) -> String {
        "fake-host 1.0".into()
    }
    fn service_send(&self, call: &RemoteCall) -> Result<(), RemoteError> {
        if let Some(e) = self.remote_fail.borrow().clone() {
            return Err(e);
        }
        self.remote_calls.borrow_mut().push(call.clone());
        Ok(())
    }
    fn log(&self, mod_id: Option<&str>, level: LogLevel, message: &str) {
        self.logs.borrow_mut().push((mod_id.map(str::to_string), level, message.to_string()));
    }
    fn online_players(&self) -> Vec<String> {
        assert!(!self.explode.get(), "the fake game exploded");
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
        self.ids(kind).borrow().iter().find(|c| c.0 == key).map(|c| c.1)
    }
    fn content_key(&self, kind: ContentKind, id: u64) -> Option<String> {
        self.ids(kind).borrow().iter().find(|c| c.1 == id).map(|c| c.0.clone())
    }
    fn give_item(&self, player: &str, item: u64, count: u32) -> Result<(), ApiError> {
        self.given.borrow_mut().push((player.into(), item, count));
        Ok(())
    }
    fn place_block(&self, x: i32, y: i32, block: u64) -> Result<(), ApiError> {
        self.placed.borrow_mut().push((x, y, block));
        Ok(())
    }
    fn block_at(&self, x: i32, y: i32) -> Option<u64> {
        self.placed.borrow().iter().rev().find(|p| p.0 == x && p.1 == y).map(|p| p.2)
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
    /// A clock an hour ahead of the real one, for tests that run Luau under it: guest deadlines are compared with the
    /// real clock, so a manual clock that stands still must not be behind it.
    pub fn ahead() -> FakeClock {
        FakeClock {
            now: Rc::new(Cell::new(Instant::now() + Duration::from_secs(3600))),
            auto: Rc::new(Cell::new(Duration::ZERO)),
        }
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
