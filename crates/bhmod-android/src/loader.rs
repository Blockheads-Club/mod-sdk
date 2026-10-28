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

use bhmod_client::cache::sha256;
use bhmod_client::installed::Installed;
use bhmod_client::native;
use bhmod_client::session::{Accepted, Output};
use bhmod_client::{Caps, Options, PackageCache, Session};
use bhmod_content::Content;
use bhmod_core::budget::Budgets;
use bhmod_core::host::{HostOptions, ModHost, ModState, ReloadResult};
use bhmod_core::remote::{RemoteCall, RemoteError, RemoteTarget};
use bhmod_core::runtime::Runtime;
use bhmod_core::storage::FileStorage;
use bhmod_core::timings::ModTimings;
use bhmod_core::types::{ApiError, ChannelSource, ChannelTarget, LogLevel, Role, Side};
use bhmod_core::GameBridge;
use bhmod_proto::{hex, Hash, ServiceCall, ServiceFailure, ServiceResult, PACKET_TYPE};

use bhmod_core::input::InputCommand;
use bhmod_core::ui::{ButtonAction, UiCommand};
use bhmod_core::voice::VoiceHost;

use crate::device::DeviceStore;
use crate::hud::Hud;
use crate::inbox::NetEvent;
use crate::overhead::{Camera, Tracker};
use crate::ui;
use crate::voice::audio::AudioIo;
use crate::voice::link::{VoiceLink, VoiceSlot};
use crate::voice::pipeline::Pipeline;
use crate::voice::udp::UdpLink;

mod client;

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
    /// Loads a native mod library (`client/native/armeabi-v7a/*.so`) and runs its `bhmod_native_init`. `data_dir` is
    /// a private writable folder of the mod.
    fn load_native(&self, mod_id: &str, lib: &Path, data_dir: &Path) -> Result<(), String>;
    /// Runs the mod's `bhmod_native_shutdown` (if any) and undoes its hooks.
    fn unload_native(&self, mod_id: &str);
    /// Fresh voice audio streams (nothing open yet); `None` without an audio backend.
    fn audio(&self) -> Option<AudioIo>;
    /// Shows the system `RECORD_AUDIO` dialog; the answer comes back as [`UiEvent::MicPermission`].
    fn ask_mic(&self);
    /// Sends a whole 0xBC packet unsequenced on ENet channel 0 (loader thread, in a world). False when it could not
    /// leave.
    fn send_unsequenced(&self, packet: &[u8]) -> bool;
    /// The game's world-to-screen transform now (overhead icons); `None` outside a world.
    fn camera(&self) -> Option<Camera>;
    /// This client's own player id while connected to a server.
    fn local_player(&self) -> Option<String>;
}

/// Makes the voice client of a new voice-enabled connection.
pub type VoiceFactory = Box<dyn Fn() -> Option<Rc<dyn VoiceLink>>>;

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
    /// Native libraries taken out of packages (`<sha256>/<abi>/<name>.so`) and the data folders of native mods.
    pub native: PathBuf,
    /// What the player set on this phone ([`DeviceStore`]).
    pub device: PathBuf,
}

impl Dirs {
    /// The standard layout under one data directory.
    pub fn under(root: &Path) -> Dirs {
        Dirs {
            cache: root.join("cache"),
            installed: root.join("installed"),
            run: root.join("run"),
            storage: root.join("storage"),
            native: root.join("native"),
            device: root.join("device"),
        }
    }
}

/// An answer from the UI thread.
#[derive(Debug, Clone, PartialEq)]
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
    /// (0.3.4) An on-screen button of a mod was used.
    Button {
        /// The mod.
        mod_id: String,
        /// The button id.
        id: String,
        /// `press`, `long-press` or `release`.
        action: String,
    },
    /// (0.3.4) The player dropped a button at screen fractions `x`, `y`.
    ButtonMoved {
        /// The mod.
        mod_id: String,
        /// The button id.
        id: String,
        /// Centre, fraction of the width.
        x: f32,
        /// Centre, fraction of the height.
        y: f32,
    },
    /// The player picked a voice mode in the long-press picker.
    VoiceMode(String),
    /// A hardware key went down (`true`) or up.
    Key(String, bool),
    /// Open the settings screen of a mod (the Mods screen).
    OpenSettings(String),
    /// A setting changed in the settings screen.
    Setting {
        /// The mod.
        mod_id: String,
        /// `client`, `server`, `keys` or `voice`.
        scope: String,
        /// The setting key, action or `master`.
        key: String,
        /// The new value as JSON text.
        json: String,
    },
    /// The answer of the `RECORD_AUDIO` dialog.
    MicPermission(bool),
    /// The host's push-to-talk seam (`Native.ptt`): talk in the default mode.
    Ptt(bool),
    /// A tap on the game at screen fractions `x`, `y` (a blockhead there is picked: `<mod-id>:ui-pick`).
    Tap(f32, f32),
}

/// Builds the runtimes for a new host.
pub type RuntimeFactory = Box<dyn Fn() -> Vec<Box<dyn Runtime>>>;

/// What mods send to the server, flushed after every host call.
enum Outgoing {
    /// A channel message (name, data).
    Channel(String, Vec<u8>),
    /// A remote service call.
    Call(RemoteCall),
}

type Outbox = Rc<RefCell<Vec<Outgoing>>>;

struct Bridge<P: Platform> {
    p: Rc<P>,
    outbox: Outbox,
    hud: Rc<RefCell<Hud>>,
    slot: Rc<VoiceSlot>,
    device: DeviceStore,
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
        // A client knows the other players from the voice state (`VOICE_STATE.names`).
        self.slot.get().map(|l| l.players().into_iter().map(|(id, _)| id).collect()).unwrap_or_default()
    }
    fn player_alias(&self, id: &str) -> Option<String> {
        self.slot.get()?.alias(id)
    }
    fn local_player(&self) -> Option<String> {
        self.p.local_player()
    }
    fn platform(&self) -> String {
        crate::PLATFORM.to_string()
    }
    fn has_touch(&self) -> bool {
        true
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
    fn ui(&self, mod_id: &str, command: &UiCommand) -> Result<(), ApiError> {
        let mut hud = self.hud.borrow_mut();
        match command {
            UiCommand::SetOverhead { player, icon } => hud.board.set(mod_id, player, icon.name()),
            UiCommand::ClearOverhead { player } => {
                hud.board.clear(mod_id, player);
            }
            UiCommand::SetButton { id, button } => {
                self.p.ui(ui::button_set(mod_id, id, button, self.device.button_pos(mod_id, id)));
            }
            UiCommand::OpenSettings { mod_id: target } => hud.open_settings.push(target.clone()),
            UiCommand::ClearMod => {
                hud.board.clear_mod(mod_id);
                self.p.ui(ui::mod_ui(mod_id, command));
            }
            _ => self.p.ui(ui::mod_ui(mod_id, command)),
        }
        Ok(())
    }
    fn voice(&self) -> Option<&dyn VoiceHost> {
        self.slot.is_set().then_some(&*self.slot as &dyn VoiceHost)
    }
    fn input(&self, mod_id: &str, command: &InputCommand) -> Result<(), ApiError> {
        self.hud.borrow_mut().input(mod_id, command);
        Ok(())
    }
    fn input_key(&self, mod_id: &str, action: &str, default_key: &str) -> Option<String> {
        Hud::key_of(&self.device, mod_id, action, default_key)
    }
    fn channel_send(&self, _mod_id: &str, name: &str, target: &ChannelTarget, data: &[u8]) -> Result<(), ApiError> {
        match target {
            ChannelTarget::Server => {
                self.outbox.borrow_mut().push(Outgoing::Channel(name.to_string(), data.to_vec()));
                Ok(())
            }
            _ => Err(ApiError::Unsupported),
        }
    }
    fn service_send(&self, call: &RemoteCall) -> Result<(), RemoteError> {
        self.outbox.borrow_mut().push(Outgoing::Call(call.clone()));
        Ok(())
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
    /// Mods whose native library is loaded (unloaded when the host stops).
    natives: Vec<String>,
    /// The mods the server sent for the current connection (what a hot reload places again).
    accepted: Vec<Accepted>,
    /// Overhead icons, key bindings, settings screens to open (shared with the bridge).
    hud: Rc<RefCell<Hud>>,
    /// The voice client the mods reach (shared with the bridge).
    slot: Rc<VoiceSlot>,
    device: DeviceStore,
    voice_factory: Option<VoiceFactory>,
    /// Audio and packets of the voice client, while one runs.
    pipeline: Option<Pipeline>,
    udp: Option<UdpLink>,
    /// The modes the server allows (`VOICE_STATE.modes`), as last posted to the picker.
    modes: Option<u8>,
    /// The voice preferences as last saved, and when they were last compared.
    prefs_saved: String,
    prefs_at: u64,
    /// Blockheads the game's update hook reported (overhead icons).
    tracker: Arc<Tracker>,
    /// True while the overlay shows overhead icons.
    overhead_shown: bool,
    /// Server-scope settings forms of mods (`SETTINGS_SCHEMA`; only admins get them).
    server_forms: BTreeMap<String, ui::Form>,
    /// The mod whose settings screen is open.
    settings_open: Option<String>,
    /// Mods granted `voice` in the running host.
    voice_mods: BTreeSet<String>,
}

impl<P: Platform + 'static> Loader<P> {
    /// A loader with nothing running.
    pub fn new(p: Rc<P>, dirs: Dirs, caps: Caps, opts: Options, runtimes: RuntimeFactory) -> Loader<P> {
        let dirs_device = dirs.device.clone();
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
            natives: Vec::new(),
            accepted: Vec::new(),
            hud: Rc::default(),
            slot: Rc::default(),
            device: DeviceStore::new(&dirs_device),
            voice_factory: None,
            pipeline: None,
            udp: None,
            modes: None,
            prefs_saved: String::new(),
            prefs_at: 0,
            tracker: Arc::new(Tracker::new()),
            overhead_shown: false,
            server_forms: BTreeMap::new(),
            settings_open: None,
            voice_mods: BTreeSet::new(),
        }
    }

    /// Voice chat: the factory of the voice client for connections whose server and loader both list `voice`.
    pub fn with_voice(mut self, factory: VoiceFactory) -> Loader<P> {
        self.voice_factory = Some(factory);
        self
    }

    /// The blockheads the game's update hook reports (shared: the hooks run on the game's other threads).
    pub fn tracker(&self) -> Arc<Tracker> {
        self.tracker.clone()
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
        self.content.lock().expect("the content lock is never held across a panic")
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
        let mut opts = self.opts.clone();
        opts.native_trusted = self.native_trusted();
        self.offer_native_to_cache();
        let mut s = Session::new(opts, PackageCache::new(&self.dirs.cache), self.caps.clone());
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
        self.stop_voice();
        self.server_forms.clear();
        if self.session.take().is_some() {
            self.p.ui(ui::closed());
        }
        if let Some(h) = self.host.as_mut() {
            // Remote calls still open can never be answered.
            h.remote_peer_gone(&RemoteTarget::Server);
        }
        if self.world.is_none() {
            self.stop_host();
        }
        self.flush_outbox();
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
            UiEvent::Button { mod_id, id, action } => self.on_button(&mod_id, &id, &action),
            UiEvent::ButtonMoved { mod_id, id, x, y } => {
                if let Err(e) = self.device.set_button_pos(&mod_id, &id, x, y) {
                    self.p.log(LogLevel::Warn, &format!("button position not saved: {e}"));
                }
            }
            UiEvent::VoiceMode(m) => {
                if let Some(mode) = bhmod_core::voice::Mode::parse(&m) {
                    let _ = self.slot.set_default_mode(mode);
                }
            }
            UiEvent::Key(name, down) => self.on_key(&name, down),
            UiEvent::OpenSettings(id) => self.open_settings(&id),
            UiEvent::Setting { mod_id, scope, key, json } => self.set_setting(&mod_id, &scope, &key, &json),
            UiEvent::MicPermission(granted) => self.on_mic_permission(granted),
            UiEvent::Ptt(down) => {
                let _ = self.slot.talk(None, down);
            }
            UiEvent::Tap(x, y) => self.on_tap(x, y),
        }
        self.flush_outbox();
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
        self.voice_frame(now);
        self.hud_frame(now);
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
                self.start_voice();
                let n =
                    self.host.as_ref().map_or(0, |h| h.mods().iter().filter(|m| m.state == ModState::Enabled).count());
                let settings = self.settings_mods();
                self.p.ui(ui::world(true, n, &settings));
            }
            None => {
                self.world = None;
                self.stop_voice();
                self.stop_host();
                self.session = None;
                self.server.clear();
                self.server_forms.clear();
                self.p.ui(ui::world(false, 0, &[]));
            }
        }
    }

    /// SHA-256 of every installed native package the player confirmed (the only native code that may run).
    fn native_trusted(&self) -> Vec<Hash> {
        let natives = self.installed().list().into_iter().filter(|m| m.manifest.permissions.native);
        natives.filter_map(|m| fs::read(&m.path).ok()).map(|b| sha256(&b)).collect()
    }

    /// Puts the confirmed native packages into the package cache, so a server that lists one (it cannot send it) is
    /// answered from the player's own copy.
    fn offer_native_to_cache(&self) {
        let cache = PackageCache::new(&self.dirs.cache);
        let natives = self.installed().list().into_iter().filter(|m| m.manifest.permissions.native);
        for bytes in natives.filter_map(|m| fs::read(&m.path).ok()) {
            let _ = cache.put(&bytes);
        }
    }

    /// Loads the native libraries of the mods that started: `(id, package path)` of every native candidate.
    fn start_natives(&mut self, candidates: &[(String, PathBuf)], loaded: &[(String, String)]) {
        let trusted = self.native_trusted();
        for (id, path) in candidates.iter().filter(|(id, _)| loaded.iter().any(|(l, _)| l == id)) {
            let Ok(bytes) = fs::read(path) else { continue };
            if !trusted.contains(&sha256(&bytes)) {
                self.p.log(
                    LogLevel::Warn,
                    &format!("{id}: native code is not an installed, confirmed package, not loaded"),
                );
                continue;
            }
            let sha = hex(&sha256(&bytes));
            let libs = match native::extract(path, &sha, native::ABI, &self.dirs.native) {
                Ok(l) if !l.is_empty() => l,
                Ok(_) => {
                    self.p.log(LogLevel::Warn, &format!("{id}: no {} library in the package", native::ABI));
                    continue;
                }
                Err(e) => {
                    self.p.log(LogLevel::Error, &format!("{id}: cannot unpack the native library: {e}"));
                    continue;
                }
            };
            let data = self.dirs.native.join("data").join(id);
            let _ = fs::create_dir_all(&data);
            let mut ok = true;
            for lib in &libs {
                match self.p.load_native(id, lib, &data) {
                    Ok(()) => self.p.log(LogLevel::Warn, &format!("{id}: native code loaded ({})", lib.display())),
                    Err(e) => {
                        self.p.log(LogLevel::Error, &format!("{id}: native code failed: {e}"));
                        ok = false;
                    }
                }
            }
            if ok {
                self.natives.push(id.clone());
            } else {
                self.p.unload_native(id);
            }
        }
    }

    fn stop_host(&mut self) {
        for id in self.natives.drain(..) {
            self.p.unload_native(&id);
        }
        if let Some(mut h) = self.host.take() {
            h.disable_all();
        }
        self.hud.borrow_mut().clear();
        self.settings_open = None;
        self.voice_mods.clear();
        *self.content_lock() = Content::new();
        self.last_tick = 0;
    }

    /// Sends what the mods queued. A failed call is answered to the mod at once (`unavailable`), which may queue
    /// more, so this repeats (a mod that calls again from every failure cannot spin: the host caps its open calls).
    fn flush_outbox(&mut self) {
        loop {
            let msgs: Vec<_> = self.outbox.borrow_mut().drain(..).collect();
            if msgs.is_empty() {
                return;
            }
            for m in msgs {
                match m {
                    Outgoing::Channel(name, data) => {
                        match self.session.as_ref().and_then(|s| s.channel(&name, &data)) {
                            Some(pkt) => self.p.send(&pkt),
                            None => {
                                self.p.log(LogLevel::Warn, &format!("channel {name}: not connected to a modded server"))
                            }
                        }
                    }
                    Outgoing::Call(call) => {
                        match self.session.as_ref().and_then(|s| s.service_call(ServiceCall::from(&call))) {
                            Some(pkt) => self.p.send(&pkt),
                            None => {
                                if let Some(h) = self.host.as_mut() {
                                    h.remote_result(&RemoteTarget::Server, call.id, Err(RemoteError::Unavailable));
                                }
                            }
                        }
                    }
                }
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
                Output::Voice(m) => self.on_voice_message(&m),
                Output::SettingsSchema(s) => self.on_settings_schema(&s),
                Output::SettingsResult(r) => self.on_settings_result(&r),
                Output::Skipped(mods) => {
                    let ids: Vec<&str> = mods.iter().map(|m| m.id.as_str()).collect();
                    self.p.log(LogLevel::Warn, &format!("optional mods this loader cannot run: {ids:?}"));
                    self.p.ui(ui::skipped(&self.server, &mods));
                }
                Output::Cosmetics(c) => {
                    let changed = self.content_lock().apply(&c);
                    self.p.log(
                        LogLevel::Info,
                        &format!(
                            "cosmetics: {} player(s) in the message, {} changed: {:?}",
                            c.players.len(),
                            changed.len(),
                            c.players
                        ),
                    );
                }
                Output::Channel { name, data } => {
                    if let Some(h) = self.host.as_mut() {
                        h.channel_receive(&name, ChannelSource::Server, data);
                    }
                }
                Output::ServiceCall(c) => {
                    let Some(h) = self.host.as_mut() else { continue };
                    let out = h.remote_call_in(
                        &RemoteTarget::Server,
                        &c.mod_id,
                        &c.service,
                        &c.version_req,
                        &c.method,
                        &c.args,
                    );
                    let outcome = out.map_err(|e| ServiceFailure::from(&e));
                    if let Some(pkt) =
                        self.session.as_ref().and_then(|s| s.service_result(ServiceResult { id: c.id, outcome }))
                    {
                        self.p.send(&pkt);
                    }
                }
                Output::ServiceResult(r) => {
                    if let Some(h) = self.host.as_mut() {
                        h.remote_result(&RemoteTarget::Server, r.id, r.outcome.map_err(RemoteError::from));
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

    /// Fills the run directory from the server's packages plus the player's enabled installed mods (server versions
    /// win) and writes `mods.toml`. Returns the packages placed, the mods that need their native library, and
    /// whether there is anything to run.
    fn place(&self, server_mods: &[Accepted]) -> Result<Placement, String> {
        let run = &self.dirs.run;
        let _ = fs::remove_dir_all(run);
        let mut grants: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut ids = BTreeSet::new();
        let mut placed: Vec<PathBuf> = Vec::new();
        let mut natives: Vec<(String, PathBuf)> = Vec::new();
        let mut copy = |from: &Path, id: &str| -> io::Result<()> {
            fs::create_dir_all(run)?;
            let to = run.join(format!("{id}.bhmod"));
            fs::copy(from, &to)?;
            placed.push(to);
            Ok(())
        };
        for a in server_mods {
            copy(&a.path, &a.manifest.id).map_err(|e| e.to_string())?;
            if a.manifest.permissions.native {
                natives.push((a.manifest.id.clone(), a.path.clone()));
            }
            grants.insert(a.manifest.id.clone(), bhmod_core::config::required_permissions(&a.manifest.permissions));
            ids.insert(a.manifest.id.clone());
        }
        for m in self.installed().list().into_iter().filter(|m| m.enabled && !ids.contains(&m.manifest.id)) {
            if copy(&m.path, &m.manifest.id).is_ok() {
                if m.manifest.permissions.native {
                    natives.push((m.manifest.id.clone(), m.path.clone()));
                }
                grants.insert(m.manifest.id.clone(), bhmod_core::config::required_permissions(&m.manifest.permissions));
            }
        }
        if grants.is_empty() {
            return Ok(Placement { placed, natives, any: false });
        }
        let mut toml = String::new();
        for (id, g) in &grants {
            let list: Vec<String> = g.iter().map(|p| format!("{p:?}")).collect();
            toml.push_str(&format!("[{id}]\ngrant = [{}]\n\n", list.join(", ")));
        }
        fs::write(run.join(bhmod_core::config::FILE_NAME), toml).map_err(|e| e.to_string())?;
        Ok(Placement { placed, natives, any: true })
    }

    /// Builds the run directory (see [`Loader::place`]) and starts a host. Returns the `(id, version)` of every
    /// loaded mod, or an error when a server mod did not start.
    fn start_host(&mut self, server_mods: &[Accepted]) -> Result<Vec<(String, String)>, String> {
        self.stop_host();
        let Placement { placed, natives, any } = self.place(server_mods)?;
        self.load_content(&placed);
        self.accepted = server_mods.to_vec();
        if !any {
            return Ok(Vec::new());
        }
        let installed = self.installed().list();
        self.voice_mods = server_mods
            .iter()
            .map(|a| &a.manifest)
            .chain(installed.iter().filter(|m| m.enabled).map(|m| &m.manifest))
            .filter(|m| m.permissions.voice)
            .map(|m| m.id.clone())
            .collect();
        let mut opts = HostOptions::new(&self.dirs.run, Side::Client);
        // The host writes the client settings file but does not make its folder.
        let _ = fs::create_dir_all(&self.dirs.device);
        opts.client_settings = Some(self.device.client_settings());
        opts.runtimes = (self.runtimes)();
        opts.budgets = self.budgets;
        opts.storage = Box::new(FileStorage::new(&self.dirs.storage));
        let bridge: Rc<dyn GameBridge> = Rc::new(Bridge {
            p: self.p.clone(),
            outbox: self.outbox.clone(),
            hud: self.hud.clone(),
            slot: self.slot.clone(),
            device: self.device.clone(),
        });
        let host = ModHost::load(opts, bridge);
        let loaded: Vec<(String, String)> =
            host.mods().into_iter().filter(|m| m.state == ModState::Loaded).map(|m| (m.id, m.version)).collect();
        self.host = Some(host);
        self.start_natives(&natives, &loaded);
        if let Some(missing) = server_mods.iter().find(|a| !loaded.iter().any(|(id, _)| *id == a.manifest.id)) {
            return Err(format!("{} did not start", missing.manifest.id));
        }
        Ok(loaded)
    }

    /// Hot reload: puts the current packages (the server's, and the installed mods that are enabled now) in the run
    /// directory again and lets the host restart the mods whose package changed, or just `only`, without leaving the
    /// world. Native libraries and content (cosmetics, items, blocks) are not reloaded; those need a new connection.
    pub fn reload(&mut self, only: Option<&str>) -> Result<Vec<ReloadResult>, String> {
        if self.host.is_none() {
            return Err("no mods are running".to_string());
        }
        let accepted = self.accepted.clone();
        self.place(&accepted)?;
        let results = self.host.as_mut().map(|h| h.reload(only)).unwrap_or_default();
        self.flush_outbox();
        Ok(results)
    }

    /// The time accounting of the running mods (see [`ModHost::timings`]).
    pub fn timings(&self, only: Option<&str>) -> Vec<ModTimings> {
        self.host.as_ref().map(|h| h.timings(only)).unwrap_or_default()
    }

    /// Clears the time accounting; false when `only` names no running mod.
    pub fn reset_timings(&mut self, only: Option<&str>) -> bool {
        self.host.as_mut().is_some_and(|h| h.reset_timings(only))
    }
}

/// What [`Loader::place`] put in the run directory.
struct Placement {
    placed: Vec<PathBuf>,
    natives: Vec<(String, PathBuf)>,
    any: bool,
}

#[cfg(test)]
mod tests;
