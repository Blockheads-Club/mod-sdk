//! Rust SDK for Blockheads mods.
//!
//! A mod is a WebAssembly component built for `wasm32-wasip2`. This crate generates the `blockheads:api` bindings
//! (with `wit-bindgen`) and adds a small layer on top so a mod reads like its Luau twin:
//!
//! ```ignore
//! use blockheads_mod::prelude::*;
//!
//! struct Hello;
//!
//! impl Mod for Hello {
//!     fn on_enable() -> Result<(), String> {
//!         events::on(EventKind::PlayerJoin, |ev| {
//!             if let Some(p) = ev.player() {
//!                 let _ = p.send_chat(&format!("Hello, {}!", p.alias()));
//!             }
//!         })?;
//!         commands::register(Command::new("hello").alias("hi"), |sender, _label, _args| {
//!             Reply::lines([format!("Hello, {}!", sender.name())])
//!         })?;
//!         Ok(())
//!     }
//! }
//!
//! blockheads_mod::export_mod!(Hello);
//! ```
//!
//! - [`events::on`] / [`events::on_with`] subscribe a closure; it gets an [`events::EventCtx`] to read the event,
//!   cancel it or replace the chat text.
//! - [`commands::register`] registers a [`commands::Command`] with a closure returning a [`commands::Reply`].
//! - [`scheduler`] runs closures later, repeatedly or at the next network poll.
//! - [`log`], [`storage`], [`players`] and [`host`] wrap the WIT imports; the raw generated bindings are in
//!   [`bindings`].
//!
//! Handlers run one at a time on the host's main thread, so mod state can live in a `thread_local!` or in the
//! closures themselves.

use core::cell::RefCell;
use std::collections::BTreeMap;

/// The bindings generated from `wit/blockheads-api` (world `server-mod`).
#[allow(missing_docs, clippy::all)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../../wit/blockheads-api",
        world: "server-mod",
        pub_export_macro: true,
        export_macro_name: "__export_server_mod",
        default_bindings_module: "blockheads_mod::bindings",
    });
}

use bindings::blockheads::api as wit;
use bindings::exports::blockheads::api::guest;

pub use wit::players::{Player, Role};
pub use wit::types::{ApiError, Pos, Priority, Side};

/// Everything a mod usually needs.
pub mod prelude {
    pub use crate::commands::{self, Command, Reply, Sender};
    pub use crate::events::{self, EventCtx, EventKind};
    pub use crate::{channels, host, log, players, scheduler, services, storage};
    pub use crate::{ApiError, Mod, Player, Priority, Role, Side};
}

/// A mod's entry points. Register handlers in [`Mod::on_enable`]; everything else is dispatched by the SDK.
pub trait Mod {
    /// Called once when the mod is enabled (on the server: when the world has loaded). An error disables the mod.
    fn on_enable() -> Result<(), String>;

    /// Called once when the mod is disabled. Flush state here.
    fn on_disable() {}

    /// Called when another mod calls a service of this mod (services arrive with API milestone M3).
    fn call_service(name: String, method: String, args: Vec<u8>) -> Result<Vec<u8>, String> {
        let _ = (name, method, args);
        Err(String::from("this mod provides no services"))
    }

    /// Called when a message arrives on a channel this mod opened (see [`channels`]).
    fn on_channel_message(channel: u32, origin: channels::Source<'_>, data: Vec<u8>) {
        let _ = (channel, origin, data);
    }
}

/// Exports a [`Mod`] implementation as the component's `guest` interface.
#[macro_export]
macro_rules! export_mod {
    ($t:ty) => {
        const _: () = {
            type BlockheadsModExport = $crate::Exported<$t>;
    $crate::bindings::__export_server_mod!(BlockheadsModExport with_types_in $crate::bindings);
        };
    };
}

/// The type [`export_mod!`] exports; it forwards the `guest` interface to `M` and the SDK's dispatch tables.
pub struct Exported<M>(core::marker::PhantomData<M>);

type EventFn = Box<dyn FnMut(&mut events::EventCtx<'_, '_>)>;
type CommandFn = Box<dyn FnMut(&commands::Sender<'_>, &str, &[String]) -> commands::Reply>;
type TaskFn = Box<dyn FnMut()>;

#[derive(Default)]
struct Registry {
    events: BTreeMap<u32, Option<EventFn>>,
    commands: BTreeMap<u32, Option<CommandFn>>,
    tasks: BTreeMap<u64, (Option<TaskFn>, bool)>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|r| f(&mut r.borrow_mut()))
}

impl<M: Mod> guest::Guest for Exported<M> {
    fn on_enable() -> Result<(), String> {
        M::on_enable()
    }

    fn on_disable() {
        M::on_disable()
    }

    fn handle_event(sub: u32, ev: wit::events::Event<'_>) -> wit::events::Outcome {
        let mut ctx = events::EventCtx::new(&ev);
        // Take the handler out while it runs so it may (un)register handlers itself.
        let f = with_registry(|r| r.events.get_mut(&sub).and_then(Option::take));
        if let Some(mut f) = f {
            f(&mut ctx);
            with_registry(|r| {
                if let Some(slot) = r.events.get_mut(&sub) {
                    *slot = Some(f);
                }
            });
        }
        ctx.into_outcome()
    }

    fn run_command(
        id: u32,
        sender: wit::events::CommandSender<'_>,
        label: String,
        args: Vec<String>,
    ) -> wit::commands::CommandOutcome {
        let sender = commands::Sender::from_wit(sender);
        let f = with_registry(|r| r.commands.get_mut(&id).and_then(Option::take));
        let Some(mut f) = f else {
            return wit::commands::CommandOutcome { handled: false, reply: Vec::new() };
        };
        let reply = f(&sender, &label, &args);
        with_registry(|r| {
            if let Some(slot) = r.commands.get_mut(&id) {
                *slot = Some(f);
            }
        });
        reply.into_wit()
    }

    fn run_task(id: u64) {
        let f = with_registry(|r| match r.tasks.get_mut(&id) {
            Some((f, true)) => f.take().map(|f| (f, true)),
            Some((f, false)) => {
                let f = f.take();
                r.tasks.remove(&id);
                f.map(|f| (f, false))
            }
            None => None,
        });
        if let Some((mut f, repeat)) = f {
            f();
            if repeat {
                with_registry(|r| {
                    if let Some((slot, _)) = r.tasks.get_mut(&id) {
                        *slot = Some(f);
                    }
                });
            }
        }
    }

    fn call_service(name: String, method: String, args: Vec<u8>) -> Result<Vec<u8>, String> {
        M::call_service(name, method, args)
    }

    fn on_channel_message(channel: u32, origin: wit::channels::Source<'_>, data: Vec<u8>) {
        M::on_channel_message(channel, origin, data)
    }
}

/// Facts about the host and this mod.
pub mod host {
    pub use crate::wit::host::{api_version, current_side, host_name, mod_id, mod_version};
}

/// Logging to the server console or client log (prefixed with the mod id by the host).
pub mod log {
    use crate::wit::log::{log, Level};

    pub use crate::wit::log::Level as LogLevel;

    /// Logs at `trace`.
    pub fn trace(msg: &str) {
        log(Level::Trace, msg)
    }
    /// Logs at `debug`.
    pub fn debug(msg: &str) {
        log(Level::Debug, msg)
    }
    /// Logs at `info`.
    pub fn info(msg: &str) {
        log(Level::Info, msg)
    }
    /// Logs at `warn`.
    pub fn warn(msg: &str) {
        log(Level::Warn, msg)
    }
    /// Logs at `error`.
    pub fn error(msg: &str) {
        log(Level::Error, msg)
    }
    /// Logs at any level.
    pub fn at(level: LogLevel, msg: &str) {
        log(level, msg)
    }
}

/// Cross-mod services: values are canonical CBOR (players as `{"$h": player.value_handle()}`).
pub mod services {
    pub use crate::wit::services::{call, provide, CallError};
}

/// Named byte channels between mods (local delivery since API 0.2).
pub mod channels {
    pub use crate::wit::channels::{close, open, send, Source, Target};
}

/// Connected players.
pub mod players {
    pub use crate::wit::players::{broadcast, find_by_alias, from_value_handle, get, online, Player, Role};
}

/// The per-mod, per-world key-value store.
pub mod storage {

    pub use crate::wit::storage::{delete, get, keys, set};

    /// Reads a UTF-8 value (`None` when missing or not UTF-8).
    pub fn get_string(key: &str) -> Option<String> {
        get(key).and_then(|v| String::from_utf8(v).ok())
    }

    /// Writes a UTF-8 value.
    pub fn set_string(key: &str, value: &str) -> Result<(), crate::ApiError> {
        set(key, value.as_bytes())
    }
}

/// Game events.
pub mod events {

    use crate::wit::events as w;
    pub use crate::wit::events::{
        BlockBreakEvent, BlockChangedEvent, BlockPlaceEvent, BreakLayer, ChatEvent, CommandEvent, CustomEvent,
        DamageEvent, DeathEvent, Event, EventKind, ObjectKind, ObjectPlaceEvent, PlayerJoinEvent, PlayerMoveEvent,
        PlayerPreLoginEvent, PlayerQuitEvent, RejectReason, SavePhase, TickEvent, WorldLoadedEvent, WorldSaveEvent,
    };
    use crate::{with_registry, ApiError, Player, Priority};

    /// One event delivered to a handler, with the handler's decision.
    pub struct EventCtx<'a, 'p> {
        event: &'a Event<'p>,
        cancelled: bool,
        text: Option<String>,
        other: Option<w::Mutation>,
    }

    impl<'a, 'p> EventCtx<'a, 'p> {
        pub(crate) fn new(event: &'a Event<'p>) -> Self {
            let cancelled = match event {
                Event::Chat(c) => c.cancelled,
                Event::Command(c) => c.cancelled,
                Event::Custom(c) => c.cancelled,
                Event::PlayerPreLogin(e) => e.cancelled,
                Event::BlockPlace(e) => e.cancelled,
                Event::BlockBreak(e) => e.cancelled,
                Event::ObjectPlace(e) => e.cancelled,
                Event::Damage(e) => e.cancelled,
                _ => false,
            };
            EventCtx { event, cancelled, text: None, other: None }
        }

        /// The raw event.
        pub fn event(&self) -> &Event<'p> {
            self.event
        }

        /// The event's kind (`None` for custom events).
        pub fn kind(&self) -> Option<EventKind> {
            Some(match self.event {
                Event::PlayerJoin(_) => EventKind::PlayerJoin,
                Event::PlayerQuit(_) => EventKind::PlayerQuit,
                Event::Chat(_) => EventKind::Chat,
                Event::Command(_) => EventKind::Command,
                Event::Tick(_) => EventKind::Tick,
                Event::WorldLoaded(_) => EventKind::WorldLoaded,
                Event::WorldSave(_) => EventKind::WorldSave,
                Event::PlayerPreLogin(_) => EventKind::PlayerPreLogin,
                Event::BlockPlace(_) => EventKind::BlockPlace,
                Event::BlockBreak(_) => EventKind::BlockBreak,
                Event::BlockChanged(_) => EventKind::BlockChanged,
                Event::ObjectPlace(_) => EventKind::ObjectPlace,
                Event::Damage(_) => EventKind::Damage,
                Event::Death(_) => EventKind::Death,
                Event::PlayerMove(_) => EventKind::PlayerMove,
                Event::Custom(_) => return None,
            })
        }

        /// The player of the event (the one who joined, chatted, placed, broke, moved, was damaged or died), or
        /// the player who typed a command. `None` for pre-login (use the event's `player_id`) and world events.
        pub fn player(&self) -> Option<&'p Player> {
            match self.event {
                Event::PlayerJoin(e) => Some(e.player),
                Event::PlayerQuit(e) => Some(e.player),
                Event::Chat(e) => Some(e.player),
                Event::Command(CommandEvent { sender: w::CommandSender::Player(p), .. }) => Some(p),
                Event::BlockPlace(e) => Some(e.player),
                Event::BlockBreak(e) => Some(e.player),
                Event::BlockChanged(e) => e.player,
                Event::ObjectPlace(e) => Some(e.player),
                Event::Damage(e) => e.player,
                Event::Death(e) => e.player,
                Event::PlayerMove(e) => Some(e.player),
                _ => None,
            }
        }

        /// Chooses the rejection reason of a cancelled `player-pre-login` (ignored for other events).
        pub fn set_reject_reason(&mut self, reason: RejectReason) {
            if matches!(self.event, Event::PlayerPreLogin(_)) {
                self.other = Some(w::Mutation::RejectReason(reason));
            }
        }

        /// Replaces the damage amount of a `damage` event (ignored for other events).
        pub fn set_damage(&mut self, amount: f32) {
            if matches!(self.event, Event::Damage(_)) {
                self.other = Some(w::Mutation::DamageAmount(amount));
            }
        }

        /// The chat text as changed by this handler so far (`None` for other events).
        pub fn text(&self) -> Option<&str> {
            match self.event {
                Event::Chat(c) => Some(self.text.as_deref().unwrap_or(&c.text)),
                _ => None,
            }
        }

        /// Replaces the chat text (ignored for other events).
        pub fn set_text(&mut self, text: impl Into<String>) {
            if matches!(self.event, Event::Chat(_)) {
                self.text = Some(text.into());
            }
        }

        /// Whether the event is cancelled (by an earlier handler or this one).
        pub fn cancelled(&self) -> bool {
            self.cancelled
        }

        /// Cancels or un-cancels the event (only chat and command events can be cancelled).
        pub fn set_cancelled(&mut self, cancelled: bool) {
            self.cancelled = cancelled;
        }

        /// Cancels the event.
        pub fn cancel(&mut self) {
            self.set_cancelled(true)
        }

        pub(crate) fn into_outcome(self) -> w::Outcome {
            let changed = match (self.event, self.text) {
                (Event::Chat(c), Some(t)) if t != c.text => Some(w::Mutation::ChatText(t)),
                _ => self.other,
            };
            w::Outcome { cancelled: self.cancelled, mutation: changed }
        }
    }

    /// Subscription options.
    #[derive(Debug, Clone, Copy)]
    pub struct On {
        /// Handler priority (default `normal`).
        pub priority: Priority,
        /// Skip the handler while the event is cancelled.
        pub ignore_cancelled: bool,
    }

    impl Default for On {
        fn default() -> Self {
            On { priority: Priority::Normal, ignore_cancelled: false }
        }
    }

    impl On {
        /// Options with a priority.
        pub fn priority(priority: Priority) -> Self {
            On { priority, ..On::default() }
        }

        /// Skips the handler while the event is cancelled.
        pub fn ignore_cancelled(mut self) -> Self {
            self.ignore_cancelled = true;
            self
        }
    }

    /// Subscribes `f` to `kind` at normal priority.
    pub fn on(kind: EventKind, f: impl FnMut(&mut EventCtx<'_, '_>) + 'static) -> Result<u32, ApiError> {
        on_with(kind, On::default(), f)
    }

    /// Subscribes `f` to `kind` with options.
    pub fn on_with(kind: EventKind, opts: On, f: impl FnMut(&mut EventCtx<'_, '_>) + 'static) -> Result<u32, ApiError> {
        let id = w::subscribe(kind, opts.priority, opts.ignore_cancelled)?;
        with_registry(|r| r.events.insert(id, Some(Box::new(f))));
        Ok(id)
    }

    /// Subscribes `f` to the custom event `name` (`<mod-id>:<event>`); the payload is canonical CBOR.
    pub fn on_custom(name: &str, opts: On, f: impl FnMut(&mut EventCtx<'_, '_>) + 'static) -> Result<u32, ApiError> {
        let id = w::subscribe_custom(name, opts.priority, opts.ignore_cancelled)?;
        with_registry(|r| r.events.insert(id, Some(Box::new(f))));
        Ok(id)
    }

    /// Fires a custom event of this mod (`<own-mod-id>:<event>`); true when it ended up cancelled.
    pub fn fire_custom(name: &str, payload: &[u8]) -> Result<bool, ApiError> {
        w::fire_custom(name, payload)
    }

    /// Removes a subscription.
    pub fn off(id: u32) -> bool {
        with_registry(|r| r.events.remove(&id));
        w::unsubscribe(id)
    }
}

/// Chat and console commands.
pub mod commands {

    use crate::wit::commands as w;
    use crate::wit::events::CommandSender;
    use crate::{with_registry, ApiError, Player, Role};

    /// Describes a command. Defaults match the Luau API: usage `/<name>`, any role, console allowed.
    #[derive(Debug, Clone)]
    pub struct Command(w::CommandSpec);

    impl Command {
        /// A command named `name` (without `/`).
        pub fn new(name: &str) -> Self {
            Command(w::CommandSpec {
                name: name.to_string(),
                aliases: Vec::new(),
                description: String::new(),
                usage: format!("/{name}"),
                min_role: Role::Player,
                console: true,
            })
        }
        /// Adds an alias.
        pub fn alias(mut self, alias: &str) -> Self {
            self.0.aliases.push(alias.to_string());
            self
        }
        /// Sets the one-line help.
        pub fn description(mut self, text: &str) -> Self {
            self.0.description = text.to_string();
            self
        }
        /// Sets the usage text shown when the handler returns [`Reply::usage`].
        pub fn usage(mut self, text: &str) -> Self {
            self.0.usage = text.to_string();
            self
        }
        /// Sets the lowest role allowed from chat.
        pub fn min_role(mut self, role: Role) -> Self {
            self.0.min_role = role;
            self
        }
        /// Allows or forbids the console.
        pub fn console(mut self, allowed: bool) -> Self {
            self.0.console = allowed;
            self
        }
        /// The WIT spec.
        pub fn spec(&self) -> &w::CommandSpec {
            &self.0
        }
    }

    /// Who ran a command.
    pub enum Sender<'p> {
        /// The server console.
        Console,
        /// A player (valid during the handler only; keep `id()` to find them later).
        Player(&'p Player),
    }

    impl<'p> Sender<'p> {
        pub(crate) fn from_wit(s: CommandSender<'p>) -> Self {
            match s {
                CommandSender::Console => Sender::Console,
                CommandSender::Player(p) => Sender::Player(p),
            }
        }

        /// The player, if a player ran the command.
        pub fn player(&self) -> Option<&'p Player> {
            match self {
                Sender::Console => None,
                Sender::Player(p) => Some(p),
            }
        }

        /// The player's alias, or `console`.
        pub fn name(&self) -> String {
            self.player().map_or_else(|| "console".to_string(), |p| p.alias())
        }
    }

    /// What a command handler returns.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Reply {
        /// False shows the usage text after the lines.
        pub handled: bool,
        /// Lines sent back to the sender.
        pub lines: Vec<String>,
    }

    impl Reply {
        /// Handled, nothing to say.
        pub fn ok() -> Self {
            Reply { handled: true, lines: Vec::new() }
        }
        /// Handled, with reply lines.
        pub fn lines<S: Into<String>>(lines: impl IntoIterator<Item = S>) -> Self {
            Reply { handled: true, lines: lines.into_iter().map(Into::into).collect() }
        }
        /// Not handled: the host shows these lines and then the usage text.
        pub fn usage<S: Into<String>>(lines: impl IntoIterator<Item = S>) -> Self {
            Reply { handled: false, lines: lines.into_iter().map(Into::into).collect() }
        }
        pub(crate) fn into_wit(self) -> w::CommandOutcome {
            w::CommandOutcome { handled: self.handled, reply: self.lines }
        }
    }

    /// Registers a command; `f(sender, label, args)` runs when it is used (`label` is the name or alias typed).
    pub fn register(
        cmd: Command,
        f: impl FnMut(&Sender<'_>, &str, &[String]) -> Reply + 'static,
    ) -> Result<u32, ApiError> {
        let id = w::register(&cmd.0)?;
        with_registry(|r| r.commands.insert(id, Some(Box::new(f))));
        Ok(id)
    }

    /// Removes a command.
    pub fn unregister(id: u32) -> bool {
        with_registry(|r| r.commands.remove(&id));
        w::unregister(id)
    }
}

/// Delayed and repeating tasks. One tick is one world step (0.2 s on the server).
pub mod scheduler {

    use crate::wit::scheduler as w;
    use crate::{with_registry, ApiError};

    /// Ticks per second on the server.
    pub const TICKS_PER_SECOND: u32 = 5;

    fn store(id: u64, f: impl FnMut() + 'static, repeat: bool) -> u64 {
        with_registry(|r| r.tasks.insert(id, (Some(Box::new(f)), repeat)));
        id
    }

    /// Runs `f` once after `delay` ticks.
    pub fn run_later(delay: u32, f: impl FnMut() + 'static) -> Result<u64, ApiError> {
        Ok(store(w::run_later(delay)?, f, false))
    }

    /// Runs `f` after `delay` ticks and then every `period` ticks until cancelled.
    pub fn run_every(delay: u32, period: u32, f: impl FnMut() + 'static) -> Result<u64, ApiError> {
        Ok(store(w::run_every(delay, period)?, f, true))
    }

    /// Runs `f` once in the next network poll (about 16 ms).
    pub fn run_next_poll(f: impl FnMut() + 'static) -> Result<u64, ApiError> {
        Ok(store(w::run_next_poll()?, f, false))
    }

    /// Cancels a task (also from inside the task itself).
    pub fn cancel(id: u64) -> bool {
        with_registry(|r| r.tasks.remove(&id));
        w::cancel(id)
    }
}

/// Converts an [`ApiError`] into the `String` error of [`Mod::on_enable`] (same text as the host and Luau use), so
/// `?` works there.
impl From<ApiError> for String {
    fn from(e: ApiError) -> String {
        match e {
            ApiError::NotPermitted(p) => format!("not permitted: {p}"),
            ApiError::InvalidArgument(a) => format!("invalid argument: {a}"),
            ApiError::LimitExceeded(l) => format!("limit exceeded: {l}"),
            ApiError::StaleHandle => "stale handle".into(),
            ApiError::Unsupported => "unsupported".into(),
            ApiError::Internal(m) => format!("internal error: {m}"),
        }
    }
}
