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

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use crate::bridge::GameBridge;
use crate::budget::{Budgets, Clock, Strikes, SystemClock};
use crate::bus::{EventBus, Subscription};
use crate::channels::{self, ChannelId, Channels, Message};
use crate::commands::CommandRegistry;
use crate::config::{required_permissions, ModsConfig, FILE_NAME};
use crate::discover::discover;
use crate::handles::Handles;
use crate::resolve::resolve;
use crate::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, Runtime, SubscriptionId, TaskId};
use crate::scheduler::Scheduler;
use crate::services::ServiceRegistry;
use crate::storage::{MemoryStorage, StorageProvider, Store};
use crate::types::{
    ApiError, ChannelSource, ChannelTarget, CommandOutcome, CommandSender, CommandSpec, Event, EventKind, LogLevel,
    Pos, Priority, Role, SavePhase, ServiceError, Side,
};
use crate::value::Value;
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
    instance: Option<Box<dyn Instance>>,
    store: Option<Store>,
    strikes: Strikes,
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
        let HostOptions { mods_dir, side, budgets, runtimes, storage, clock } = options;
        let mut host = ModHost {
            slots: Vec::new(),
            bus: EventBus::default(),
            commands: CommandRegistry::default(),
            scheduler: Scheduler::default(),
            services: ServiceRegistry::default(),
            channels: Channels::default(),
            handles: Handles::default(),
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
        };
        let config = host.read_config(&mods_dir);
        let (packages, problems) = discover(&mods_dir);
        for p in problems {
            host.log(LogLevel::Error, &format!("cannot load {}: {}", p.origin.display(), p.message));
        }
        let kinds: Vec<_> = runtimes.iter().map(|r| r.kind()).collect();
        let (ordered, problems, skipped) = resolve(packages, &config, side, &kinds);
        for p in problems {
            host.log(LogLevel::Error, &format!("not loading {}: {}", p.origin.display(), p.message));
        }
        for s in skipped {
            host.log(LogLevel::Debug, &format!("skipping {s}"));
        }
        let limits = InstanceLimits { memory_bytes: budgets.memory_bytes };
        for pkg in ordered {
            let files = pkg.side_files(side).expect("resolve keeps only mods with an entry on this side");
            let entry = match side {
                Side::Server => &pkg.manifest.entry.server,
                Side::Client => &pkg.manifest.entry.client,
            };
            let kind = entry.as_ref().map(|e| e.runtime);
            let rt = runtimes.iter().find(|r| Some(r.kind()) == kind).expect("resolve checked the runtime");
            let id = pkg.manifest.id.clone();
            match rt.instantiate(&id, &files, limits) {
                Ok(instance) => host.slots.push(Slot {
                    info: ModInfo {
                        id,
                        version: pkg.manifest.version.clone(),
                        origin: pkg.origin.clone(),
                        sha256: pkg.sha256.clone(),
                        state: ModState::Loaded,
                    },
                    permissions: required_permissions(&pkg.manifest.permissions),
                    deps: pkg
                        .manifest
                        .dependencies
                        .keys()
                        .chain(pkg.manifest.soft_dependencies.keys())
                        .cloned()
                        .collect(),
                    provides: pkg
                        .manifest
                        .provides
                        .services
                        .iter()
                        .map(|s| (s.name.clone(), s.version.clone()))
                        .collect(),
                    instance: Some(instance),
                    store: None,
                    strikes: Strikes::default(),
                }),
                Err(e) => host.log(LogLevel::Error, &format!("cannot start {id}: {e}")),
            }
        }
        if !host.slots.is_empty() {
            let names: Vec<String> = host.slots.iter().map(|s| format!("{} {}", s.info.id, s.info.version)).collect();
            host.log(LogLevel::Info, &format!("loaded {} mod(s): {}", names.len(), names.join(", ")));
        }
        host
    }

    fn read_config(&self, dir: &Path) -> ModsConfig {
        let path = dir.join(FILE_NAME);
        let Ok(text) = std::fs::read_to_string(&path) else { return ModsConfig::default() };
        ModsConfig::parse(&text).unwrap_or_else(|e| {
            self.log(LogLevel::Error, &format!("{} is invalid, no permissions are granted: {e}", path.display()));
            ModsConfig::default()
        })
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
        what: &str,
        f: impl FnOnce(&mut dyn Instance, &mut dyn HostApi) -> Result<R, CallError>,
    ) -> Option<Result<R, CallError>> {
        // A mod on the call stack has its instance taken out, so it cannot be entered twice.
        let mut instance = self.slots[owner].instance.take()?;
        let start = self.clock.now();
        let own = start + self.budgets.hard;
        let deadline = self.stack.last().map_or(own, |&(_, outer)| own.min(outer));
        self.stack.push((owner, deadline));
        let result = {
            let mut ctx = Ctx { host: self, owner, deadline };
            catch_unwind(AssertUnwindSafe(|| f(instance.as_mut(), &mut ctx)))
                .unwrap_or_else(|p| Err(CallError::Trap(format!("panic: {}", panic_text(p)))))
        };
        self.stack.pop();
        let elapsed = self.clock.now().saturating_duration_since(start);
        let slot = &mut self.slots[owner];
        let id = slot.info.id.clone();
        slot.instance = Some(instance);
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
                self.call(sub.owner, &what, |i, h| i.handle_event(h, sub.id, &ev));
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
            if let Some(Err(e)) = self.call(owner, "on-disable", |i, h| i.on_disable(h)) {
                let id = self.slots[owner].info.id.clone();
                self.bridge.log(Some(&id), LogLevel::Warn, &format!("on-disable failed: {e}"));
            }
        }
        self.bus.remove_owner(owner);
        self.commands.remove_owner(owner);
        self.scheduler.remove_owner(owner);
        self.services.remove_owner(owner);
        self.channels.remove_owner(owner);
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
        for i in 0..self.slots.len() {
            if self.slots[i].info.state != ModState::Loaded {
                continue;
            }
            let id = self.slots[i].info.id.clone();
            match self.storage.open(&id).and_then(Store::open) {
                Ok(s) => self.slots[i].store = Some(s),
                Err(e) => self.bridge.log(Some(&id), LogLevel::Error, &format!("storage unavailable: {e}")),
            }
            self.slots[i].info.state = ModState::Enabled;
            match self.call(i, "on-enable", |inst, h| inst.on_enable(h)).unwrap_or(Ok(())) {
                Ok(()) => self.bridge.log(Some(&id), LogLevel::Info, "enabled"),
                Err(e) => {
                    self.slots[i].info.state = ModState::Loaded;
                    self.disable_slot(i, format!("on-enable failed: {e}"));
                }
            }
        }
        self.fire(Event::WorldLoaded { world_name: world_name.to_string(), world_id: world_id.to_string() });
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
    pub fn fire(&mut self, mut ev: Event) -> Event {
        let kind = ev.kind();
        let custom = match &ev {
            Event::Custom { name, .. } => Some(name.clone()),
            _ => None,
        };
        for sub in self.bus.plan(kind, custom.as_deref()) {
            if self.slots[sub.owner].info.state != ModState::Enabled || (sub.ignore_cancelled && ev.cancelled()) {
                continue;
            }
            if self.on_stack(sub.owner) {
                self.deferred.push((sub, ev.clone()));
                continue;
            }
            let snapshot = ev.clone();
            let what = format!("{} handler", custom.as_deref().unwrap_or(kind.name()));
            let result = self.call(sub.owner, &what, |i, h| i.handle_event(h, sub.id, &snapshot));
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
        }
        ev
    }

    /// A channel message arrived from the network (milestone M5: 0xBC `CHANNEL`). It is delivered with the next
    /// poll to every mod that opened `name`. False when the queue is full.
    pub fn channel_receive(&mut self, name: &str, source: ChannelSource, data: Vec<u8>) -> bool {
        self.channels.enqueue(Message { name: name.to_string(), from_owner: None, source, data })
    }

    /// Delivers queued channel messages.
    fn deliver_channels(&mut self) {
        for (msg, to) in self.channels.drain() {
            for (owner, ch) in to {
                // A mod disabled meanwhile has no instance, so `call` skips it.
                self.call(owner, "channel message", |i, h| i.on_channel_message(h, ch, &msg.source, &msg.data));
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
        match self.call(cmd.owner, &what, |i, h| i.run_command(h, cmd.id, &sender, &name, &args)) {
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
                self.call(owner, "scheduled task", |i, h| i.run_task(h, id));
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
    }

    /// One network poll: runs next-poll tasks and delivers channel messages.
    pub fn poll(&mut self) {
        if !self.enabled {
            return;
        }
        if self.scheduler.has_poll() {
            let tasks = self.scheduler.take_poll();
            self.run_tasks(tasks, None);
        }
        if self.channels.has_queued() {
            self.deliver_channels();
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

    /// True when the calling mod is `id` or depends (softly) on it.
    fn may_use(&self, id: &str) -> bool {
        self.slot().info.id == id || self.slot().deps.iter().any(|d| d == id)
    }
}

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
        match self.host.call(provider, &what, |i, h| i.call_service(h, name, method, args)) {
            Some(Ok(out)) => match Value::decode(&out) {
                Ok(_) => Ok(out),
                Err(e) => Err(ServiceError::BadValue(format!("result: {e}"))),
            },
            Some(Err(CallError::Failed(m))) => Err(ServiceError::ProviderError(m)),
            // `None` cannot happen: the provider is enabled (it is registered) and not on the stack.
            _ => Err(ServiceError::Trap),
        }
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
}
