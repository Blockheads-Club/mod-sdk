//! The loader core: one [`Session`] per server connection, one [`ModHost`] per world, and the player's installed
//! mods. Everything here runs on the game thread (the hooks call it); the game is reached only through
//! [`Platform`], so the whole flow is testable on a desktop with a fake platform.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use bhmod_client::installed::Installed;
use bhmod_client::session::{Accepted, Output};
use bhmod_client::{Caps, Options, PackageCache, Session};
use bhmod_content::Content;
use bhmod_core::budget::Budgets;
use bhmod_core::host::{HostOptions, ModHost, ModState};
use bhmod_core::runtime::Runtime;
use bhmod_core::storage::FileStorage;
use bhmod_core::types::{ApiError, ChannelSource, ChannelTarget, LogLevel, Role, Side};
use bhmod_core::GameBridge;
use bhmod_proto::{Hash, PACKET_TYPE};

use crate::inbox::NetEvent;
use crate::ui;

/// Mods tick at the server's world step rate.
pub const TICK_MS: u64 = 200;
/// Progress reaches the UI at most this often.
pub const PROGRESS_MS: u64 = 100;

/// What the loader needs from the game.
pub trait Platform {
    /// Milliseconds of a monotonic clock.
    fn now_ms(&self) -> u64;
    /// Writes a log line (logcat).
    fn log(&self, level: LogLevel, msg: &str);
    /// Sends a packet to the server (reliable).
    fn send(&self, packet: &[u8]);
    /// Sends the held 0x1f, with `bhcModLoader` when `Some`.
    fn release_join(&self, mod_loader: Option<Hash>);
    /// Stops joining (the game shows its rejection screen).
    fn abort_join(&self);
    /// Posts a UI command (JSON) to the Java side; never blocks.
    fn ui(&self, json: String);
}

/// Where the loader keeps its files.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// Package cache (server mods, by SHA-256).
    pub cache: PathBuf,
    /// The player's installed mods.
    pub installed: PathBuf,
    /// The mods directory of the running host (rebuilt for every world).
    pub run: PathBuf,
    /// Mod storage, per world.
    pub storage: PathBuf,
}

impl Dirs {
    /// The standard layout under one data directory.
    pub fn under(root: &Path) -> Dirs {
        Dirs {
            cache: root.join("cache"),
            installed: root.join("installed"),
            run: root.join("run"),
            storage: root.join("storage"),
        }
    }
}

/// An answer from the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// The join prompt was answered.
    Decide(bool),
    /// The player cancelled the modded join (prompt or download).
    Cancel,
    /// A mod dialog was answered (`button` counts from 0, [`bhmod_core::ui::CLOSED`] when it went away).
    Dialog {
        /// The dialog id.
        dialog: u32,
        /// The pressed button.
        button: u32,
        /// The field's text.
        text: Option<String>,
    },
}

/// Builds the runtimes for a new host.
pub type RuntimeFactory = Box<dyn Fn() -> Vec<Box<dyn Runtime>>>;

/// Messages mods send on channels, flushed after every host call.
type Outbox = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

struct Bridge<P: Platform> {
    p: Rc<P>,
    outbox: Outbox,
}

impl<P: Platform> GameBridge for Bridge<P> {
    fn side(&self) -> Side {
        Side::Client
    }
    fn host_name(&self) -> String {
        format!("bhmod-android {}", crate::VERSION)
    }
    fn log(&self, mod_id: Option<&str>, level: LogLevel, message: &str) {
        self.p.log(level, &format!("[{}] {message}", mod_id.unwrap_or("loader")));
    }
    fn online_players(&self) -> Vec<String> {
        Vec::new()
    }
    fn player_alias(&self, _id: &str) -> Option<String> {
        None
    }
    fn player_role(&self, _id: &str) -> Role {
        Role::Player
    }
    fn send_chat(&self, _id: &str, _text: &str) -> bool {
        false
    }
    fn broadcast(&self, _text: &str) {}
    fn kick(&self, _id: &str, _reason: &str) -> bool {
        false
    }
    fn is_builtin_command(&self, _name: &str) -> bool {
        false
    }
    fn ui(&self, mod_id: &str, command: &bhmod_core::ui::UiCommand) -> Result<(), ApiError> {
        self.p.ui(ui::mod_ui(mod_id, command));
        Ok(())
    }
    fn channel_send(&self, _mod_id: &str, name: &str, target: &ChannelTarget, data: &[u8]) -> Result<(), ApiError> {
        match target {
            ChannelTarget::Server => {
                self.outbox.borrow_mut().push((name.to_string(), data.to_vec()));
                Ok(())
            }
            _ => Err(ApiError::Unsupported),
        }
    }
}

/// The loader.
pub struct Loader<P: Platform + 'static> {
    p: Rc<P>,
    dirs: Dirs,
    caps: Caps,
    opts: Options,
    runtimes: RuntimeFactory,
    session: Option<Session>,
    server: String,
    host: Option<ModHost>,
    outbox: Outbox,
    world: Option<String>,
    last_tick: u64,
    last_progress: u64,
    budgets: Budgets,
    content: Arc<Mutex<Content>>,
}

impl<P: Platform + 'static> Loader<P> {
    /// A loader with nothing running.
    pub fn new(p: Rc<P>, dirs: Dirs, caps: Caps, opts: Options, runtimes: RuntimeFactory) -> Loader<P> {
        Loader {
            p,
            dirs,
            caps,
            opts,
            runtimes,
            session: None,
            server: String::new(),
            host: None,
            outbox: Rc::new(RefCell::new(Vec::new())),
            world: None,
            last_tick: 0,
            last_progress: 0,
            budgets: Budgets::default(),
            content: Arc::new(Mutex::new(Content::new())),
        }
    }

    /// Time and memory budgets of the mods (the default is the server's).
    pub fn with_budgets(mut self, budgets: Budgets) -> Loader<P> {
        self.budgets = budgets;
        self
    }

    /// The installed-mods store (also used from Java threads through its own instance).
    pub fn installed(&self) -> Installed {
        Installed::new(&self.dirs.installed, self.caps.clone())
    }

    /// The cosmetics of the running mods and who wears them. Shared: the game hooks read it from the game's own
    /// threads.
    pub fn content(&self) -> Arc<Mutex<Content>> {
        self.content.clone()
    }

    fn content_lock(&self) -> std::sync::MutexGuard<'_, Content> {
        self.content.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// True while mods run.
    pub fn running(&self) -> bool {
        self.host.as_ref().is_some_and(|h| !h.is_empty())
    }

    /// The state of the current connection (for logs).
    pub fn session_state(&self) -> &'static str {
        self.session.as_ref().map_or("none", |s| s.state_name())
    }

    /// The server's first packet (0x23) arrived on a new connection to `server` (`host:port`).
    pub fn on_contact(&mut self, server: &str) {
        self.server = server.to_string();
        let mut s = Session::new(self.opts.clone(), PackageCache::new(&self.dirs.cache), self.caps.clone());
        let out = s.contact(self.p.now_ms());
        self.session = Some(s);
        self.p.log(LogLevel::Info, &format!("connecting to {server}: HELLO sent"));
        self.handle(out);
    }

    /// The game is about to send its 0x1f (held until [`Platform::release_join`]).
    pub fn on_join_request(&mut self) {
        match self.session.as_mut() {
            Some(s) => {
                let out = s.join_requested();
                self.handle(out);
            }
            None => self.p.release_join(None),
        }
    }

    /// A packet from the server. True when it was a 0xBC packet (the game never sees those).
    pub fn on_packet(&mut self, data: &[u8]) -> bool {
        if data.first() != Some(&PACKET_TYPE) {
            return false;
        }
        let now = self.p.now_ms();
        if let Some(s) = self.session.as_mut() {
            let out = s.receive(data, now);
            self.handle(out);
        }
        true
    }

    /// A network callback queued by another game thread ([`crate::inbox`]).
    pub fn on_net(&mut self, ev: NetEvent) {
        match ev {
            NetEvent::Contact(server) => self.on_contact(&server),
            NetEvent::Packet(data) => {
                self.on_packet(&data);
            }
            NetEvent::Disconnect => self.on_disconnect(),
        }
    }

    /// The connection closed.
    pub fn on_disconnect(&mut self) {
        if self.session.take().is_some() {
            self.p.ui(ui::closed());
        }
        if self.world.is_none() {
            self.stop_host();
        }
    }

    /// An answer from the UI.
    pub fn on_ui(&mut self, ev: UiEvent) {
        let now = self.p.now_ms();
        match ev {
            UiEvent::Decide(accept) => {
                if let Some(s) = self.session.as_mut() {
                    let out = s.decide(accept, now);
                    self.handle(out);
                }
            }
            UiEvent::Dialog { dialog, button, text } => {
                if let Some(h) = self.host.as_mut() {
                    h.ui_result(dialog, button, text);
                }
            }
            UiEvent::Cancel => {
                if self.session.take().is_some() {
                    self.p.log(LogLevel::Info, "the player cancelled the modded join");
                    self.p.abort_join();
                }
            }
        }
    }

    /// Once per frame: timeouts, mod ticks and channel traffic.
    pub fn on_frame(&mut self) {
        let now = self.p.now_ms();
        if let Some(s) = self.session.as_mut() {
            let out = s.tick(now);
            self.handle(out);
        }
        if let Some(h) = self.host.as_mut() {
            h.poll();
            if now >= self.last_tick + TICK_MS {
                let dt = if self.last_tick == 0 { TICK_MS } else { now - self.last_tick };
                self.last_tick = now;
                h.tick(dt as f32 / 1000.0);
            }
        }
        self.flush_outbox();
    }

    /// A world opened (`Some(name)`) or closed (`None`).
    pub fn on_world(&mut self, world: Option<&str>) {
        match world {
            Some(name) => {
                self.world = Some(name.to_string());
                if self.host.is_none() {
                    if let Err(e) = self.start_host(&[]) {
                        self.p.log(LogLevel::Error, &format!("installed mods did not start: {e}"));
                    }
                }
                let id =
                    if self.server.is_empty() { format!("local:{name}") } else { format!("server:{}", self.server) };
                if let Some(h) = self.host.as_mut() {
                    h.enable(name, &id);
                }
                self.flush_outbox();
                let n =
                    self.host.as_ref().map_or(0, |h| h.mods().iter().filter(|m| m.state == ModState::Enabled).count());
                self.p.ui(ui::world(true, n));
            }
            None => {
                self.world = None;
                self.stop_host();
                self.session = None;
                self.server.clear();
                self.p.ui(ui::world(false, 0));
            }
        }
    }

    fn stop_host(&mut self) {
        if let Some(mut h) = self.host.take() {
            h.disable_all();
        }
        *self.content_lock() = Content::new();
        self.last_tick = 0;
    }

    fn flush_outbox(&mut self) {
        let msgs: Vec<_> = self.outbox.borrow_mut().drain(..).collect();
        for (name, data) in msgs {
            match self.session.as_ref().and_then(|s| s.channel(&name, &data)) {
                Some(pkt) => self.p.send(&pkt),
                None => self.p.log(LogLevel::Warn, &format!("channel {name}: not connected to a modded server")),
            }
        }
    }

    fn handle(&mut self, outs: Vec<Output>) {
        for o in outs {
            match o {
                Output::Send(b) => self.p.send(&b),
                Output::Prompt { required, mods, download } => {
                    self.p.log(
                        LogLevel::Info,
                        &format!("{} lists {} mod(s), asking the player", self.server, mods.len()),
                    );
                    self.p.ui(ui::prompt(&self.server, required, &mods, download));
                }
                Output::Progress { id, index, count, done, size } => {
                    let now = self.p.now_ms();
                    if done == size || now >= self.last_progress + PROGRESS_MS {
                        self.last_progress = now;
                        self.p.ui(ui::progress(&id, index, count, done, size));
                    }
                }
                Output::Load(accepted) => {
                    let result = self.start_host(&accepted);
                    if let Err(e) = &result {
                        self.p.log(LogLevel::Error, &format!("server mods did not start: {e}"));
                    }
                    let out = self.session.as_mut().map(|s| s.loaded(result)).unwrap_or_default();
                    self.handle(out);
                }
                Output::Join(key) => {
                    self.p.log(
                        LogLevel::Info,
                        &format!("joining {} ({})", self.server, if key.is_some() { "modded" } else { "vanilla" }),
                    );
                    self.p.release_join(key);
                    self.p.ui(ui::joined());
                }
                Output::Failed(f) => {
                    self.p.log(LogLevel::Warn, &format!("modded join stopped: {f}"));
                    self.p.ui(ui::failed(&self.server, &f, &self.opts.loader));
                    self.p.abort_join();
                }
                Output::Reject(r) => self.p.ui(ui::reject(&self.server, &r)),
                Output::Cosmetics(c) => {
                    let changed = self.content_lock().apply(&c);
                    self.p.log(
                        LogLevel::Info,
                        &format!("cosmetics: {} player(s) in the message, {} changed", c.players.len(), changed.len()),
                    );
                }
                Output::Channel { name, data } => {
                    if let Some(h) = self.host.as_mut() {
                        h.channel_receive(&name, ChannelSource::Server, data);
                    }
                }
            }
        }
    }

    /// Reads the cosmetics of the packages that run, with the ids of the server's world.
    fn load_content(&mut self, packages: &[PathBuf]) {
        let registry = self.session.as_ref().map(|s| s.registry().clone()).unwrap_or_default();
        let problems = self.content_lock().load(&registry, packages);
        for problem in problems {
            self.p.log(LogLevel::Warn, &format!("content: {problem}"));
        }
    }

    /// Builds the run directory from the server's packages plus the player's enabled installed mods (server
    /// versions win) and starts a host. Returns the `(id, version)` of every loaded mod, or an error when a server
    /// mod did not start.
    fn start_host(&mut self, server_mods: &[Accepted]) -> Result<Vec<(String, String)>, String> {
        self.stop_host();
        let run_dir = self.dirs.run.clone();
        let run = &run_dir;
        let _ = fs::remove_dir_all(run);
        let mut grants: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut ids = BTreeSet::new();
        let mut placed: Vec<PathBuf> = Vec::new();
        let mut copy = |from: &Path, id: &str| -> io::Result<()> {
            fs::create_dir_all(run)?;
            let to = run.join(format!("{id}.bhmod"));
            fs::copy(from, &to)?;
            placed.push(to);
            Ok(())
        };
        for a in server_mods {
            copy(&a.path, &a.manifest.id).map_err(|e| e.to_string())?;
            grants.insert(a.manifest.id.clone(), bhmod_core::config::required_permissions(&a.manifest.permissions));
            ids.insert(a.manifest.id.clone());
        }
        for m in self.installed().list().into_iter().filter(|m| m.enabled && !ids.contains(&m.manifest.id)) {
            if copy(&m.path, &m.manifest.id).is_ok() {
                grants.insert(m.manifest.id.clone(), bhmod_core::config::required_permissions(&m.manifest.permissions));
            }
        }
        self.load_content(&placed);
        if grants.is_empty() {
            return Ok(Vec::new());
        }
        let mut toml = String::new();
        for (id, g) in &grants {
            let list: Vec<String> = g.iter().map(|p| format!("{p:?}")).collect();
            toml.push_str(&format!("[{id}]\ngrant = [{}]\n\n", list.join(", ")));
        }
        fs::write(run.join(bhmod_core::config::FILE_NAME), toml).map_err(|e| e.to_string())?;
        let mut opts = HostOptions::new(run, Side::Client);
        opts.runtimes = (self.runtimes)();
        opts.budgets = self.budgets;
        opts.storage = Box::new(FileStorage::new(&self.dirs.storage));
        let bridge: Rc<dyn GameBridge> = Rc::new(Bridge { p: self.p.clone(), outbox: self.outbox.clone() });
        let host = ModHost::load(opts, bridge);
        let loaded: Vec<(String, String)> =
            host.mods().into_iter().filter(|m| m.state == ModState::Loaded).map(|m| (m.id, m.version)).collect();
        self.host = Some(host);
        if let Some(missing) = server_mods.iter().find(|a| !loaded.iter().any(|(id, _)| *id == a.manifest.id)) {
            return Err(format!("{} did not start", missing.manifest.id));
        }
        Ok(loaded)
    }
}

#[cfg(test)]
mod tests;
