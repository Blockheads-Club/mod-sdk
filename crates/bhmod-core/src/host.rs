//! [`ModHost`]: loads mods and runs them against a [`GameBridge`].
//!
//! The host program calls the `ModHost` from its main thread at its hook points (world loaded, join, quit, chat,
//! command, tick, poll, save, stop, and the API 0.2 gameplay events through [`ModHost::fire`]). Every guest call
//! runs under the time budget, inside `catch_unwind`; traps, timeouts and panics are strikes, and too many strikes
//! disable the mod, never the game.
//!
//! Mods call each other through the host (services, custom events, channels). A mod whose code is running is on
//! the call stack; calling into it again (A -> B -> A) gives `reentrant` for services. Events fired meanwhile
//! (including a mod's own custom events) reach that mod's handlers later: they are queued and delivered once the
//! outermost call has returned, and their outcome is ignored (the event is already decided). A nested call ends at
//! the earlier of its own hard deadline and the caller's.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use bhmod_manifest::settings::{Scope, Settings, SettingsError};
use semver::VersionReq;
use serde_json::{Map, Value as Json};

use crate::bridge::GameBridge;
use crate::budget::{Budgets, Clock, Strikes, SystemClock};
use crate::bus::{EventBus, Subscription};
use crate::channels::{self, ChannelId, Channels, Message};
use crate::commands::CommandRegistry;
use crate::config::{required_permissions, ModsConfig, FILE_NAME};
use crate::discover::{discover, Package};
use crate::handles::Handles;
use crate::input::{InputCommand, InputState};
use crate::remote::{self, Remote, RemoteCall, RemoteCaller, RemoteError, RemoteLimits, RemoteTarget};
use crate::resolve::resolve;
use crate::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, Runtime, SubscriptionId, TaskId};
use crate::scheduler::Scheduler;
use crate::services::ServiceRegistry;
use crate::settings as cfg;
use crate::storage::{MemoryStorage, StorageProvider, Store};
use crate::timings::{HandlerKind, ModTimings, Timings};
use crate::types::{
    ApiError, ChannelSource, ChannelTarget, CommandOutcome, CommandSender, CommandSpec, ContentKind, CosmeticInfo,
    Event, EventKind, LogLevel, Pos, Priority, Role, SavePhase, ServiceError, Side,
};
use crate::ui::{Button, ButtonAction, Dialog, NoticeLevel, OverheadIcon, UiCommand, UiState, Widget};
use crate::value::Value;
use crate::voice::{VoiceCall, VoiceEvent, VoiceReply};
use crate::API_VERSION;

/// Longest log message in bytes.
pub const MAX_LOG: usize = 4096;
/// Most deferred event deliveries run after one outermost call.
pub const MAX_DEFERRED: usize = 1024;

/// How to build a [`ModHost`].
pub struct HostOptions {
    /// The mods directory (packages and `mods.toml`).
    pub mods_dir: PathBuf,
    /// This host's side.
    pub side: Side,
    /// Time and memory budgets.
    pub budgets: Budgets,
    /// Available runtimes.
    pub runtimes: Vec<Box<dyn Runtime>>,
    /// Where mod storage lives (per world).
    pub storage: Box<dyn StorageProvider>,
    /// The clock used for budgets and strikes.
    pub clock: Box<dyn Clock>,
    /// Rate and count limits of cross-side service calls.
    pub remote: RemoteLimits,
    /// (API 0.3.4, clients) The device's file for `scope = "client"` settings (same layout as `mods.toml`:
    /// `[<mod-id>.settings]`). When set on a client host, settings are read from and written to it (valid on every
    /// server) and only client-scope keys can be changed; `None` keeps them in `mods.toml`.
    pub client_settings: Option<PathBuf>,
}

impl HostOptions {
    /// Options with the default budgets, in-memory storage, the system clock and every built-in runtime.
    pub fn new(mods_dir: impl Into<PathBuf>, side: Side) -> HostOptions {
        HostOptions {
            mods_dir: mods_dir.into(),
            side,
            budgets: Budgets::default(),
            runtimes: crate::default_runtimes(),
            storage: Box::new(MemoryStorage::new()),
            clock: Box::new(SystemClock),
            remote: RemoteLimits::default(),
            client_settings: None,
        }
    }
}

/// The state of one mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModState {
    /// Instantiated, waiting for the world.
    Loaded,
    /// Running.
    Enabled,
    /// Stopped (the text says why).
    Disabled(String),
}

/// A loaded mod as shown to admins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInfo {
    /// Mod id.
    pub id: String,
    /// Mod version.
    pub version: String,
    /// Where it came from.
    pub origin: PathBuf,
    /// SHA-256 of its zip.
    pub sha256: Option<String>,
    /// Current state.
    pub state: ModState,
}

struct Slot {
    info: ModInfo,
    permissions: Vec<String>,
    /// Ids of the mods this one depends or soft-depends on (it may call their services and open their channels).
    deps: Vec<String>,
    /// `[provides]` as `(name, version)`.
    provides: Vec<(String, String)>,
    /// `[content] cosmetics` as `(key, slot)`.
    cosmetics: Vec<(String, String)>,
    /// `[content] items` keys.
    items: Vec<String>,
    /// `[content] blocks` as `(key, has a known base)`.
    blocks: Vec<(String, bool)>,
    /// The `[provides]` services that are `remote = true`, as `(name, allowed methods)`; no methods = all.
    remote: Vec<(String, Vec<String>)>,
    instance: Option<Box<dyn Instance>>,
    store: Option<Store>,
    strikes: Strikes,
    timings: Timings,
    /// 1 for the first load, one more after every reload (what a service proxy remembers to notice a reload).
    generation: u32,
    /// Hash of the package this slot runs (what `reload` compares).
    fingerprint: u64,
    /// `[settings]` of the manifest.
    settings: Rc<Settings>,
    /// The current value of every setting.
    values: Map<String, Json>,
    /// Whether players must run the client half (manifest and the admin's `client-required`).
    client_required: bool,
    /// A call into this instance finished (until then calls get the cold-start limit, [`Budgets::cold`]).
    warm: bool,
}

/// The mod host.
pub struct ModHost {
    slots: Vec<Slot>,
    bus: EventBus,
    commands: CommandRegistry,
    scheduler: Scheduler,
    services: ServiceRegistry,
    channels: Channels,
    handles: Handles,
    ui: UiState,
    input: InputState,
    remote: Remote,
    /// Mods whose code is running, outermost first, with the deadline of each call.
    stack: Vec<(usize, Instant)>,
    /// Event deliveries to mods that were on the stack when the event fired.
    deferred: Vec<(Subscription, Event)>,
    draining: bool,
    bridge: Rc<dyn GameBridge>,
    side: Side,
    budgets: Budgets,
    storage: Box<dyn StorageProvider>,
    clock: Box<dyn Clock>,
    enabled: bool,
    tick: u64,
    mods_dir: PathBuf,
    runtimes: Vec<Box<dyn Runtime>>,
    /// `(world name, world id)` once the world is loaded (a reloaded mod gets `world-loaded` with it).
    world: Option<(String, String)>,
    /// The last `mods.toml` that parsed (settings keep their values when an edit breaks the file).
    config: ModsConfig,
    /// What `mods.toml` looked like when it was last read or written (see [`cfg::stamp`]).
    config_stamp: u64,
    /// When `poll` looks at `mods.toml` again.
    config_check: Option<Instant>,
    /// The device's client settings (API 0.3.4), when this is a client host with a settings file.
    client_config: ModsConfig,
    client_settings: Option<PathBuf>,
    /// Mods whose settings changed since [`ModHost::take_changed_settings`].
    changed_settings: Vec<String>,
}

/// What a reload did to one mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Stopped and started again from the package on disk (old version, new version).
    Reloaded(String, String),
    /// The package on disk is the one that runs (only reported by a reload of everything).
    Unchanged,
    /// A package that was not loaded before now runs (its version).
    Added(String),
    /// The package is gone (or no longer loadable): the mod was stopped.
    Removed(String),
    /// Nothing changed; the text says why. A running mod keeps running when its new package cannot start.
    Failed(String),
}

/// The result for one mod of [`ModHost::reload`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadResult {
    /// Mod id (the file name for a package that could not be read).
    pub id: String,
    /// What happened.
    pub outcome: ReloadOutcome,
}

impl fmt::Display for ReloadResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = &self.id;
        match &self.outcome {
            ReloadOutcome::Reloaded(a, b) if a == b => write!(f, "{id}: reloaded ({a})"),
            ReloadOutcome::Reloaded(a, b) => write!(f, "{id}: reloaded ({a} -> {b})"),
            ReloadOutcome::Unchanged => write!(f, "{id}: unchanged"),
            ReloadOutcome::Added(v) => write!(f, "{id}: loaded ({v})"),
            ReloadOutcome::Removed(why) => write!(f, "{id}: stopped ({why})"),
            // A package that could not even be read has no id: the text names the file.
            ReloadOutcome::Failed(why) if id == "-" => f.write_str(why),
            ReloadOutcome::Failed(why) => write!(f, "{id}: not reloaded: {why}"),
        }
    }
}

/// A player change the host reports with [`ModHost::player_event`] (API 0.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEvent {
    /// `player:sleep`.
    Sleep,
    /// `player:wake`.
    Wake,
    /// `player:respawn`.
    Respawn,
}

impl PlayerEvent {
    /// The event name after `player:`.
    pub fn name(self) -> &'static str {
        match self {
            PlayerEvent::Sleep => "sleep",
            PlayerEvent::Wake => "wake",
            PlayerEvent::Respawn => "respawn",
        }
    }
}

/// The reason shown for a mod whose package vanished.
const GONE: &str = "its package is gone or no longer loadable";

fn fingerprint(pkg: &Package) -> u64 {
    let mut h = DefaultHasher::new();
    pkg.sha256.hash(&mut h);
    format!("{:?}", pkg.manifest).hash(&mut h);
    pkg.files.hash(&mut h);
    h.finish()
}

fn panic_text(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

/// Splits a command line (`/name a b`) into the lower-cased name and the arguments.
pub fn split_command(line: &str) -> Option<(String, Vec<String>)> {
    let rest = line.trim_end_matches(['\n', '\r']).strip_prefix('/')?;
    let mut parts = rest.split(' ').filter(|s| !s.is_empty());
    let name = parts.next()?.to_lowercase();
    Some((name, parts.map(str::to_string).collect()))
}

impl ModHost {
    /// Discovers, resolves and instantiates the mods in `options.mods_dir`. Problems are logged through the bridge;
    /// a mods directory that does not exist gives an empty host and logs nothing.
    pub fn load(options: HostOptions, bridge: Rc<dyn GameBridge>) -> ModHost {
        let HostOptions { mods_dir, side, budgets, runtimes, storage, clock, remote, client_settings } = options;
        let client_settings = client_settings.filter(|_| side == Side::Client);
        let mut host = ModHost {
            slots: Vec::new(),
            bus: EventBus::default(),
            commands: CommandRegistry::default(),
            scheduler: Scheduler::default(),
            services: ServiceRegistry::default(),
            channels: Channels::default(),
            handles: Handles::default(),
            ui: UiState::default(),
            input: InputState::default(),
            remote: Remote::new(remote),
            stack: Vec::new(),
            deferred: Vec::new(),
            draining: false,
            bridge,
            side,
            budgets,
            storage,
            clock,
            enabled: false,
            tick: 0,
            mods_dir,
            runtimes,
            world: None,
            config: ModsConfig::default(),
            config_stamp: 0,
            config_check: None,
            client_config: ModsConfig::default(),
            client_settings,
            changed_settings: Vec::new(),
        };
        if let Some(p) = host.client_settings.clone() {
            host.client_config = host.read_config_file(&p, "client settings are not read").unwrap_or_default();
        }
        let (ordered, problems, skipped) = host.scan();
        for p in problems {
            host.log(LogLevel::Error, &p);
        }
        for s in skipped {
            host.log(LogLevel::Debug, &format!("skipping {s}"));
        }
        for pkg in ordered {
            if let Err(e) = host.add_slot(&pkg) {
                host.log(LogLevel::Error, &format!("cannot start {}: {e}", pkg.manifest.id));
            }
        }
        if !host.slots.is_empty() {
            let names: Vec<String> = host.slots.iter().map(|s| format!("{} {}", s.info.id, s.info.version)).collect();
            host.log(LogLevel::Info, &format!("loaded {} mod(s): {}", names.len(), names.join(", ")));
        }
        host
    }

    /// Discovers and resolves the packages in the mods directory: the packages in load order, the problems (as log
    /// lines) and the skipped ones.
    fn scan(&mut self) -> (Vec<Package>, Vec<String>, Vec<String>) {
        self.config_stamp = cfg::stamp(&self.mods_dir);
        let config = match self.read_config(&self.mods_dir) {
            Some(c) => {
                self.config = c.clone();
                c
            }
            None => ModsConfig::default(),
        };
        let (packages, found) = discover(&self.mods_dir);
        let mut problems: Vec<String> =
            found.iter().map(|p| format!("cannot load {}: {}", p.origin.display(), p.message)).collect();
        let kinds: Vec<_> = self.runtimes.iter().map(|r| r.kind()).collect();
        let (ordered, bad, skipped) = resolve(packages, &config, self.side, &kinds);
        problems.extend(bad.iter().map(|p| format!("not loading {}: {}", p.origin.display(), p.message)));
        (ordered, problems, skipped)
    }

    /// Instantiates a package on this side (the old state of a mod is not touched).
    fn instantiate(&self, pkg: &Package) -> Result<Box<dyn Instance>, String> {
        let files = pkg.side_files(self.side).expect("resolve keeps only mods with an entry on this side");
        let entry = match self.side {
            Side::Server => &pkg.manifest.entry.server,
            Side::Client => &pkg.manifest.entry.client,
        };
        let kind = entry.as_ref().map(|e| e.runtime);
        let rt = self.runtimes.iter().find(|r| Some(r.kind()) == kind).expect("resolve checked the runtime");
        rt.instantiate(&pkg.manifest.id, &files, InstanceLimits { memory_bytes: self.budgets.memory_bytes })
    }

    /// A slot in state `Loaded` for a package and its instance.
    fn make_slot(&self, pkg: &Package, instance: Box<dyn Instance>, generation: u32) -> Slot {
        let id = &pkg.manifest.id;
        let values = self.resolve_settings(id, &pkg.manifest.settings);
        Slot {
            info: ModInfo {
                id: pkg.manifest.id.clone(),
                version: pkg.manifest.version.clone(),
                origin: pkg.origin.clone(),
                sha256: pkg.sha256.clone(),
                state: ModState::Loaded,
            },
            permissions: required_permissions(&pkg.manifest.permissions),
            deps: pkg.manifest.dependencies.keys().chain(pkg.manifest.soft_dependencies.keys()).cloned().collect(),
            provides: pkg.manifest.provides.services.iter().map(|s| (s.name.clone(), s.version.clone())).collect(),
            cosmetics: pkg.manifest.content.cosmetics.iter().map(|c| (c.key.clone(), c.slot.clone())).collect(),
            items: pkg.manifest.content.items.iter().map(|i| i.key.clone()).collect(),
            blocks: pkg.manifest.content.blocks.iter().map(|b| (b.key.clone(), b.base.is_some())).collect(),
            remote: pkg
                .manifest
                .provides
                .services
                .iter()
                .filter(|s| s.remote)
                .map(|s| (s.name.clone(), s.remote_methods.clone()))
                .collect(),
            instance: Some(instance),
            store: None,
            strikes: Strikes::default(),
            timings: Timings::new(self.clock.now()),
            generation,
            fingerprint: fingerprint(pkg),
            settings: Rc::new(pkg.manifest.settings.clone()),
            values,
            client_required: crate::config::client_required(&pkg.manifest, &self.config.get(id)),
            warm: false,
        }
    }

    /// The values of `settings` from the stored ones; ignored stored values are logged.
    fn resolve_settings(&self, id: &str, settings: &Settings) -> Map<String, Json> {
        let stored = match &self.client_settings {
            // A client reads only its own (client-scope) keys from the device; server keys keep their default.
            Some(_) => {
                let mut m = self.client_config.get(id).settings_json();
                m.retain(|k, _| settings.get(k).is_none_or(|f| f.scope == Scope::Client));
                m
            }
            None => self.config.get(id).settings_json(),
        };
        let (values, problems) = settings.resolve(&stored);
        for p in problems {
            let msg = match p {
                SettingsError::UnknownKey(k) => format!("{FILE_NAME}: ignoring unknown setting `{k}`"),
                other => format!("{FILE_NAME}: invalid setting {other}; using the default"),
            };
            self.bridge.log(Some(id), LogLevel::Warn, &msg);
        }
        values
    }

    /// Instantiates a package and appends it as a new slot (state `Loaded`).
    fn add_slot(&mut self, pkg: &Package) -> Result<usize, String> {
        let instance = self.instantiate(pkg)?;
        let slot = self.make_slot(pkg, instance, 1);
        self.slots.push(slot);
        Ok(self.slots.len() - 1)
    }

    /// Reads `mods.toml` (a missing file is empty); `None` (logged) when it does not parse.
    fn read_config(&self, dir: &Path) -> Option<ModsConfig> {
        self.read_config_file(&dir.join(FILE_NAME), "no permissions are granted (settings stay)")
    }

    fn read_config_file(&self, path: &Path, consequence: &str) -> Option<ModsConfig> {
        let parsed = match std::fs::read_to_string(path) {
            Ok(text) => ModsConfig::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(ModsConfig::default()),
            Err(e) => Err(e.to_string()),
        };
        match parsed {
            Ok(c) => Some(c),
            Err(e) => {
                let p = path.display();
                self.log(LogLevel::Error, &format!("{p} is invalid, {consequence}: {e}"));
                None
            }
        }
    }

    fn log(&self, level: LogLevel, msg: &str) {
        self.bridge.log(None, level, msg);
    }

    /// True when no mod was loaded (the host program should then behave exactly like vanilla).
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Every loaded mod in load order.
    pub fn mods(&self) -> Vec<ModInfo> {
        self.slots.iter().map(|s| s.info.clone()).collect()
    }

    /// True while `owner`'s code is running (it is on the call stack).
    fn on_stack(&self, owner: usize) -> bool {
        self.stack.iter().any(|(o, _)| *o == owner)
    }

    /// Runs one guest call for `owner` under the budget. `None` when the mod has no instance (disabled) or is
    /// already on the call stack.
    fn call<R>(
        &mut self,
        owner: usize,
        kind: HandlerKind,
        what: &str,
        f: impl FnOnce(&mut dyn Instance, &mut dyn HostApi) -> Result<R, CallError>,
    ) -> Option<Result<R, CallError>> {
        // A mod on the call stack has its instance taken out, so it cannot be entered twice.
        let mut instance = self.slots[owner].instance.take()?;
        let start = self.clock.now();
        let cold = kind == HandlerKind::Lifecycle || !self.slots[owner].warm;
        let own = start + self.budgets.hard_for(cold);
        let deadline = self.stack.last().map_or(own, |&(_, outer)| own.min(outer));
        self.stack.push((owner, deadline));
        let result = {
            let mut ctx = Ctx { host: self, owner, deadline };
            catch_unwind(AssertUnwindSafe(|| f(instance.as_mut(), &mut ctx)))
                .unwrap_or_else(|p| Err(CallError::Trap(format!("panic: {}", panic_text(p)))))
        };
        self.stack.pop();
        let end = self.clock.now();
        let elapsed = end.saturating_duration_since(start);
        let slot = &mut self.slots[owner];
        let id = slot.info.id.clone();
        slot.instance = Some(instance);
        slot.warm = true;
        slot.timings.record(kind, start, elapsed, self.budgets.soft, result.as_ref().err());
        slot.timings.prune(end, self.budgets.window);
        if elapsed > self.budgets.soft {
            self.bridge.log(
                Some(&id),
                LogLevel::Warn,
                &format!(
                    "{what} took {:.1} ms (budget {} ms)",
                    elapsed.as_secs_f64() * 1000.0,
                    self.budgets.soft.as_millis()
                ),
            );
        }
        if let Err(e @ (CallError::Trap(_) | CallError::Timeout)) = &result {
            self.bridge.log(Some(&id), LogLevel::Error, &format!("{what}: {e}"));
            let slot = &mut self.slots[owner];
            slot.timings.strike();
            if !matches!(slot.info.state, ModState::Disabled(_)) && slot.strikes.record(self.clock.now(), &self.budgets)
            {
                let n = self.budgets.strikes;
                let secs = self.budgets.window.as_secs();
                self.disable_slot(owner, format!("{n} faults within {secs} s"));
            }
        }
        if self.stack.is_empty() && !self.deferred.is_empty() && !self.draining {
            self.drain_deferred();
        }
        Some(result)
    }

    /// Delivers the queued deliveries (outcomes ignored). Deliveries queued meanwhile run too, up to a limit.
    fn drain_deferred(&mut self) {
        self.draining = true;
        let mut budget = MAX_DEFERRED;
        while !self.deferred.is_empty() && budget > 0 {
            let batch = std::mem::take(&mut self.deferred);
            for (sub, ev) in batch {
                budget = budget.saturating_sub(1);
                // A mod disabled meanwhile has no instance, so `call` skips it.
                let what = format!("{} handler", ev.kind().name());
                self.call(sub.owner, HandlerKind::Event, &what, |i, h| i.handle_event(h, sub.id, &ev));
            }
        }
        if !self.deferred.is_empty() {
            self.log(LogLevel::Warn, &format!("dropped {} deferred event deliveries (loop?)", self.deferred.len()));
            self.deferred.clear();
        }
        self.draining = false;
    }

    /// Disables one mod: `on-disable`, then everything it registered is removed and its storage flushed.
    fn disable_slot(&mut self, owner: usize, reason: String) {
        if matches!(self.slots[owner].info.state, ModState::Disabled(_)) {
            return;
        }
        let was_enabled = self.slots[owner].info.state == ModState::Enabled;
        self.slots[owner].info.state = ModState::Disabled(reason.clone());
        if was_enabled {
            if let Some(Err(e)) = self.call(owner, HandlerKind::Lifecycle, "on-disable", |i, h| i.on_disable(h)) {
                let id = self.slots[owner].info.id.clone();
                self.bridge.log(Some(&id), LogLevel::Warn, &format!("on-disable failed: {e}"));
            }
        }
        self.bus.remove_owner(owner);
        self.commands.remove_owner(owner);
        self.scheduler.remove_owner(owner);
        self.services.remove_owner(owner);
        self.channels.remove_owner(owner);
        self.remote.remove_owner(owner);
        if self.ui.clear_owner(owner) {
            let id = self.slots[owner].info.id.clone();
            // The loader drops the mod's widgets and dialogs; a bridge without UI has nothing to drop.
            let _ = self.bridge.ui(&id, &UiCommand::ClearMod);
        }
        if self.input.clear_owner(owner) {
            let id = self.slots[owner].info.id.clone();
            let _ = self.bridge.input(&id, &InputCommand::ClearMod);
        }
        let slot = &mut self.slots[owner];
        slot.instance = None;
        let id = slot.info.id.clone();
        if let Some(mut store) = slot.store.take() {
            if let Err(e) = store.flush() {
                self.bridge.log(Some(&id), LogLevel::Error, &format!("storage flush failed: {e}"));
            }
        }
        self.bridge.log(Some(&id), LogLevel::Info, &format!("disabled: {reason}"));
    }

    /// The world is loaded: opens each mod's storage, calls `on-enable` in load order, then fires `world-loaded`.
    pub fn enable(&mut self, world_name: &str, world_id: &str) {
        if self.enabled {
            return;
        }
        self.enabled = true;
        self.world = Some((world_name.to_string(), world_id.to_string()));
        for i in 0..self.slots.len() {
            self.enable_slot(i);
        }
        self.fire(Event::WorldLoaded { world_name: world_name.to_string(), world_id: world_id.to_string() });
    }

    /// Opens the storage of a `Loaded` mod and calls its `on-enable` (a failure disables it).
    fn enable_slot(&mut self, i: usize) {
        if self.slots[i].info.state != ModState::Loaded {
            return;
        }
        let id = self.slots[i].info.id.clone();
        match self.storage.open(&id).and_then(Store::open) {
            Ok(s) => self.slots[i].store = Some(s),
            Err(e) => self.bridge.log(Some(&id), LogLevel::Error, &format!("storage unavailable: {e}")),
        }
        self.slots[i].info.state = ModState::Enabled;
        match self.call(i, HandlerKind::Lifecycle, "on-enable", |inst, h| inst.on_enable(h)).unwrap_or(Ok(())) {
            Ok(()) => self.bridge.log(Some(&id), LogLevel::Info, "enabled"),
            Err(e) => {
                self.slots[i].info.state = ModState::Loaded;
                self.disable_slot(i, format!("on-enable failed: {e}"));
            }
        }
    }

    /// Hot reload (admin command `/mods reload [id]`): reads the packages in the mods directory again and restarts
    /// the changed mods without touching the world.
    ///
    /// - With an `id`, that mod is reloaded whether its package changed or not; a mod that is not loaded yet but has
    ///   a package is started; a mod whose package is gone is stopped. Without an `id`, every mod whose package
    ///   changed is reloaded, new packages are started and mods whose package vanished are stopped.
    /// - A reload is `on-disable`, then everything the mod registered is removed (handlers, commands, scheduled
    ///   tasks, services, channels, remote calls, UI), its storage flushed, and the new package is started as at
    ///   boot: storage is kept, `on-enable` runs, then `world-loaded` is delivered to that mod alone. Players who
    ///   are online are not announced again (use `players.online()` in `on-enable`).
    /// - A new package that cannot start leaves the old mod running. Budgets, strike counters and timings of the mod
    ///   start from zero.
    /// - Services other mods got from `services.get` before the reload are stale: calling them gives a clear error
    ///   (the proxy has to be fetched again). Calls by name keep working with the new instance.
    pub fn reload(&mut self, only: Option<&str>) -> Vec<ReloadResult> {
        let (ordered, problems, skipped) = self.scan();
        let mut results = Vec::new();
        let mut present: Vec<String> = Vec::new();
        for pkg in ordered {
            let id = pkg.manifest.id.clone();
            present.push(id.clone());
            if only.is_some_and(|o| o != id) {
                continue;
            }
            let outcome = match self.slots.iter().position(|s| s.info.id == id) {
                Some(i) if only.is_none() && self.slots[i].fingerprint == fingerprint(&pkg) => ReloadOutcome::Unchanged,
                Some(i) => self.replace_slot(i, &pkg),
                None => self.start_new(&pkg),
            };
            results.push(ReloadResult { id, outcome });
        }
        for i in 0..self.slots.len() {
            let id = self.slots[i].info.id.clone();
            if present.contains(&id) || only.is_some_and(|o| o != id) {
                continue;
            }
            if self.slots[i].info.state == ModState::Disabled(GONE.to_string()) {
                continue;
            }
            let why = skipped.iter().find_map(|s| s.strip_prefix(&format!("{id}: "))).unwrap_or(GONE);
            self.disable_slot(i, GONE.to_string());
            results.push(ReloadResult { id, outcome: ReloadOutcome::Removed(why.to_string()) });
        }
        match only {
            Some(o) if !results.iter().any(|r| r.id == o) => {
                let mut why = format!("no package for {o} in the mods directory");
                if let Some(p) = problems.iter().find(|p| p.contains(o)) {
                    why = p.clone();
                }
                results.push(ReloadResult { id: o.to_string(), outcome: ReloadOutcome::Failed(why) });
            }
            Some(_) => {}
            None => results.extend(
                problems.into_iter().map(|p| ReloadResult { id: "-".to_string(), outcome: ReloadOutcome::Failed(p) }),
            ),
        }
        let changed = results.iter().filter(|r| !matches!(r.outcome, ReloadOutcome::Unchanged)).count();
        if changed > 0 {
            self.log(LogLevel::Info, &format!("reload: {changed} change(s)"));
        }
        results
    }

    /// Restarts the mod in slot `i` from `pkg`.
    fn replace_slot(&mut self, i: usize, pkg: &Package) -> ReloadOutcome {
        let instance = match self.instantiate(pkg) {
            Ok(inst) => inst,
            Err(e) => return ReloadOutcome::Failed(format!("cannot start: {e} (the running version stays)")),
        };
        let old = self.slots[i].info.version.clone();
        self.disable_slot(i, "reloading".to_string());
        let generation = self.slots[i].generation.wrapping_add(1).max(1);
        self.slots[i] = self.make_slot(pkg, instance, generation);
        self.start_slot(i).unwrap_or(ReloadOutcome::Reloaded(old, pkg.manifest.version.clone()))
    }

    /// Starts a package that was not loaded before.
    fn start_new(&mut self, pkg: &Package) -> ReloadOutcome {
        match self.add_slot(pkg) {
            Ok(i) => self.start_slot(i).unwrap_or(ReloadOutcome::Added(pkg.manifest.version.clone())),
            Err(e) => ReloadOutcome::Failed(format!("cannot start: {e}")),
        }
    }

    /// Enables a slot that was added or replaced after the world was loaded, like at boot. `Some(failure)` when the
    /// mod ended up disabled (its `on-enable` failed).
    fn start_slot(&mut self, i: usize) -> Option<ReloadOutcome> {
        let (world_name, world_id) = self.world.clone()?;
        self.enable_slot(i);
        self.fire_filtered(Event::WorldLoaded { world_name, world_id }, Some(&[i]));
        match &self.slots[i].info.state {
            ModState::Disabled(why) => Some(ReloadOutcome::Failed(format!("the new version is disabled: {why}"))),
            _ => None,
        }
    }

    /// The time accounting of every mod over the rolling window (`budgets.window`), in load order. `only` limits it
    /// to one mod.
    pub fn timings(&self, only: Option<&str>) -> Vec<ModTimings> {
        let now = self.clock.now();
        self.slots
            .iter()
            .filter(|s| only.is_none_or(|o| o == s.info.id))
            .map(|s| ModTimings {
                id: s.info.id.clone(),
                state: match &s.info.state {
                    ModState::Loaded => "loaded".to_string(),
                    ModState::Enabled => "enabled".to_string(),
                    ModState::Disabled(why) => format!("disabled: {why}"),
                },
                window: self.budgets.window,
                kinds: s.timings.snapshot(now, self.budgets.window),
                strikes: s.strikes.count(),
                strikes_total: s.timings.strikes_total(),
            })
            .collect()
    }

    /// Clears the time accounting of every mod (or of `only`); false when `only` names no mod. Strikes that count
    /// towards disabling a mod are not forgiven, only the report is cleared.
    pub fn reset_timings(&mut self, only: Option<&str>) -> bool {
        let now = self.clock.now();
        let mut found = only.is_none();
        for s in self.slots.iter_mut().filter(|s| only.is_none_or(|o| o == s.info.id)) {
            s.timings = Timings::new(now);
            found = true;
        }
        found
    }

    fn slot_of(&self, id: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.info.id == id)
    }

    /// The settings of mod `id` as canonical JSON `{ "fields": [...], "values": {...} }` (what the host app and
    /// 0xBC `SETTINGS_SCHEMA` show). `None` for a mod that is not loaded.
    pub fn settings_schema(&self, id: &str) -> Option<Json> {
        let s = &self.slots[self.slot_of(id)?];
        Some(s.settings.to_json(&s.values))
    }

    /// The settings form of mod `id` restricted to one scope (API 0.3.4): `({"fields": [...]}, {key: value})`, the
    /// `schema` and `values` of 0xBC `SETTINGS_SCHEMA` for `Scope::Server`, the loader's own form for
    /// `Scope::Client`. `None` when the mod is not loaded or declares no setting of that scope.
    pub fn settings_form(&self, id: &str, scope: Scope) -> Option<(Json, Json)> {
        let s = &self.slots[self.slot_of(id)?];
        let mut only = (*s.settings).clone();
        only.fields.retain(|f| f.scope == scope);
        if only.fields.is_empty() {
            return None;
        }
        let mut form = only.to_json(&s.values);
        let values = form.as_object_mut().and_then(|o| o.remove("values")).unwrap_or_default();
        Some((form, values))
    }

    /// The ids of the mods whose setting values changed since the last call (any source: `/mods set`, the host app,
    /// a mod, `mods.toml` edits, 0xBC `SETTINGS_SET`), in order, without repeats. The server uses it to send admins
    /// fresh `SETTINGS_SCHEMA`s ([`crate::netsettings`]).
    pub fn take_changed_settings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.changed_settings)
    }

    /// [`ModHost::settings_schema`] as canonical CBOR (the `schema` byte string of 0xBC `SETTINGS_SCHEMA`).
    pub fn settings_schema_cbor(&self, id: &str) -> Option<Vec<u8>> {
        self.settings_schema(id).map(|j| cfg::to_cbor(&j))
    }

    /// [`ModHost::set_setting`] with the value as canonical CBOR (the `value` byte string of 0xBC `SETTINGS_SET`;
    /// bytes that do not decode break the `type` rule). The sender's rights are the host program's job.
    pub fn set_setting_cbor(&mut self, id: &str, key: &str, value: &[u8]) -> Result<Json, SettingsError> {
        let j = cfg::from_cbor(value).map_err(|e| SettingsError::Invalid {
            path: key.to_string(),
            rule: "type",
            message: e.to_string(),
            expected: None,
            got: None,
        })?;
        self.set_setting(id, key, &j)
    }

    /// The current setting values of mod `id` (every key). `None` for a mod that is not loaded.
    pub fn settings_values(&self, id: &str) -> Option<Map<String, Json>> {
        self.slot_of(id).map(|i| self.slots[i].values.clone())
    }

    /// Whether players must run the client half of mod `id`: the manifest, overridden by the admin's
    /// `client-required` in `mods.toml` (never for mods with `[content]`). `None` for a mod that is not loaded.
    pub fn client_required(&self, id: &str) -> Option<bool> {
        self.slot_of(id).map(|i| self.slots[i].client_required)
    }

    /// The admin changes a setting of mod `id` (host app, `/mods set`, 0xBC `SETTINGS_SET` after the host program
    /// checked the sender's rights). The value is validated and normalised, stored in memory and in `mods.toml`
    /// (a write failure is logged; the new value still applies until the next restart), and
    /// `<mod-id>:settings-changed` fires when it differs from the old one. `UnknownKey` also for an unknown mod.
    pub fn set_setting(&mut self, id: &str, key: &str, value: &Json) -> Result<Json, SettingsError> {
        let i = self.slot_of(id).ok_or_else(|| SettingsError::UnknownKey(format!("{id}.{key}")))?;
        self.set_in_slot(i, key, value)
    }

    fn set_in_slot(&mut self, i: usize, key: &str, value: &Json) -> Result<Json, SettingsError> {
        let slot = &self.slots[i];
        let field = slot.settings.get(key).ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        if self.client_settings.is_some() && field.scope != Scope::Client {
            return Err(SettingsError::Invalid {
                path: key.to_string(),
                rule: "scope",
                message: "a server setting: only the server admin changes it".into(),
                expected: Some("client".into()),
                got: Some("server".into()),
            });
        }
        let v = bhmod_manifest::settings::validate_value(field, value)?;
        let stored = (v != field.default).then_some(&v);
        let id = slot.info.id.clone();
        let (file, config) = match &self.client_settings {
            Some(p) => (p.clone(), &mut self.client_config),
            None => (self.mods_dir.join(FILE_NAME), &mut self.config),
        };
        if let Err(e) = crate::config::write_setting_at(&file, &id, key, stored) {
            self.bridge.log(Some(&id), LogLevel::Error, &format!("setting {key} not saved: {e}"));
        }
        let entry = config.mods.entry(id).or_default();
        match stored.and_then(crate::config::json_to_toml_value) {
            Some(t) => {
                entry.settings.insert(key.to_string(), t);
            }
            None => {
                entry.settings.remove(key);
            }
        }
        self.config_stamp = cfg::stamp(&self.mods_dir);
        let old = self.slots[i].values.insert(key.to_string(), v.clone());
        if old.as_ref() != Some(&v) {
            self.settings_changed(i, vec![key.to_string()]);
        }
        Ok(v)
    }

    /// Fires `<mod-id>:settings-changed` (to every mod subscribed to that name; the owner on the stack gets it once
    /// its call returned).
    fn settings_changed(&mut self, i: usize, keys: Vec<String>) {
        let source = self.slots[i].info.id.clone();
        if !self.changed_settings.contains(&source) {
            self.changed_settings.push(source.clone());
        }
        if !self.enabled {
            return;
        }
        let name = format!("{source}:settings-changed");
        self.fire(Event::Custom { name, source, payload: cfg::changed_payload(&keys), cancelled: false });
    }

    /// Reads `mods.toml` again and applies the setting values of every loaded mod (`poll` does this by itself
    /// when the file changed, at most every [`cfg::CHECK_INTERVAL`]). A file that does not parse changes nothing
    /// (logged); an invalid value falls back to the default (logged). Mods whose values changed get
    /// `<mod-id>:settings-changed`. `enabled`, `grant` and `client-required` need `/mods reload` (or a restart).
    /// Returns the ids of the mods whose settings changed.
    pub fn reload_settings(&mut self) -> Vec<String> {
        self.config_stamp = cfg::stamp(&self.mods_dir);
        let Some(config) = self.read_config(&self.mods_dir) else { return Vec::new() };
        self.config = config;
        let mut changed = Vec::new();
        for i in 0..self.slots.len() {
            let id = self.slots[i].info.id.clone();
            let settings = self.slots[i].settings.clone();
            let values = self.resolve_settings(&id, &settings);
            let keys: Vec<String> = values
                .iter()
                .filter(|(k, v)| self.slots[i].values.get(*k) != Some(*v))
                .map(|(k, _)| k.clone())
                .collect();
            if keys.is_empty() {
                continue;
            }
            self.slots[i].values = values;
            self.bridge.log(Some(&id), LogLevel::Info, &format!("settings changed: {}", keys.join(", ")));
            self.settings_changed(i, keys);
            changed.push(id);
        }
        changed
    }

    /// The settings subcommands of the host program's own `/mods` command: `settings <id>` lists a mod's settings,
    /// `set <id> <key> <value>` changes one (`args` are the words after `/mods`). `is_admin`: the sender is an admin
    /// or the console. Returns `None` when `args` is not one of these subcommands (the host program handles its own
    /// `/mods reload|list|timings`), otherwise the reply lines. The vanilla `/mod <player>` stays the game's.
    pub fn settings_command(&mut self, is_admin: bool, args: &[impl AsRef<str>]) -> Option<Vec<String>> {
        let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        if !matches!(args.first(), Some(&("settings" | "set"))) {
            return None;
        }
        if !is_admin {
            return Some(vec!["You do not have permission to change mod settings.".to_string()]);
        }
        Some(self.settings_lines(&args))
    }

    fn settings_lines(&mut self, args: &[&str]) -> Vec<String> {
        const USAGE: &str = "Usage: /mods settings <mod> | /mods set <mod> <key> <value>";
        match args {
            ["settings", id] => {
                let Some(i) = self.slot_of(id) else { return vec![format!("No mod {id} is loaded.")] };
                let s = &self.slots[i];
                if s.settings.fields.is_empty() {
                    return vec![format!("{id} has no settings.")];
                }
                let mut out = vec![format!("{id} settings:")];
                for f in &s.settings.fields {
                    let restart = if f.restart { " (restart)" } else { "" };
                    out.push(format!("{} = {}{restart}", f.key, cfg::show(&s.values[&f.key], 200)));
                }
                out
            }
            ["set", id, key, value @ ..] if !value.is_empty() => {
                let Some(i) = self.slot_of(id) else { return vec![format!("No mod {id} is loaded.")] };
                let Some(field) = self.slots[i].settings.get(key) else {
                    return vec![format!("{id} has no setting {key} (see /mods settings {id}).")];
                };
                let parsed = bhmod_manifest::settings::parse_input(field, &value.join(" "));
                match parsed.and_then(|v| self.set_in_slot(i, key, &v)) {
                    Ok(v) => {
                        let restart = if self.slots[i].settings.get(key).is_some_and(|f| f.restart) {
                            " It takes effect after a restart."
                        } else {
                            ""
                        };
                        vec![format!("{id}: {key} = {}.{restart}", cfg::show(&v, 200))]
                    }
                    Err(e) => vec![format!("Not changed: {e}")],
                }
            }
            _ => vec![USAGE.to_string()],
        }
    }

    /// Server stop: `on-disable` for every running mod in reverse load order, storage flushed.
    pub fn disable_all(&mut self) {
        for i in (0..self.slots.len()).rev() {
            self.disable_slot(i, "host stopping".to_string());
        }
        self.enabled = false;
    }

    /// True when the host is enabled and some mod listens to `kind` (lets the host program skip building events
    /// nobody wants, such as `block-changed` or `player-move`).
    pub fn wants(&self, kind: EventKind) -> bool {
        self.enabled && self.bus.has_listeners(kind)
    }

    /// Delivers an event to every subscriber and returns it with the handlers' changes applied. Handlers of mods
    /// that are on the call stack get the event later (see the module docs). After a `player-quit`, the player's
    /// handle becomes stale.
    pub fn fire(&mut self, ev: Event) -> Event {
        self.fire_filtered(ev, None)
    }

    /// [`ModHost::fire`] for the handlers of one mod (`only`: its slot index) instead of every subscriber.
    fn fire_filtered(&mut self, mut ev: Event, only: Option<&[usize]>) -> Event {
        let kind = ev.kind();
        let custom = match &ev {
            Event::Custom { name, .. } => Some(name.clone()),
            _ => None,
        };
        for sub in self.bus.plan(kind, custom.as_deref()) {
            if only.is_some_and(|o| !o.contains(&sub.owner)) {
                continue;
            }
            if self.slots[sub.owner].info.state != ModState::Enabled || (sub.ignore_cancelled && ev.cancelled()) {
                continue;
            }
            if self.on_stack(sub.owner) {
                self.deferred.push((sub, ev.clone()));
                continue;
            }
            let snapshot = ev.clone();
            let what = format!("{} handler", custom.as_deref().unwrap_or(kind.name()));
            let result = self.call(sub.owner, HandlerKind::Event, &what, |i, h| i.handle_event(h, sub.id, &snapshot));
            if let Some(Ok(outcome)) = result {
                if sub.priority != Priority::Monitor && !ev.apply(outcome) {
                    let id = self.slots[sub.owner].info.id.clone();
                    self.bridge.log(
                        Some(&id),
                        LogLevel::Warn,
                        &format!("ignored a change that does not fit {}", kind.name()),
                    );
                }
            }
        }
        if let Event::PlayerQuit { player, .. } = &ev {
            self.handles.invalidate(player);
            self.remote_peer_gone(&RemoteTarget::Player(player.clone()));
        }
        ev
    }

    /// Delivers the answer of a remote call to the mod that made it: the custom event `<mod-id>:service-result`
    /// (see `services.wit`). A mod that is not enabled gets nothing (`fire_filtered` skips it, and a mod
    /// disabled meanwhile has no open calls left); a mod on the call stack gets it once its call has returned.
    fn deliver_remote(&mut self, owner: usize, id: u32, outcome: Result<Vec<u8>, RemoteError>) {
        let source = self.slots[owner].info.id.clone();
        let payload = remote::result_payload(id, &outcome).encode();
        let ev = Event::Custom { name: format!("{source}:service-result"), source, payload, cancelled: false };
        self.fire_filtered(ev, Some(&[owner]));
    }

    /// Fails the calls that time out now with `timeout`.
    fn expire_remote(&mut self) {
        let now = self.clock.now();
        for (id, p) in self.remote.expired(now) {
            self.deliver_remote(p.owner, id, Err(RemoteError::Timeout));
        }
    }

    /// The answer to a remote call this side made (0xBC `SERVICE_RESULT`) arrived from `from`. Delivered to the
    /// calling mod as the `service-result` event. False when no such call is open for that peer (late answer after a
    /// timeout, a cancelled call, or a peer answering a call that was sent to someone else).
    pub fn remote_result(&mut self, from: &RemoteTarget, id: u32, outcome: Result<Vec<u8>, RemoteError>) -> bool {
        let Some(p) = self.remote.answer(id, from) else { return false };
        let outcome = outcome.and_then(|bytes| {
            if bytes.len() > remote::MAX_PAYLOAD {
                return Err(RemoteError::TooLarge);
            }
            Value::decode(&bytes).map(|_| bytes).map_err(|e| RemoteError::BadValue(format!("result: {e}")))
        });
        self.deliver_remote(p.owner, id, outcome);
        true
    }

    /// The peer is gone (the player quit, or the client lost the server): its open calls fail with `disconnected`.
    /// A `player-quit` event does this by itself.
    pub fn remote_peer_gone(&mut self, peer: &RemoteTarget) {
        for (id, p) in self.remote.gone(peer) {
            self.deliver_remote(p.owner, id, Err(RemoteError::Disconnected));
        }
    }

    /// A remote call from the other side arrived (0xBC `SERVICE_CALL`, `from` is the player or the server and
    /// `caller_mod` the calling mod). Runs the provider's `call-service` under the provider's time budget and returns
    /// the result for the host program to send back as `SERVICE_RESULT`. Only services declared `remote = true` in
    /// the provider's `mod.toml` are reachable (and only the listed `remote-methods`), and every peer has a rate
    /// limit ([`RemoteLimits`]); the failures are those of the protocol's `service_error` codes.
    pub fn remote_call_in(
        &mut self,
        from: &RemoteTarget,
        caller_mod: &str,
        name: &str,
        version_req: &str,
        method: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, RemoteError> {
        if !self.enabled || !self.stack.is_empty() {
            return Err(RemoteError::Busy);
        }
        let now = self.clock.now();
        if !self.remote.allow(from, now) {
            return Err(RemoteError::Busy);
        }
        if args.len() > remote::MAX_PAYLOAD {
            return Err(RemoteError::TooLarge);
        }
        if !bhmod_manifest::rules::is_mod_id(caller_mod) || method.len() > remote::MAX_NAME {
            return Err(RemoteError::BadValue("caller mod or method name".into()));
        }
        match Value::decode(args) {
            Ok(Value::List(_)) => {}
            Ok(_) => return Err(RemoteError::BadValue("arguments must be a list".into())),
            Err(e) => return Err(RemoteError::BadValue(format!("arguments: {e}"))),
        }
        let provider = self.services.lookup(name, version_req)?.owner;
        let exposed = self.slots[provider]
            .remote
            .iter()
            .any(|(n, methods)| n == name && (methods.is_empty() || methods.iter().any(|m| m == method)));
        if !exposed {
            return Err(RemoteError::NotPermitted);
        }
        let player = match from {
            RemoteTarget::Player(p) => Some(p.clone()),
            RemoteTarget::Server => None,
        };
        self.remote.caller = Some(RemoteCaller { mod_id: caller_mod.to_string(), player });
        let what = format!("remote service {name}.{method}");
        let out = self.call(provider, HandlerKind::Service, &what, |i, h| i.call_service(h, name, method, args));
        self.remote.caller = None;
        match out {
            Some(Ok(bytes)) if bytes.len() > remote::MAX_PAYLOAD => Err(RemoteError::TooLarge),
            Some(Ok(bytes)) => match Value::decode(&bytes) {
                Ok(_) => Ok(bytes),
                Err(e) => Err(RemoteError::BadValue(format!("result: {e}"))),
            },
            // The error text goes to another machine: no stack traceback (script paths), at most 256 bytes.
            Some(Err(CallError::Failed(m))) => {
                let first = m.split("\nstack traceback:").next().unwrap_or_default();
                Err(RemoteError::ProviderError(truncate(first, remote::MAX_ERROR).to_string()))
            }
            // `None` cannot happen: the provider is enabled (it is registered) and not on the stack.
            _ => Err(RemoteError::Trap),
        }
    }

    /// The loader's answer to a dialog (API 0.3): `button` counts from 0 ([`crate::ui::CLOSED`] when the dialog went away
    /// without an answer), `text` is the field's text. Delivered as the custom event `<mod-id>:ui-action` with the
    /// payload `{"dialog", "button", "text"}`, to the mod that opened the dialog only. False for an unknown or
    /// already answered dialog.
    pub fn ui_result(&mut self, dialog: u32, button: u32, text: Option<String>) -> bool {
        let Some(owner) = self.ui.answered(dialog) else { return false };
        let id = self.slots[owner].info.id.clone();
        let payload = Value::Map(vec![
            ("dialog".into(), Value::Int(i64::from(dialog))),
            ("button".into(), Value::Int(i64::from(button))),
            ("text".into(), text.map_or(Value::Null, Value::Text)),
        ]);
        let ev =
            Event::Custom { name: format!("{id}:ui-action"), source: id, payload: payload.encode(), cancelled: false };
        self.fire_filtered(ev, Some(&[owner]));
        true
    }

    /// Something the voice host reports (API 0.3.4): fired as the custom event `voice:talk-start`, `voice:talk-stop`,
    /// `voice:state` or `voice:mic` to the mods holding the `voice` permission.
    pub fn voice_event(&mut self, event: &VoiceEvent) {
        let (name, payload) = event.custom();
        let to: Vec<usize> =
            (0..self.slots.len()).filter(|i| self.slots[*i].permissions.iter().any(|p| p == "voice")).collect();
        let source = crate::voice::EVENT_PREFIX.to_string();
        self.fire_filtered(Event::Custom { name, source, payload, cancelled: false }, Some(&to));
    }

    /// A player's blockhead fell asleep, woke up or respawned (API 0.3.4, servers): the custom event `player:sleep`,
    /// `player:wake` or `player:respawn` with `{"player": id}` to every subscribed mod.
    pub fn player_event(&mut self, event: PlayerEvent, player: &str) {
        let payload = Value::Map(vec![("player".into(), Value::text(player))]).encode();
        let name = format!("player:{}", event.name());
        self.fire(Event::Custom { name, source: "player".into(), payload, cancelled: false });
    }

    /// The player tapped or clicked a blockhead (API 0.3.4, clients): every mod gets its own custom event
    /// `<mod-id>:ui-pick` with `{"player": id}`.
    pub fn ui_pick(&mut self, player: &str) {
        let payload = Value::Map(vec![("player".into(), Value::text(player))]).encode();
        for i in 0..self.slots.len() {
            let id = self.slots[i].info.id.clone();
            let ev =
                Event::Custom { name: format!("{id}:ui-pick"), source: id, payload: payload.clone(), cancelled: false };
            self.fire_filtered(ev, Some(&[i]));
        }
    }

    /// The player used an on-screen button of mod `mod_id` (API 0.3.4): the custom event `<mod-id>:ui-button`
    /// `{"id", "action"}` to that mod only. False when the mod has no such button.
    pub fn ui_button(&mut self, mod_id: &str, id: &str, action: ButtonAction) -> bool {
        let Some(owner) = self.slot_of(mod_id).filter(|o| self.ui.has_button(*o, id)) else { return false };
        let payload = Value::Map(vec![("id".into(), Value::text(id)), ("action".into(), Value::text(action.name()))]);
        let ev = Event::Custom {
            name: format!("{mod_id}:ui-button"),
            source: mod_id.to_string(),
            payload: payload.encode(),
            cancelled: false,
        };
        self.fire_filtered(ev, Some(&[owner]));
        true
    }

    /// A bound key of mod `mod_id` went down or up (API 0.3.4): the custom event `<mod-id>:input`
    /// `{"action", "pressed"}` to that mod only. False when the mod has no such action.
    pub fn input_event(&mut self, mod_id: &str, action: &str, pressed: bool) -> bool {
        let Some(owner) = self.slot_of(mod_id).filter(|o| self.input.default_key(*o, action).is_some()) else {
            return false;
        };
        let payload =
            Value::Map(vec![("action".into(), Value::text(action)), ("pressed".into(), Value::Bool(pressed))]);
        let ev = Event::Custom {
            name: format!("{mod_id}:input"),
            source: mod_id.to_string(),
            payload: payload.encode(),
            cancelled: false,
        };
        self.fire_filtered(ev, Some(&[owner]));
        true
    }

    /// A channel message arrived from the network (milestone M5: 0xBC `CHANNEL`). It is delivered with the next
    /// poll to every mod that opened `name`. False when the queue is full.
    pub fn channel_receive(&mut self, name: &str, source: ChannelSource, data: Vec<u8>) -> bool {
        self.channels.enqueue(Message { name: name.to_string(), from_owner: None, source, data })
    }

    /// Every channel name some mod has open, sorted (milestone M5: the server maps them to 0xBC wire ids).
    pub fn channel_names(&self) -> Vec<String> {
        self.channels.names()
    }

    /// Delivers queued channel messages.
    fn deliver_channels(&mut self) {
        for (msg, to) in self.channels.drain() {
            for (owner, ch) in to {
                // A mod disabled meanwhile has no instance, so `call` skips it.
                self.call(owner, HandlerKind::Channel, "channel message", |i, h| {
                    i.on_channel_message(h, ch, &msg.source, &msg.data)
                });
            }
        }
    }

    /// True when `line` is `/name ...` and `name` is a mod command (so the host routes it to [`ModHost::command`]
    /// for any player).
    pub fn owns_command(&self, line: &str) -> bool {
        self.enabled && split_command(line).is_some_and(|(name, _)| self.commands.lookup(&name).is_some())
    }

    /// Handles a command line. Fires `command` first. Returns `None` when the host should run it as a built-in
    /// command, otherwise the reply lines for the sender (empty when a handler cancelled it).
    pub fn command(&mut self, sender: CommandSender, line: &str) -> Option<Vec<String>> {
        if !self.enabled {
            return None;
        }
        let (name, args) = split_command(line)?;
        if self.bus.has_listeners(EventKind::Command) {
            let raw = line.trim_end_matches(['\n', '\r']).to_string();
            let ev = Event::Command {
                sender: sender.clone(),
                name: name.clone(),
                args: args.clone(),
                raw,
                cancelled: false,
            };
            if self.fire(ev).cancelled() {
                return Some(Vec::new());
            }
        }
        let cmd = self.commands.lookup(&name)?.clone();
        let allowed = match &sender {
            CommandSender::Console => cmd.spec.console,
            CommandSender::Player(id) => self.bridge.player_role(id) >= cmd.spec.min_role,
        };
        if !allowed {
            return Some(vec![match sender {
                CommandSender::Console => format!("/{name} cannot be used from the console."),
                CommandSender::Player(_) => format!("You do not have permission to use /{name}."),
            }]);
        }
        let what = format!("command /{name}");
        match self.call(cmd.owner, HandlerKind::Command, &what, |i, h| i.run_command(h, cmd.id, &sender, &name, &args))
        {
            Some(Ok(CommandOutcome { handled: true, reply })) => Some(reply),
            Some(Ok(CommandOutcome { handled: false, mut reply })) => {
                reply.push(format!("Usage: {}", cmd.spec.usage));
                Some(reply)
            }
            _ => Some(vec![format!("An error occurred while running /{name}.")]),
        }
    }

    fn run_tasks(&mut self, tasks: Vec<(usize, TaskId)>, budget_start: Option<Instant>) {
        for (owner, id) in tasks {
            if let Some(start) = budget_start {
                if self.clock.now().saturating_duration_since(start) > self.budgets.step {
                    self.scheduler.postpone(owner, id);
                    continue;
                }
            }
            if self.slots[owner].info.state == ModState::Enabled {
                self.call(owner, HandlerKind::Task, "scheduled task", |i, h| i.run_task(h, id));
            }
        }
    }

    /// One world step: fires `tick`, then runs due tasks until the step budget is used up (the rest move to the
    /// next step).
    pub fn tick(&mut self, dt: f32) {
        if !self.enabled {
            return;
        }
        let start = self.clock.now();
        self.tick += 1;
        if self.bus.has_listeners(EventKind::Tick) {
            self.fire(Event::Tick { tick: self.tick, dt });
        }
        let due = self.scheduler.advance();
        self.run_tasks(due, Some(start));
        if self.channels.has_queued() {
            self.deliver_channels();
        }
        if self.remote.has_pending() {
            self.expire_remote();
        }
    }

    /// One network poll: runs next-poll tasks and delivers channel messages.
    pub fn poll(&mut self) {
        if !self.enabled {
            return;
        }
        // The real clock on purpose: looking at a file is not mod time, and the fake clocks of tests stay untouched.
        let now = Instant::now();
        if self.config_check.is_none_or(|t| now >= t) {
            self.config_check = Some(now + cfg::CHECK_INTERVAL);
            if cfg::stamp(&self.mods_dir) != self.config_stamp {
                self.reload_settings();
            }
        }
        if self.scheduler.has_poll() {
            let tasks = self.scheduler.take_poll();
            self.run_tasks(tasks, None);
        }
        if self.channels.has_queued() {
            self.deliver_channels();
        }
        if self.remote.has_pending() {
            self.expire_remote();
        }
    }

    /// The world is being saved. Storage is flushed after the `post` handlers.
    pub fn world_save(&mut self, phase: SavePhase) {
        if !self.enabled {
            return;
        }
        self.fire(Event::WorldSave { phase });
        if phase == SavePhase::Post {
            for s in &mut self.slots {
                if let Some(store) = &mut s.store {
                    if let Err(e) = store.flush() {
                        self.bridge.log(Some(&s.info.id), LogLevel::Error, &format!("storage flush failed: {e}"));
                    }
                }
            }
        }
    }
}

/// The host API of one mod during one call.
struct Ctx<'a> {
    host: &'a mut ModHost,
    owner: usize,
    deadline: Instant,
}

impl Ctx<'_> {
    fn slot(&self) -> &Slot {
        &self.host.slots[self.owner]
    }

    fn require(&self, perm: &str) -> Result<(), ApiError> {
        if self.slot().permissions.iter().any(|p| p == perm) {
            Ok(())
        } else {
            Err(ApiError::NotPermitted(perm.to_string()))
        }
    }

    fn store(&mut self) -> Result<&mut Store, ApiError> {
        self.host.slots[self.owner].store.as_mut().ok_or_else(|| ApiError::Internal("storage unavailable".into()))
    }

    fn log_as_mod(&self, level: LogLevel, msg: &str) {
        self.host.bridge.log(Some(&self.slot().info.id), level, msg);
    }

    /// Equipping is a server thing: clients get `unsupported`.
    fn require_content_side(&self) -> Result<(), ApiError> {
        if self.host.side == Side::Server {
            Ok(())
        } else {
            Err(ApiError::Unsupported)
        }
    }

    /// Why `key` cannot be equipped by the calling mod: another mod's key, or no such key.
    fn unknown_cosmetic(&self, key: &str) -> ApiError {
        let declared_elsewhere = self.host.slots.iter().any(|s| s.cosmetics.iter().any(|(k, _)| k == key));
        if declared_elsewhere {
            ApiError::NotPermitted(format!("`{key}` belongs to another mod"))
        } else {
            ApiError::InvalidArgument(format!("`{key}` is not a cosmetic declared by this mod"))
        }
    }

    /// The id of content `key` of `kind` that the calling mod declares itself (`not-permitted` for another mod's key,
    /// `invalid-argument` for an unknown one or one without an id in this world).
    fn own_content_id(&self, kind: ContentKind, key: &str) -> Result<u64, ApiError> {
        let declared = |slot: &Slot| match kind {
            ContentKind::Item => slot.items.iter().any(|k| k == key),
            _ => slot.blocks.iter().any(|(k, _)| k == key),
        };
        if !declared(self.slot()) {
            return Err(if self.host.slots.iter().any(declared) {
                ApiError::NotPermitted(format!("`{key}` belongs to another mod"))
            } else {
                ApiError::InvalidArgument(format!("`{key}` is not declared by this mod"))
            });
        }
        self.host
            .bridge
            .content_id(kind, key)
            .ok_or_else(|| ApiError::InvalidArgument(format!("`{key}` has no id in this world")))
    }

    /// True when the calling mod is `id` or depends (softly) on it.
    fn may_use(&self, id: &str) -> bool {
        self.slot().info.id == id || self.slot().deps.iter().any(|d| d == id)
    }
}

/// Most items one `content.give-item` call hands out.
const MAX_GIVE: u32 = 64;

fn limit(what: &str) -> ApiError {
    ApiError::LimitExceeded(what.to_string())
}

/// Truncates to at most `max` bytes on a char boundary.
pub fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl HostApi for Ctx<'_> {
    fn deadline(&self) -> Instant {
        self.deadline
    }
    fn api_version(&self) -> String {
        API_VERSION.to_string()
    }
    fn mod_id(&self) -> String {
        self.slot().info.id.clone()
    }
    fn mod_version(&self) -> String {
        self.slot().info.version.clone()
    }
    fn current_side(&self) -> Side {
        self.host.side
    }
    fn host_name(&self) -> String {
        self.host.bridge.host_name()
    }
    fn host_platform(&mut self) -> String {
        self.host.bridge.platform()
    }
    fn player_sleeping(&mut self, id: &str) -> bool {
        self.host.side == Side::Server && self.host.bridge.player_sleeping(id)
    }
    fn player_alive(&mut self, id: &str) -> bool {
        self.host.side == Side::Server && self.host.bridge.player_alive(id)
    }
    fn players_local(&mut self) -> Option<String> {
        self.host.bridge.local_player()
    }
    fn players_has_mod(&mut self, id: &str, mod_id: &str) -> bool {
        self.host.side == Side::Server && self.host.bridge.player_has_mod(id, mod_id)
    }
    fn terrain_world_width(&mut self) -> Option<i32> {
        self.host.bridge.world_width()
    }
    fn ui_has_touch(&mut self) -> bool {
        self.host.bridge.has_touch()
    }
    fn log(&mut self, level: LogLevel, message: &str) {
        self.log_as_mod(level, truncate(message, MAX_LOG));
    }
    fn player_alias(&mut self, id: &str) -> Result<String, ApiError> {
        self.host.bridge.player_alias(id).ok_or(ApiError::StaleHandle)
    }
    fn player_is_online(&mut self, id: &str) -> bool {
        self.host.bridge.player_alias(id).is_some()
    }
    fn player_role(&mut self, id: &str) -> Result<Role, ApiError> {
        self.host.bridge.player_alias(id).ok_or(ApiError::StaleHandle)?;
        Ok(self.host.bridge.player_role(id))
    }
    fn player_send_chat(&mut self, id: &str, text: &str) -> Result<(), ApiError> {
        self.host.bridge.send_chat(id, text).then_some(()).ok_or(ApiError::StaleHandle)
    }
    fn player_kick(&mut self, id: &str, reason: &str) -> Result<(), ApiError> {
        self.require("players:kick")?;
        self.host.bridge.kick(id, reason).then_some(()).ok_or(ApiError::StaleHandle)
    }
    fn players_get(&mut self, id: &str) -> Option<String> {
        self.host.bridge.player_alias(id).map(|_| id.to_string())
    }
    fn players_find_by_alias(&mut self, alias: &str) -> Option<String> {
        let want = alias.to_lowercase();
        let bridge = &self.host.bridge;
        bridge.online_players().into_iter().find(|id| bridge.player_alias(id).is_some_and(|a| a.to_lowercase() == want))
    }
    fn players_online(&mut self) -> Vec<String> {
        self.host.bridge.online_players()
    }
    fn players_broadcast(&mut self, text: &str) -> Result<(), ApiError> {
        self.require("players:chat")?;
        self.host.bridge.broadcast(text);
        Ok(())
    }
    fn subscribe(&mut self, kind: EventKind, priority: Priority, ignore: bool) -> Result<SubscriptionId, ApiError> {
        if kind == EventKind::Custom {
            return Err(ApiError::InvalidArgument("subscribe to custom events by name".into()));
        }
        self.host.bus.subscribe(self.owner, kind, priority, ignore).ok_or_else(|| limit("subscriptions per mod"))
    }
    fn unsubscribe(&mut self, sub: SubscriptionId) -> bool {
        self.host.bus.unsubscribe(self.owner, sub)
    }
    fn register_command(&mut self, spec: CommandSpec) -> Result<CommandId, ApiError> {
        let ModHost { commands, bridge, .. } = &mut *self.host;
        commands.register(self.owner, spec, &|n| bridge.is_builtin_command(n))
    }
    fn unregister_command(&mut self, id: CommandId) -> bool {
        self.host.commands.unregister(self.owner, id)
    }
    fn run_later(&mut self, delay: u32) -> Result<TaskId, ApiError> {
        self.host.scheduler.run_later(self.owner, delay).ok_or_else(|| limit("tasks per mod"))
    }
    fn run_every(&mut self, delay: u32, period: u32) -> Result<TaskId, ApiError> {
        if period == 0 {
            return Err(ApiError::InvalidArgument("period must be at least 1".into()));
        }
        self.host.scheduler.run_every(self.owner, delay, period).ok_or_else(|| limit("tasks per mod"))
    }
    fn run_next_poll(&mut self) -> Result<TaskId, ApiError> {
        self.host.scheduler.run_next_poll(self.owner).ok_or_else(|| limit("tasks per mod"))
    }
    fn cancel_task(&mut self, id: TaskId) -> bool {
        self.host.scheduler.cancel(self.owner, id)
    }
    fn storage_get(&mut self, key: &str) -> Option<Vec<u8>> {
        self.store().ok()?.get(key)
    }
    fn storage_set(&mut self, key: &str, value: &[u8]) -> Result<(), ApiError> {
        self.store()?.set(key, value)
    }
    fn storage_delete(&mut self, key: &str) -> bool {
        let Ok(store) = self.store() else { return false };
        match store.delete(key) {
            Ok(found) => found,
            Err(e) => {
                self.log_as_mod(LogLevel::Error, &format!("storage delete failed: {e}"));
                false
            }
        }
    }
    fn storage_keys(&mut self, prefix: &str) -> Vec<String> {
        self.store().map(|s| s.keys(prefix)).unwrap_or_default()
    }

    fn player_pos(&mut self, id: &str) -> Result<Option<Pos>, ApiError> {
        self.host.bridge.player_alias(id).ok_or(ApiError::StaleHandle)?;
        Ok(self.host.bridge.player_pos(id))
    }
    fn content_id(&mut self, kind: ContentKind, key: &str) -> Option<u64> {
        self.host.bridge.content_id(kind, key)
    }
    fn content_key(&mut self, kind: ContentKind, id: u64) -> Option<String> {
        self.host.bridge.content_key(kind, id)
    }
    fn content_cosmetics(&mut self) -> Vec<CosmeticInfo> {
        let bridge = &self.host.bridge;
        let mut out = Vec::new();
        for slot in &self.host.slots {
            for (key, cslot) in &slot.cosmetics {
                let id = bridge.content_id(ContentKind::Cosmetic, key);
                out.push(CosmeticInfo { key: key.clone(), mod_id: slot.info.id.clone(), slot: cslot.clone(), id });
            }
        }
        out
    }
    fn content_equipped(&mut self, id: &str) -> Result<Vec<(String, String)>, ApiError> {
        self.require_content_side()?;
        self.host.bridge.player_alias(id).ok_or(ApiError::StaleHandle)?;
        let bridge = &self.host.bridge;
        Ok(bridge
            .cosmetics_of(id)
            .into_iter()
            .filter_map(|(slot, cid)| bridge.content_key(ContentKind::Cosmetic, cid).map(|k| (slot, k)))
            .collect())
    }
    fn content_equip(&mut self, id: &str, key: &str) -> Result<(), ApiError> {
        self.require_content_side()?;
        let slot = self
            .slot()
            .cosmetics
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| self.unknown_cosmetic(key))?;
        self.host.bridge.player_alias(id).ok_or(ApiError::StaleHandle)?;
        let cid = self
            .host
            .bridge
            .content_id(ContentKind::Cosmetic, key)
            .ok_or_else(|| ApiError::InvalidArgument(format!("`{key}` has no id in this world")))?;
        self.host.bridge.set_cosmetic(id, &slot, Some(cid))
    }
    fn content_unequip(&mut self, id: &str, slot: &str) -> Result<(), ApiError> {
        self.require_content_side()?;
        if !bhmod_manifest::content::SLOTS.contains(&slot) {
            return Err(ApiError::InvalidArgument(format!("`{slot}` is not a cosmetic slot")));
        }
        self.host.bridge.player_alias(id).ok_or(ApiError::StaleHandle)?;
        self.host.bridge.set_cosmetic(id, slot, None)
    }
    fn content_give_item(&mut self, id: &str, key: &str, count: u32) -> Result<(), ApiError> {
        self.require_content_side()?;
        let item = self.own_content_id(ContentKind::Item, key)?;
        self.host.bridge.player_alias(id).ok_or(ApiError::StaleHandle)?;
        if count == 0 || count > MAX_GIVE {
            return Err(limit("items per give-item call"));
        }
        self.host.bridge.give_item(id, item, count)
    }
    fn content_place_block(&mut self, x: i32, y: i32, key: &str) -> Result<(), ApiError> {
        self.require_content_side()?;
        let block = self.own_content_id(ContentKind::Block, key)?;
        let has_base = self.slot().blocks.iter().any(|(k, base)| k == key && *base);
        if !has_base {
            return Err(ApiError::InvalidArgument(format!("`{key}` declares no base block")));
        }
        self.host.bridge.place_block(x, y, block)
    }
    fn content_block_at(&mut self, x: i32, y: i32) -> Option<String> {
        let id = self.host.bridge.block_at(x, y)?;
        self.host.bridge.content_key(ContentKind::Block, id)
    }
    fn handle_of_player(&mut self, id: &str) -> u64 {
        self.host.handles.of_player(id)
    }
    fn player_of_handle(&mut self, handle: u64) -> Result<String, ApiError> {
        self.host.handles.player(handle).map(str::to_string)
    }

    fn subscribe_custom(&mut self, name: &str, priority: Priority, ignore: bool) -> Result<SubscriptionId, ApiError> {
        if channels::split_name(name).is_none() {
            return Err(ApiError::InvalidArgument(format!("custom event name {name:?} is not <mod-id>:<name>")));
        }
        self.host.bus.subscribe_custom(self.owner, name, priority, ignore).ok_or_else(|| limit("subscriptions per mod"))
    }
    fn fire_custom(&mut self, name: &str, payload: &[u8]) -> Result<bool, ApiError> {
        let source = self.slot().info.id.clone();
        if channels::split_name(name) != Some(source.as_str()) {
            return Err(ApiError::InvalidArgument(format!("custom event names must be {source}:<name>")));
        }
        Value::decode(payload).map_err(|e| ApiError::InvalidArgument(format!("payload: {e}")))?;
        let ev = Event::Custom { name: name.to_string(), source, payload: payload.to_vec(), cancelled: false };
        Ok(self.host.fire(ev).cancelled())
    }

    fn provide_service(&mut self, name: &str, version: &str) -> Result<(), ApiError> {
        let declared = self.slot().provides.clone();
        self.host.services.provide(self.owner, name, version, &declared)
    }
    fn service_generation(&mut self, name: &str, req: &str) -> u32 {
        self.host.services.lookup(name, req).map_or(0, |p| self.host.slots[p.owner].generation)
    }
    fn call_service(&mut self, name: &str, req: &str, method: &str, args: &[u8]) -> Result<Vec<u8>, ServiceError> {
        let provider = self.host.services.lookup(name, req)?.owner;
        let pid = self.host.slots[provider].info.id.clone();
        if self.host.on_stack(provider) {
            return Err(ServiceError::Reentrant);
        }
        if !self.may_use(&pid) {
            return Err(ServiceError::NotPermitted);
        }
        match Value::decode(args) {
            Ok(Value::List(_)) => {}
            Ok(_) => return Err(ServiceError::BadValue("arguments must be a list".into())),
            Err(e) => return Err(ServiceError::BadValue(format!("arguments: {e}"))),
        }
        let what = format!("service {name}.{method}");
        match self.host.call(provider, HandlerKind::Service, &what, |i, h| i.call_service(h, name, method, args)) {
            Some(Ok(out)) => match Value::decode(&out) {
                Ok(_) => Ok(out),
                Err(e) => Err(ServiceError::BadValue(format!("result: {e}"))),
            },
            Some(Err(CallError::Failed(m))) => Err(ServiceError::ProviderError(m)),
            // `None` cannot happen: the provider is enabled (it is registered) and not on the stack.
            _ => Err(ServiceError::Trap),
        }
    }

    fn call_remote(
        &mut self,
        to: RemoteTarget,
        name: &str,
        version_req: &str,
        method: &str,
        args: &[u8],
        timeout_ms: u32,
    ) -> Result<u32, ApiError> {
        self.require("channels")?;
        match (&to, self.host.side) {
            (RemoteTarget::Server, Side::Client) => {}
            (RemoteTarget::Player(p), Side::Server) => {
                self.host.bridge.player_alias(p).ok_or(ApiError::StaleHandle)?;
            }
            (RemoteTarget::Server, Side::Server) => {
                return Err(ApiError::InvalidArgument("a server mod calls players, not the server".into()))
            }
            (RemoteTarget::Player(_), Side::Client) => {
                return Err(ApiError::InvalidArgument("a client mod calls the server, not players".into()))
            }
        }
        for (what, v) in [("service", name), ("method", method)] {
            if v.is_empty() || v.len() > remote::MAX_NAME {
                return Err(ApiError::InvalidArgument(format!("{what} name must be 1 to {} bytes", remote::MAX_NAME)));
            }
        }
        VersionReq::parse(version_req)
            .map_err(|_| ApiError::InvalidArgument(format!("{version_req:?} is not a semver requirement")))?;
        if args.len() > remote::MAX_PAYLOAD {
            return Err(limit("remote call arguments"));
        }
        match Value::decode(args) {
            Ok(Value::List(_)) => {}
            Ok(_) => return Err(ApiError::InvalidArgument("arguments must be a list".into())),
            Err(e) => return Err(ApiError::InvalidArgument(format!("arguments: {e}"))),
        }
        let timeout = match timeout_ms {
            0 => remote::DEFAULT_TIMEOUT_MS,
            t if t <= remote::MAX_TIMEOUT_MS => t,
            _ => return Err(ApiError::InvalidArgument(format!("timeout is at most {} ms", remote::MAX_TIMEOUT_MS))),
        };
        let deadline = self.host.clock.now() + Duration::from_millis(u64::from(timeout));
        let id = self.host.remote.begin(self.owner, to.clone(), deadline)?;
        let call = RemoteCall {
            id,
            target: to.clone(),
            mod_id: self.slot().info.id.clone(),
            service: name.to_string(),
            version_req: version_req.to_string(),
            method: method.to_string(),
            args: args.to_vec(),
        };
        if let Err(e) = self.host.bridge.service_send(&call) {
            // The caller is on the call stack, so the event reaches it once this call has returned.
            self.host.remote.answer(id, &to);
            self.host.deliver_remote(self.owner, id, Err(e));
        }
        Ok(id)
    }
    fn cancel_remote(&mut self, id: u32) -> bool {
        self.host.remote.cancel(self.owner, id)
    }
    fn remote_caller(&mut self) -> Option<RemoteCaller> {
        self.host.remote.caller.clone()
    }

    fn settings_get(&mut self, key: &str) -> Option<Vec<u8>> {
        self.slot().values.get(key).map(|v| cfg::to_value(v).encode())
    }
    fn settings_all(&mut self) -> Vec<u8> {
        let fields = &self.slot().settings.fields;
        let values = &self.slot().values;
        Value::Map(fields.iter().map(|f| (f.key.clone(), cfg::to_value(&values[&f.key]))).collect()).encode()
    }
    fn settings_set(&mut self, key: &str, value: &[u8]) -> Result<(), ApiError> {
        let j = cfg::from_cbor(value)?;
        self.host.set_in_slot(self.owner, key, &j).map(|_| ()).map_err(|e| ApiError::InvalidArgument(e.to_string()))
    }

    fn channel_open(&mut self, name: &str) -> Result<ChannelId, ApiError> {
        self.require("channels")?;
        let Some(m) = channels::split_name(name) else {
            return Err(ApiError::InvalidArgument(format!("channel name {name:?} is not <mod-id>:<name>")));
        };
        if !self.may_use(m) {
            return Err(ApiError::NotPermitted(format!("channel of {m} (not a dependency)")));
        }
        self.host.channels.open(self.owner, name).ok_or_else(|| limit("channels per mod"))
    }
    fn channel_send(&mut self, channel: ChannelId, to: ChannelTarget, data: &[u8]) -> Result<(), ApiError> {
        let name = self
            .host
            .channels
            .name_of(self.owner, channel)
            .ok_or_else(|| ApiError::InvalidArgument(format!("channel {channel} is not open")))?
            .to_string();
        if data.len() > channels::MAX_MESSAGE {
            return Err(limit("channel message size"));
        }
        let id = self.slot().info.id.clone();
        match to {
            ChannelTarget::Local => {
                let msg = Message {
                    name,
                    from_owner: Some(self.owner),
                    source: ChannelSource::Local(id),
                    data: data.to_vec(),
                };
                self.host.channels.enqueue(msg).then_some(()).ok_or_else(|| limit("queued channel messages"))
            }
            other => self.host.bridge.channel_send(&id, &name, &other, data),
        }
    }
    fn channel_close(&mut self, channel: ChannelId) -> bool {
        self.host.channels.close(self.owner, channel)
    }

    fn ui_set_widget(&mut self, id: &str, widget: Widget) -> Result<(), ApiError> {
        self.host.ui.check_set(self.owner, id, &widget)?;
        let mod_id = self.slot().info.id.clone();
        self.host.bridge.ui(&mod_id, &UiCommand::SetWidget { id: id.to_string(), widget })?;
        self.host.ui.added(self.owner, id);
        Ok(())
    }
    fn ui_remove_widget(&mut self, id: &str) -> bool {
        if !self.host.ui.removed(self.owner, id) {
            return false;
        }
        let mod_id = self.slot().info.id.clone();
        let _ = self.host.bridge.ui(&mod_id, &UiCommand::RemoveWidget { id: id.to_string() });
        true
    }
    fn ui_toast(&mut self, text: &str) -> Result<(), ApiError> {
        crate::ui::check_notice(text)?;
        let mod_id = self.slot().info.id.clone();
        self.host.bridge.ui(&mod_id, &UiCommand::Toast { text: text.to_string() })
    }
    fn ui_notice(&mut self, text: &str, level: NoticeLevel) -> Result<(), ApiError> {
        crate::ui::check_notice(text)?;
        let mod_id = self.slot().info.id.clone();
        self.host.bridge.ui(&mod_id, &UiCommand::Notice { text: text.to_string(), level })
    }
    fn ui_show_dialog(&mut self, dialog: Dialog) -> Result<u32, ApiError> {
        let id = self.host.ui.open_dialog(self.owner, &dialog)?;
        let mod_id = self.slot().info.id.clone();
        if let Err(e) = self.host.bridge.ui(&mod_id, &UiCommand::ShowDialog { id, dialog }) {
            self.host.ui.close_dialog(self.owner, id);
            return Err(e);
        }
        Ok(id)
    }
    fn ui_close_dialog(&mut self, id: u32) -> bool {
        if !self.host.ui.close_dialog(self.owner, id) {
            return false;
        }
        let mod_id = self.slot().info.id.clone();
        let _ = self.host.bridge.ui(&mod_id, &UiCommand::CloseDialog { id });
        true
    }
    fn ui_set_overhead(&mut self, player: &str, icon: OverheadIcon) -> Result<(), ApiError> {
        self.host.bridge.player_alias(player).ok_or(ApiError::StaleHandle)?;
        let mod_id = self.slot().info.id.clone();
        self.host.bridge.ui(&mod_id, &UiCommand::SetOverhead { player: player.to_string(), icon })?;
        self.host.ui.overhead_set(self.owner, player);
        Ok(())
    }
    fn ui_clear_overhead(&mut self, player: &str) -> bool {
        if !self.host.ui.overhead_cleared(self.owner, player) {
            return false;
        }
        let mod_id = self.slot().info.id.clone();
        let _ = self.host.bridge.ui(&mod_id, &UiCommand::ClearOverhead { player: player.to_string() });
        true
    }
    fn ui_set_button(&mut self, id: &str, button: Button) -> Result<(), ApiError> {
        self.host.ui.check_button(self.owner, id, &button)?;
        let mod_id = self.slot().info.id.clone();
        self.host.bridge.ui(&mod_id, &UiCommand::SetButton { id: id.to_string(), button })?;
        self.host.ui.button_added(self.owner, id);
        Ok(())
    }
    fn ui_remove_button(&mut self, id: &str) -> bool {
        if !self.host.ui.button_removed(self.owner, id) {
            return false;
        }
        let mod_id = self.slot().info.id.clone();
        let _ = self.host.bridge.ui(&mod_id, &UiCommand::RemoveButton { id: id.to_string() });
        true
    }
    fn ui_open_settings(&mut self, mod_id: &str) -> Result<(), ApiError> {
        if self.host.slot_of(mod_id).is_none() {
            return Err(ApiError::InvalidArgument(format!("no loaded mod {mod_id:?}")));
        }
        let own = self.slot().info.id.clone();
        self.host.bridge.ui(&own, &UiCommand::OpenSettings { mod_id: mod_id.to_string() })
    }

    fn voice(&mut self, call: VoiceCall) -> Result<VoiceReply, ApiError> {
        self.require("voice")?;
        if call.side().is_some_and(|s| s != self.host.side) {
            return Err(ApiError::Unsupported);
        }
        call.check()?;
        for p in call.players() {
            self.host.bridge.player_alias(p).ok_or(ApiError::StaleHandle)?;
        }
        let voice = self.host.bridge.voice().ok_or(ApiError::Unsupported)?;
        call.run(voice)
    }

    fn terrain_solid_count(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<u32, ApiError> {
        self.require_content_side()?;
        let bridge = &self.host.bridge;
        Ok(crate::voice::solid_count((x1, y1), (x2, y2), bridge.world_width(), &|x, y| bridge.tile_solid(x, y)))
    }

    fn input_bind(&mut self, action: &str, default_key: &str) -> Result<(), ApiError> {
        if self.host.side != Side::Client {
            return Err(ApiError::Unsupported);
        }
        self.host.input.check_bind(self.owner, action, default_key)?;
        let mod_id = self.slot().info.id.clone();
        let cmd = InputCommand::Bind { action: action.to_string(), default_key: default_key.to_string() };
        self.host.bridge.input(&mod_id, &cmd)?;
        self.host.input.bound(self.owner, action, default_key);
        Ok(())
    }
    fn input_unbind(&mut self, action: &str) -> bool {
        if !self.host.input.unbound(self.owner, action) {
            return false;
        }
        let mod_id = self.slot().info.id.clone();
        let _ = self.host.bridge.input(&mod_id, &InputCommand::Unbind { action: action.to_string() });
        true
    }
    fn input_key_of(&mut self, action: &str) -> Option<String> {
        let default = self.host.input.default_key(self.owner, action)?.to_string();
        self.host.bridge.input_key(&self.slot().info.id, action, &default)
    }
}
