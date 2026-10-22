//! [`ModHost`]: loads mods and runs them against a [`GameBridge`].
//!
//! The host program calls the `ModHost` from its main thread at its hook points (world loaded, join, quit, chat,
//! command, tick, poll, save, stop). Every guest call runs under the time budget, inside `catch_unwind`; traps,
//! timeouts and panics are strikes, and too many strikes disable the mod, never the game.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use crate::bridge::GameBridge;
use crate::budget::{Budgets, Clock, Strikes, SystemClock};
use crate::bus::EventBus;
use crate::commands::CommandRegistry;
use crate::config::{required_permissions, ModsConfig, FILE_NAME};
use crate::discover::discover;
use crate::resolve::resolve;
use crate::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, Runtime, SubscriptionId, TaskId};
use crate::scheduler::Scheduler;
use crate::storage::{MemoryStorage, StorageProvider, Store};
use crate::types::{
    ApiError, CommandOutcome, CommandSender, CommandSpec, Event, EventKind, LogLevel, Priority, Role, SavePhase, Side,
};
use crate::API_VERSION;

/// Longest log message in bytes.
pub const MAX_LOG: usize = 4096;

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

    /// Runs one guest call for `owner` under the budget. `None` when the mod has no instance (disabled).
    fn call<R>(
        &mut self,
        owner: usize,
        what: &str,
        f: impl FnOnce(&mut dyn Instance, &mut dyn HostApi) -> Result<R, CallError>,
    ) -> Option<Result<R, CallError>> {
        let mut instance = self.slots[owner].instance.take()?;
        let start = self.clock.now();
        let deadline = start + self.budgets.hard;
        let result = {
            let ModHost { slots, bus, commands, scheduler, bridge, side, .. } = self;
            let mut ctx = Ctx {
                owner,
                slot: &mut slots[owner],
                bus,
                commands,
                scheduler,
                bridge: bridge.as_ref(),
                side: *side,
                deadline,
            };
            catch_unwind(AssertUnwindSafe(|| f(instance.as_mut(), &mut ctx)))
                .unwrap_or_else(|p| Err(CallError::Trap(format!("panic: {}", panic_text(p)))))
        };
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
        Some(result)
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

    /// Delivers an event to every subscriber and returns it with the handlers' changes applied.
    pub fn fire(&mut self, mut ev: Event) -> Event {
        let kind = ev.kind();
        for sub in self.bus.plan(kind) {
            if self.slots[sub.owner].info.state != ModState::Enabled || (sub.ignore_cancelled && ev.cancelled()) {
                continue;
            }
            let snapshot = ev.clone();
            let what = format!("{} handler", kind.name());
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
        ev
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
    }

    /// One network poll: runs next-poll tasks.
    pub fn poll(&mut self) {
        if self.enabled && self.scheduler.has_poll() {
            let tasks = self.scheduler.take_poll();
            self.run_tasks(tasks, None);
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
    owner: usize,
    slot: &'a mut Slot,
    bus: &'a mut EventBus,
    commands: &'a mut CommandRegistry,
    scheduler: &'a mut Scheduler,
    bridge: &'a dyn GameBridge,
    side: Side,
    deadline: Instant,
}

impl Ctx<'_> {
    fn require(&self, perm: &str) -> Result<(), ApiError> {
        if self.slot.permissions.iter().any(|p| p == perm) {
            Ok(())
        } else {
            Err(ApiError::NotPermitted(perm.to_string()))
        }
    }

    fn store(&mut self) -> Result<&mut Store, ApiError> {
        self.slot.store.as_mut().ok_or_else(|| ApiError::Internal("storage unavailable".into()))
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
        self.slot.info.id.clone()
    }
    fn mod_version(&self) -> String {
        self.slot.info.version.clone()
    }
    fn current_side(&self) -> Side {
        self.side
    }
    fn host_name(&self) -> String {
        self.bridge.host_name()
    }
    fn log(&mut self, level: LogLevel, message: &str) {
        self.bridge.log(Some(&self.slot.info.id), level, truncate(message, MAX_LOG));
    }
    fn player_alias(&mut self, id: &str) -> Result<String, ApiError> {
        self.bridge.player_alias(id).ok_or(ApiError::StaleHandle)
    }
    fn player_is_online(&mut self, id: &str) -> bool {
        self.bridge.player_alias(id).is_some()
    }
    fn player_role(&mut self, id: &str) -> Result<Role, ApiError> {
        self.bridge.player_alias(id).ok_or(ApiError::StaleHandle)?;
        Ok(self.bridge.player_role(id))
    }
    fn player_send_chat(&mut self, id: &str, text: &str) -> Result<(), ApiError> {
        self.bridge.send_chat(id, text).then_some(()).ok_or(ApiError::StaleHandle)
    }
    fn player_kick(&mut self, id: &str, reason: &str) -> Result<(), ApiError> {
        self.require("players:kick")?;
        self.bridge.kick(id, reason).then_some(()).ok_or(ApiError::StaleHandle)
    }
    fn players_get(&mut self, id: &str) -> Option<String> {
        self.bridge.player_alias(id).map(|_| id.to_string())
    }
    fn players_find_by_alias(&mut self, alias: &str) -> Option<String> {
        let want = alias.to_lowercase();
        self.bridge
            .online_players()
            .into_iter()
            .find(|id| self.bridge.player_alias(id).is_some_and(|a| a.to_lowercase() == want))
    }
    fn players_online(&mut self) -> Vec<String> {
        self.bridge.online_players()
    }
    fn players_broadcast(&mut self, text: &str) -> Result<(), ApiError> {
        self.require("players:chat")?;
        self.bridge.broadcast(text);
        Ok(())
    }
    fn subscribe(&mut self, kind: EventKind, priority: Priority, ignore: bool) -> Result<SubscriptionId, ApiError> {
        self.bus.subscribe(self.owner, kind, priority, ignore).ok_or_else(|| limit("subscriptions per mod"))
    }
    fn unsubscribe(&mut self, sub: SubscriptionId) -> bool {
        self.bus.unsubscribe(self.owner, sub)
    }
    fn register_command(&mut self, spec: CommandSpec) -> Result<CommandId, ApiError> {
        let bridge = self.bridge;
        self.commands.register(self.owner, spec, &|n| bridge.is_builtin_command(n))
    }
    fn unregister_command(&mut self, id: CommandId) -> bool {
        self.commands.unregister(self.owner, id)
    }
    fn run_later(&mut self, delay: u32) -> Result<TaskId, ApiError> {
        self.scheduler.run_later(self.owner, delay).ok_or_else(|| limit("tasks per mod"))
    }
    fn run_every(&mut self, delay: u32, period: u32) -> Result<TaskId, ApiError> {
        if period == 0 {
            return Err(ApiError::InvalidArgument("period must be at least 1".into()));
        }
        self.scheduler.run_every(self.owner, delay, period).ok_or_else(|| limit("tasks per mod"))
    }
    fn run_next_poll(&mut self) -> Result<TaskId, ApiError> {
        self.scheduler.run_next_poll(self.owner).ok_or_else(|| limit("tasks per mod"))
    }
    fn cancel_task(&mut self, id: TaskId) -> bool {
        self.scheduler.cancel(self.owner, id)
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
                self.bridge.log(Some(&self.slot.info.id), LogLevel::Error, &format!("storage delete failed: {e}"));
                false
            }
        }
    }
    fn storage_keys(&mut self, prefix: &str) -> Vec<String> {
        self.store().map(|s| s.keys(prefix)).unwrap_or_default()
    }
}
