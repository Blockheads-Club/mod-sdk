//! Host-side data types mirroring `blockheads:api@0.2.0` (`types`, `players`, `events`, `commands`, `services`,
//! `channels`, `log`).
//!
//! They are plain Rust data so every runtime (Luau now, WASM in M2, JVM in M9) converts from and to the same shapes.

use std::fmt;

/// Event handler priority in Spigot order (`types.priority`). `Ord` follows the run order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Runs first.
    Lowest,
    /// After `lowest`.
    Low,
    /// The default.
    Normal,
    /// After `normal`.
    High,
    /// Last handler that may still cancel or modify.
    Highest,
    /// Sees the final state; its outcome is ignored.
    Monitor,
}

impl Priority {
    /// All priorities in run order.
    pub const ALL: [Priority; 6] =
        [Priority::Lowest, Priority::Low, Priority::Normal, Priority::High, Priority::Highest, Priority::Monitor];

    /// The WIT name (`"normal"`).
    pub fn name(self) -> &'static str {
        match self {
            Priority::Lowest => "lowest",
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Highest => "highest",
            Priority::Monitor => "monitor",
        }
    }

    /// Parses a WIT name.
    pub fn parse(s: &str) -> Option<Priority> {
        Priority::ALL.into_iter().find(|p| p.name() == s)
    }
}

/// A player's role on the server (`players.role`). `Ord` follows the rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// A normal player.
    Player,
    /// On the moderator list.
    Moderator,
    /// On the admin list.
    Admin,
    /// The server owner / console operator.
    Owner,
}

impl Role {
    /// All roles from lowest to highest.
    pub const ALL: [Role; 4] = [Role::Player, Role::Moderator, Role::Admin, Role::Owner];

    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            Role::Player => "player",
            Role::Moderator => "moderator",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }

    /// Parses a WIT name.
    pub fn parse(s: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.name() == s)
    }
}

/// A kind of content a mod declares in `[content]` (`content.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentKind {
    /// A new block.
    Block,
    /// A new item.
    Item,
    /// A new cosmetic.
    Cosmetic,
}

impl ContentKind {
    /// All kinds.
    pub const ALL: [ContentKind; 3] = [ContentKind::Block, ContentKind::Item, ContentKind::Cosmetic];

    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            ContentKind::Block => "block",
            ContentKind::Item => "item",
            ContentKind::Cosmetic => "cosmetic",
        }
    }

    /// Parses a WIT name.
    pub fn parse(s: &str) -> Option<ContentKind> {
        ContentKind::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// Which program the host is (`types.side`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Dedicated server or the host of a local world.
    Server,
    /// A game client.
    Client,
}

impl Side {
    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            Side::Server => "server",
            Side::Client => "client",
        }
    }
}

/// Log severity (`log.level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// Very detailed.
    Trace,
    /// Diagnostic.
    Debug,
    /// Normal.
    Info,
    /// Something looks wrong.
    Warn,
    /// Something failed.
    Error,
}

impl LogLevel {
    /// All levels from lowest to highest.
    pub const ALL: [LogLevel; 5] = [LogLevel::Trace, LogLevel::Debug, LogLevel::Info, LogLevel::Warn, LogLevel::Error];

    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }

    /// Parses a WIT name.
    pub fn parse(s: &str) -> Option<LogLevel> {
        LogLevel::ALL.into_iter().find(|l| l.name() == s)
    }
}

/// The event kinds (`events.event-kind`, plus [`EventKind::Custom`] for `subscribe-custom`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    /// A player finished joining.
    PlayerJoin,
    /// A player left or was kicked.
    PlayerQuit,
    /// A chat message (cancellable, text replaceable).
    Chat,
    /// A `/command` line (cancellable).
    Command,
    /// One world step.
    Tick,
    /// The world finished loading.
    WorldLoaded,
    /// The world is being saved.
    WorldSave,
    /// A player passed every vanilla join check and is about to be approved (cancellable with a reason). API 0.2.
    PlayerPreLogin,
    /// A player places a block or item into a tile (cancellable). API 0.2.
    BlockPlace,
    /// A player removes a block or back wall (cancellable). API 0.2.
    BlockBreak,
    /// A tile changed, by a player or by the simulation (monitor only). API 0.2.
    BlockChanged,
    /// A player places a workbench or an interaction object such as a chest (cancellable). API 0.2.
    ObjectPlace,
    /// A blockhead is about to take damage (cancellable, amount replaceable). API 0.2.
    Damage,
    /// A blockhead died. API 0.2.
    Death,
    /// A player's blockhead moved to another tile. API 0.2.
    PlayerMove,
    /// A custom event fired by a mod (subscribed by name with `subscribe-custom`).
    Custom,
}

impl EventKind {
    /// All kinds that `subscribe` accepts, in WIT order (everything but [`EventKind::Custom`]).
    pub const ALL: [EventKind; 15] = [
        EventKind::PlayerJoin,
        EventKind::PlayerQuit,
        EventKind::Chat,
        EventKind::Command,
        EventKind::Tick,
        EventKind::WorldLoaded,
        EventKind::WorldSave,
        EventKind::PlayerPreLogin,
        EventKind::BlockPlace,
        EventKind::BlockBreak,
        EventKind::BlockChanged,
        EventKind::ObjectPlace,
        EventKind::Damage,
        EventKind::Death,
        EventKind::PlayerMove,
    ];

    /// The WIT name (`"player-join"`).
    pub fn name(self) -> &'static str {
        match self {
            EventKind::PlayerJoin => "player-join",
            EventKind::PlayerQuit => "player-quit",
            EventKind::Chat => "chat",
            EventKind::Command => "command",
            EventKind::Tick => "tick",
            EventKind::WorldLoaded => "world-loaded",
            EventKind::WorldSave => "world-save",
            EventKind::PlayerPreLogin => "player-pre-login",
            EventKind::BlockPlace => "block-place",
            EventKind::BlockBreak => "block-break",
            EventKind::BlockChanged => "block-changed",
            EventKind::ObjectPlace => "object-place",
            EventKind::Damage => "damage",
            EventKind::Death => "death",
            EventKind::PlayerMove => "player-move",
            EventKind::Custom => "custom",
        }
    }

    /// Parses a WIT name of a subscribable kind (`"custom"` is not one).
    pub fn parse(s: &str) -> Option<EventKind> {
        EventKind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// Whether handlers may cancel events of this kind.
    pub fn cancellable(self) -> bool {
        matches!(
            self,
            EventKind::Chat
                | EventKind::Command
                | EventKind::PlayerPreLogin
                | EventKind::BlockPlace
                | EventKind::BlockBreak
                | EventKind::ObjectPlace
                | EventKind::Damage
                | EventKind::Custom
        )
    }

    /// The first API minor version that has this kind (`(0, 1)` or `(0, 2)`).
    pub fn since(self) -> (u64, u64) {
        match self {
            EventKind::PlayerJoin
            | EventKind::PlayerQuit
            | EventKind::Chat
            | EventKind::Command
            | EventKind::Tick
            | EventKind::WorldLoaded
            | EventKind::WorldSave
            | EventKind::Custom => (0, 1),
            _ => (0, 2),
        }
    }
}

/// A tile position (`types.pos`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Pos {
    /// Column; wraps around the world width.
    pub x: i32,
    /// Row from bedrock upwards.
    pub y: i32,
}

/// A vanilla rejection reason a `player-pre-login` handler may use (`events.reject-reason`). Vanilla clients show
/// their own localized text for each code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectReason {
    /// 3: the client is too old.
    ClientTooOld,
    /// 5: the server is full.
    ServerFull,
    /// 6: blacklisted.
    Blacklisted,
    /// 7: not on the whitelist.
    NotWhitelisted,
    /// 10: wrong password.
    IncorrectPassword,
    /// 11: kicked (the default).
    Kicked,
    /// 12: banned.
    Banned,
    /// 14: invalid user name.
    InvalidUsername,
    /// 15: the server is still loading.
    StillLoading,
}

impl RejectReason {
    /// All reasons.
    pub const ALL: [RejectReason; 9] = [
        RejectReason::ClientTooOld,
        RejectReason::ServerFull,
        RejectReason::Blacklisted,
        RejectReason::NotWhitelisted,
        RejectReason::IncorrectPassword,
        RejectReason::Kicked,
        RejectReason::Banned,
        RejectReason::InvalidUsername,
        RejectReason::StillLoading,
    ];

    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            RejectReason::ClientTooOld => "client-too-old",
            RejectReason::ServerFull => "server-full",
            RejectReason::Blacklisted => "blacklisted",
            RejectReason::NotWhitelisted => "not-whitelisted",
            RejectReason::IncorrectPassword => "incorrect-password",
            RejectReason::Kicked => "kicked",
            RejectReason::Banned => "banned",
            RejectReason::InvalidUsername => "invalid-username",
            RejectReason::StillLoading => "still-loading",
        }
    }

    /// Parses a WIT name.
    pub fn parse(s: &str) -> Option<RejectReason> {
        RejectReason::ALL.into_iter().find(|r| r.name() == s)
    }

    /// The vanilla 0x26 reason byte.
    pub fn code(self) -> u8 {
        match self {
            RejectReason::ClientTooOld => 3,
            RejectReason::ServerFull => 5,
            RejectReason::Blacklisted => 6,
            RejectReason::NotWhitelisted => 7,
            RejectReason::IncorrectPassword => 10,
            RejectReason::Kicked => 11,
            RejectReason::Banned => 12,
            RejectReason::InvalidUsername => 14,
            RejectReason::StillLoading => 15,
        }
    }
}

/// Which layer a `block-break` removes (`events.break-layer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BreakLayer {
    /// The tile itself, its contents or water.
    Block,
    /// The back wall.
    BackWall,
}

impl BreakLayer {
    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            BreakLayer::Block => "block",
            BreakLayer::BackWall => "back-wall",
        }
    }
}

/// What an `object-place` places (`events.object-kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    /// A workbench (crafting station); `item` is the workbench type.
    Workbench,
    /// An interaction object (chest, sign, door, bed, ...); `item` is its item type.
    InteractionObject,
}

impl ObjectKind {
    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            ObjectKind::Workbench => "workbench",
            ObjectKind::InteractionObject => "interaction-object",
        }
    }
}

/// `events.save-phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SavePhase {
    /// Before the save.
    Pre,
    /// After the save.
    Post,
}

impl SavePhase {
    /// The WIT name.
    pub fn name(self) -> &'static str {
        match self {
            SavePhase::Pre => "pre",
            SavePhase::Post => "post",
        }
    }
}

/// Who issued a command (`events.command-sender`). Players are referred to by their player id (the host handle).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CommandSender {
    /// The server console.
    Console,
    /// A player, by player id.
    Player(String),
}

/// One event (`events.event`).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// `player-join`.
    PlayerJoin {
        /// Player id.
        player: String,
    },
    /// `player-quit`.
    PlayerQuit {
        /// Player id.
        player: String,
        /// Kicked or banned rather than leaving.
        was_kick: bool,
    },
    /// `chat`.
    Chat {
        /// Player id.
        player: String,
        /// Message text (after earlier handlers' changes).
        text: String,
        /// Cancelled by an earlier handler.
        cancelled: bool,
    },
    /// `command`.
    Command {
        /// Who typed it.
        sender: CommandSender,
        /// Command name without `/`, lower-cased.
        name: String,
        /// The rest of the line split on spaces.
        args: Vec<String>,
        /// The full line.
        raw: String,
        /// Cancelled by an earlier handler.
        cancelled: bool,
    },
    /// `tick`.
    Tick {
        /// Steps since the world loaded, from 1.
        tick: u64,
        /// Step length in seconds.
        dt: f32,
    },
    /// `world-loaded`.
    WorldLoaded {
        /// Display name.
        world_name: String,
        /// Save id.
        world_id: String,
    },
    /// `world-save`.
    WorldSave {
        /// Before or after.
        phase: SavePhase,
    },
    /// `player-pre-login`. The player is not online yet: use `player` as an id, not as a handle.
    PlayerPreLogin {
        /// Player id.
        player: String,
        /// Alias the player joins with.
        alias: String,
        /// Cancelled (= rejected) by an earlier handler.
        cancelled: bool,
        /// The reason sent when the event ends up cancelled.
        reason: RejectReason,
    },
    /// `block-place`.
    BlockPlace {
        /// Player id.
        player: String,
        /// Target tile.
        pos: Pos,
        /// Item type being placed.
        item: u32,
        /// Item data A (for example the paint or the rotation).
        data_a: u16,
        /// Item data B.
        data_b: u16,
        /// Cancelled by an earlier handler.
        cancelled: bool,
    },
    /// `block-break`.
    BlockBreak {
        /// Player id.
        player: String,
        /// Target tile.
        pos: Pos,
        /// Tile or back wall.
        layer: BreakLayer,
        /// The tile type before the removal.
        tile: u32,
        /// Cancelled by an earlier handler.
        cancelled: bool,
    },
    /// `block-changed` (monitor only).
    BlockChanged {
        /// The tile.
        pos: Pos,
        /// The new tile type.
        tile: u32,
        /// The player who caused it, when known.
        player: Option<String>,
    },
    /// `object-place`.
    ObjectPlace {
        /// Player id.
        player: String,
        /// Target tile.
        pos: Pos,
        /// What is placed.
        object: ObjectKind,
        /// Workbench type or item type.
        item: u32,
        /// Cancelled by an earlier handler.
        cancelled: bool,
    },
    /// `damage`.
    Damage {
        /// The owning player of the damaged blockhead.
        player: Option<String>,
        /// The blockhead's unique id.
        blockhead: u64,
        /// Damage (1.0 = full health), after earlier handlers' changes.
        amount: f32,
        /// The attacking player for PvP damage.
        attacker: Option<String>,
        /// Cancelled by an earlier handler.
        cancelled: bool,
    },
    /// `death`.
    Death {
        /// The owning player of the blockhead.
        player: Option<String>,
        /// The blockhead's unique id.
        blockhead: u64,
    },
    /// `player-move`: the player's blockhead entered another tile.
    PlayerMove {
        /// Player id.
        player: String,
        /// Previous tile.
        from: Pos,
        /// New tile.
        to: Pos,
    },
    /// `custom`.
    Custom {
        /// `<mod-id>:<name>`.
        name: String,
        /// The firing mod.
        source: String,
        /// Canonical CBOR payload.
        payload: Vec<u8>,
        /// Cancelled by an earlier handler.
        cancelled: bool,
    },
}

impl Event {
    /// The event's kind.
    pub fn kind(&self) -> EventKind {
        match self {
            Event::PlayerJoin { .. } => EventKind::PlayerJoin,
            Event::PlayerQuit { .. } => EventKind::PlayerQuit,
            Event::Chat { .. } => EventKind::Chat,
            Event::Command { .. } => EventKind::Command,
            Event::Tick { .. } => EventKind::Tick,
            Event::WorldLoaded { .. } => EventKind::WorldLoaded,
            Event::WorldSave { .. } => EventKind::WorldSave,
            Event::PlayerPreLogin { .. } => EventKind::PlayerPreLogin,
            Event::BlockPlace { .. } => EventKind::BlockPlace,
            Event::BlockBreak { .. } => EventKind::BlockBreak,
            Event::BlockChanged { .. } => EventKind::BlockChanged,
            Event::ObjectPlace { .. } => EventKind::ObjectPlace,
            Event::Damage { .. } => EventKind::Damage,
            Event::Death { .. } => EventKind::Death,
            Event::PlayerMove { .. } => EventKind::PlayerMove,
            Event::Custom { .. } => EventKind::Custom,
        }
    }

    /// The cancelled flag (always false for events that cannot be cancelled).
    pub fn cancelled(&self) -> bool {
        match self {
            Event::Chat { cancelled, .. }
            | Event::Command { cancelled, .. }
            | Event::PlayerPreLogin { cancelled, .. }
            | Event::BlockPlace { cancelled, .. }
            | Event::BlockBreak { cancelled, .. }
            | Event::ObjectPlace { cancelled, .. }
            | Event::Damage { cancelled, .. }
            | Event::Custom { cancelled, .. } => *cancelled,
            _ => false,
        }
    }

    /// Applies a handler outcome. Returns false when the mutation did not fit the event (it is then ignored; the
    /// cancelled flag still applies).
    pub fn apply(&mut self, outcome: Outcome) -> bool {
        match self {
            Event::Chat { cancelled, text, .. } => {
                *cancelled = outcome.cancelled;
                match outcome.mutation {
                    None => true,
                    Some(Mutation::ChatText(t)) => {
                        *text = t;
                        true
                    }
                    Some(_) => false,
                }
            }
            Event::PlayerPreLogin { cancelled, reason, .. } => {
                *cancelled = outcome.cancelled;
                match outcome.mutation {
                    None => true,
                    Some(Mutation::RejectReason(r)) => {
                        *reason = r;
                        true
                    }
                    Some(_) => false,
                }
            }
            Event::Damage { cancelled, amount, .. } => {
                *cancelled = outcome.cancelled;
                match outcome.mutation {
                    None => true,
                    Some(Mutation::DamageAmount(a)) if a.is_finite() && a >= 0.0 => {
                        *amount = a;
                        true
                    }
                    Some(_) => false,
                }
            }
            Event::Custom { cancelled, payload, .. } => {
                *cancelled = outcome.cancelled;
                match outcome.mutation {
                    None => true,
                    Some(Mutation::CustomPayload(p)) if crate::value::Value::decode(&p).is_ok() => {
                        *payload = p;
                        true
                    }
                    Some(_) => false,
                }
            }
            Event::Command { cancelled, .. }
            | Event::BlockPlace { cancelled, .. }
            | Event::BlockBreak { cancelled, .. }
            | Event::ObjectPlace { cancelled, .. } => {
                *cancelled = outcome.cancelled;
                outcome.mutation.is_none()
            }
            _ => outcome.mutation.is_none(),
        }
    }
}

/// A change a handler wants to make (`events.mutation`).
#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    /// Replace the chat text.
    ChatText(String),
    /// Replace a custom event payload (canonical CBOR).
    CustomPayload(Vec<u8>),
    /// The rejection reason of a `player-pre-login` (API 0.2).
    RejectReason(RejectReason),
    /// The damage amount (finite, >= 0; API 0.2).
    DamageAmount(f32),
}

/// What a handler returns (`events.outcome`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    /// The new cancelled state.
    pub cancelled: bool,
    /// An optional change.
    pub mutation: Option<Mutation>,
}

/// `commands.command-spec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    /// Name without `/`.
    pub name: String,
    /// Extra names.
    pub aliases: Vec<String>,
    /// One line of help.
    pub description: String,
    /// Usage text shown when the handler returns `handled = false`.
    pub usage: String,
    /// Lowest role allowed from chat.
    pub min_role: Role,
    /// Whether the console may run it.
    pub console: bool,
}

impl CommandSpec {
    /// A spec with the Luau defaults (`min_role = player`, `console = true`, no aliases).
    pub fn new(name: &str) -> CommandSpec {
        CommandSpec {
            name: name.to_string(),
            aliases: Vec::new(),
            description: String::new(),
            usage: format!("/{name}"),
            min_role: Role::Player,
            console: true,
        }
    }
}

/// `commands.command-outcome`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandOutcome {
    /// False shows the usage text.
    pub handled: bool,
    /// Lines sent back to the sender.
    pub reply: Vec<String>,
}

/// `types.api-error`: errors returned by host functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// Missing permission (named).
    NotPermitted(String),
    /// Malformed argument.
    InvalidArgument(String),
    /// A size or count limit was hit.
    LimitExceeded(String),
    /// The handle no longer exists.
    StaleHandle,
    /// Not available on this side / in this host version.
    Unsupported,
    /// Host failure.
    Internal(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::NotPermitted(p) => write!(f, "not permitted: {p}"),
            ApiError::InvalidArgument(a) => write!(f, "invalid argument: {a}"),
            ApiError::LimitExceeded(l) => write!(f, "limit exceeded: {l}"),
            ApiError::StaleHandle => f.write_str("stale handle"),
            ApiError::Unsupported => f.write_str("unsupported"),
            ApiError::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// `services.call-error`: why a service call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// No provider with that name is loaded.
    NotFound,
    /// Providers exist but none matches the version requirement.
    VersionMismatch,
    /// The caller does not depend (or soft-depend) on the provider.
    NotPermitted,
    /// The call would re-enter a mod that is already on the call stack.
    Reentrant,
    /// The provider returned an error.
    ProviderError(String),
    /// The provider trapped or ran over its time budget.
    Trap,
    /// Arguments or result were not valid canonical CBOR (or the arguments not a list).
    BadValue(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::NotFound => f.write_str("not-found"),
            ServiceError::VersionMismatch => f.write_str("version-mismatch"),
            ServiceError::NotPermitted => f.write_str("not-permitted"),
            ServiceError::Reentrant => f.write_str("reentrant"),
            ServiceError::ProviderError(m) => write!(f, "provider-error: {m}"),
            ServiceError::Trap => f.write_str("trap"),
            ServiceError::BadValue(m) => write!(f, "bad-value: {m}"),
        }
    }
}

impl std::error::Error for ServiceError {}

/// `channels.target`: where a channel message goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChannelTarget {
    /// Every other mod on this side that opened the same channel name.
    Local,
    /// The server (from a client mod).
    Server,
    /// One player's client (from a server mod), by player id.
    Player(String),
    /// Every connected client running the mod (from a server mod).
    AllPlayers,
}

/// `channels.source`: where a received channel message came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChannelSource {
    /// Another mod on the same side (its id).
    Local(String),
    /// The server.
    Server,
    /// A player's client, by player id.
    Player(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for p in Priority::ALL {
            assert_eq!(Priority::parse(p.name()), Some(p));
        }
        assert_eq!(Priority::parse("urgent"), None);
        for r in Role::ALL {
            assert_eq!(Role::parse(r.name()), Some(r));
        }
        assert_eq!(Role::parse("god"), None);
        for l in LogLevel::ALL {
            assert_eq!(LogLevel::parse(l.name()), Some(l));
        }
        assert_eq!(LogLevel::parse("fatal"), None);
        for k in EventKind::ALL {
            assert_eq!(EventKind::parse(k.name()), Some(k));
        }
        assert_eq!(EventKind::parse("custom"), None);
        assert_eq!(EventKind::Custom.name(), "custom");
        assert_eq!(EventKind::parse("explode"), None);
        for r in RejectReason::ALL {
            assert_eq!(RejectReason::parse(r.name()), Some(r));
        }
        assert_eq!(RejectReason::parse("nope"), None);
        let codes: Vec<u8> = RejectReason::ALL.iter().map(|r| r.code()).collect();
        assert_eq!(codes, vec![3, 5, 6, 7, 10, 11, 12, 14, 15]);
        assert_eq!(Side::Server.name(), "server");
        assert_eq!(Side::Client.name(), "client");
        assert_eq!(SavePhase::Pre.name(), "pre");
        assert_eq!(SavePhase::Post.name(), "post");
        assert_eq!(BreakLayer::Block.name(), "block");
        assert_eq!(BreakLayer::BackWall.name(), "back-wall");
        assert_eq!(ObjectKind::Workbench.name(), "workbench");
        assert_eq!(ObjectKind::InteractionObject.name(), "interaction-object");
        assert!(Priority::Lowest < Priority::Monitor);
        assert!(Role::Player < Role::Owner);
    }

    #[test]
    fn cancellable_kinds_and_versions() {
        let c: Vec<_> = EventKind::ALL.into_iter().filter(|k| k.cancellable()).collect();
        use EventKind::*;
        assert_eq!(c, vec![Chat, Command, PlayerPreLogin, BlockPlace, BlockBreak, ObjectPlace, Damage]);
        assert!(Custom.cancellable());
        assert_eq!(Tick.since(), (0, 1));
        assert_eq!(Custom.since(), (0, 1));
        assert_eq!(Damage.since(), (0, 2));
    }

    fn all_events() -> Vec<Event> {
        let pos = Pos { x: 1, y: 2 };
        vec![
            Event::PlayerJoin { player: "p".into() },
            Event::PlayerQuit { player: "p".into(), was_kick: true },
            Event::Chat { player: "p".into(), text: "hi".into(), cancelled: false },
            Event::Command {
                sender: CommandSender::Console,
                name: "x".into(),
                args: vec![],
                raw: "/x".into(),
                cancelled: false,
            },
            Event::Tick { tick: 1, dt: 0.2 },
            Event::WorldLoaded { world_name: "W".into(), world_id: "id".into() },
            Event::WorldSave { phase: SavePhase::Pre },
            Event::PlayerPreLogin {
                player: "p".into(),
                alias: "A".into(),
                cancelled: false,
                reason: RejectReason::Kicked,
            },
            Event::BlockPlace { player: "p".into(), pos, item: 1, data_a: 0, data_b: 0, cancelled: false },
            Event::BlockBreak { player: "p".into(), pos, layer: BreakLayer::Block, tile: 1, cancelled: false },
            Event::BlockChanged { pos, tile: 2, player: None },
            Event::ObjectPlace { player: "p".into(), pos, object: ObjectKind::Workbench, item: 1, cancelled: false },
            Event::Damage { player: None, blockhead: 1, amount: 0.5, attacker: None, cancelled: false },
            Event::Death { player: None, blockhead: 1 },
            Event::PlayerMove { player: "p".into(), from: pos, to: pos },
        ]
    }

    fn custom() -> Event {
        Event::Custom { name: "m:x".into(), source: "m".into(), payload: vec![0xf6], cancelled: false }
    }

    #[test]
    fn event_kind_and_apply() {
        for (ev, kind) in all_events().into_iter().zip(EventKind::ALL) {
            assert_eq!(ev.kind(), kind);
            assert!(!ev.cancelled());
        }
        assert_eq!(custom().kind(), EventKind::Custom);
        let mut chat = all_events().remove(2);
        assert!(chat.apply(Outcome { cancelled: true, mutation: Some(Mutation::ChatText("x".into())) }));
        assert!(chat.cancelled());
        assert!(matches!(&chat, Event::Chat { text, .. } if text == "x"));
        assert!(chat.apply(Outcome { cancelled: false, mutation: None }));
        assert!(!chat.cancelled());
        assert!(!chat.apply(Outcome { cancelled: false, mutation: Some(Mutation::CustomPayload(vec![])) }));

        for i in [3, 8, 9, 11] {
            let mut ev = all_events().remove(i);
            assert!(ev.apply(Outcome { cancelled: true, mutation: None }));
            assert!(ev.cancelled());
            assert!(!ev.apply(Outcome { cancelled: true, mutation: Some(Mutation::ChatText("x".into())) }));
        }

        let mut pre = all_events().remove(7);
        assert!(pre.apply(Outcome { cancelled: true, mutation: Some(Mutation::RejectReason(RejectReason::Banned)) }));
        assert!(matches!(pre, Event::PlayerPreLogin { cancelled: true, reason: RejectReason::Banned, .. }));
        assert!(pre.apply(Outcome { cancelled: true, mutation: None }));
        assert!(!pre.apply(Outcome { cancelled: true, mutation: Some(Mutation::DamageAmount(1.0)) }));

        let mut dmg = all_events().remove(12);
        assert!(dmg.apply(Outcome { cancelled: false, mutation: Some(Mutation::DamageAmount(0.25)) }));
        assert!(matches!(dmg, Event::Damage { amount, .. } if amount == 0.25));
        assert!(!dmg.apply(Outcome { cancelled: false, mutation: Some(Mutation::DamageAmount(-1.0)) }));
        assert!(!dmg.apply(Outcome { cancelled: false, mutation: Some(Mutation::DamageAmount(f32::NAN)) }));
        assert!(dmg.apply(Outcome { cancelled: true, mutation: None }));
        assert!(dmg.cancelled());

        let mut c = custom();
        assert!(c.apply(Outcome { cancelled: true, mutation: Some(Mutation::CustomPayload(vec![0x01])) }));
        assert!(matches!(&c, Event::Custom { payload, cancelled: true, .. } if payload == &[0x01]));
        assert!(!c.apply(Outcome { cancelled: true, mutation: Some(Mutation::CustomPayload(vec![0xff])) }));
        assert!(c.apply(Outcome { cancelled: false, mutation: None }));
        assert!(!c.cancelled());

        let mut tick = all_events().remove(4);
        assert!(tick.apply(Outcome::default()));
        assert!(!tick.apply(Outcome { cancelled: true, mutation: Some(Mutation::ChatText("x".into())) }));
        assert!(!tick.cancelled());
    }

    #[test]
    fn command_spec_defaults() {
        let s = CommandSpec::new("hello");
        assert_eq!(s.usage, "/hello");
        assert_eq!(s.min_role, Role::Player);
        assert!(s.console);
        assert!(s.aliases.is_empty());
    }

    #[test]
    fn error_display() {
        let cases = [
            (ApiError::NotPermitted("players:kick".into()), "not permitted: players:kick"),
            (ApiError::InvalidArgument("key".into()), "invalid argument: key"),
            (ApiError::LimitExceeded("value".into()), "limit exceeded: value"),
            (ApiError::StaleHandle, "stale handle"),
            (ApiError::Unsupported, "unsupported"),
            (ApiError::Internal("x".into()), "internal error: x"),
        ];
        for (e, s) in cases {
            assert_eq!(e.to_string(), s);
        }
        let cases = [
            (ServiceError::NotFound, "not-found"),
            (ServiceError::VersionMismatch, "version-mismatch"),
            (ServiceError::NotPermitted, "not-permitted"),
            (ServiceError::Reentrant, "reentrant"),
            (ServiceError::ProviderError("x".into()), "provider-error: x"),
            (ServiceError::Trap, "trap"),
            (ServiceError::BadValue("y".into()), "bad-value: y"),
        ];
        for (e, s) in cases {
            assert_eq!(e.to_string(), s);
        }
    }
}
