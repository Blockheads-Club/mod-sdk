//! Host-side data types mirroring `blockheads:api@0.1.0` (`types`, `players`, `events`, `commands`, `log`).
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

/// The built-in events of API 0.1 (`events.event-kind`).
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
}

impl EventKind {
    /// All kinds.
    pub const ALL: [EventKind; 7] = [
        EventKind::PlayerJoin,
        EventKind::PlayerQuit,
        EventKind::Chat,
        EventKind::Command,
        EventKind::Tick,
        EventKind::WorldLoaded,
        EventKind::WorldSave,
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
        }
    }

    /// Parses a WIT name.
    pub fn parse(s: &str) -> Option<EventKind> {
        EventKind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// Whether handlers may cancel events of this kind.
    pub fn cancellable(self) -> bool {
        matches!(self, EventKind::Chat | EventKind::Command)
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

/// One event (`events.event`, without `custom`, which arrives with M3).
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
        }
    }

    /// The cancelled flag (always false for events that cannot be cancelled).
    pub fn cancelled(&self) -> bool {
        match self {
            Event::Chat { cancelled, .. } | Event::Command { cancelled, .. } => *cancelled,
            _ => false,
        }
    }

    /// Applies a handler outcome. Returns false when the mutation did not fit the event (it is then ignored).
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
                    Some(Mutation::CustomPayload(_)) => false,
                }
            }
            Event::Command { cancelled, .. } => {
                *cancelled = outcome.cancelled;
                outcome.mutation.is_none()
            }
            _ => outcome.mutation.is_none(),
        }
    }
}

/// A change a handler wants to make (`events.mutation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    /// Replace the chat text.
    ChatText(String),
    /// Replace a custom event payload (M3).
    CustomPayload(Vec<u8>),
}

/// What a handler returns (`events.outcome`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
        assert_eq!(EventKind::parse("block-place"), None);
        assert_eq!(Side::Server.name(), "server");
        assert_eq!(Side::Client.name(), "client");
        assert_eq!(SavePhase::Pre.name(), "pre");
        assert_eq!(SavePhase::Post.name(), "post");
        assert!(Priority::Lowest < Priority::Monitor);
        assert!(Role::Player < Role::Owner);
    }

    #[test]
    fn cancellable_kinds() {
        let c: Vec<_> = EventKind::ALL.into_iter().filter(|k| k.cancellable()).collect();
        assert_eq!(c, vec![EventKind::Chat, EventKind::Command]);
    }

    fn all_events() -> Vec<Event> {
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
        ]
    }

    #[test]
    fn event_kind_and_apply() {
        for (ev, kind) in all_events().into_iter().zip(EventKind::ALL) {
            assert_eq!(ev.kind(), kind);
            assert!(!ev.cancelled());
        }
        let mut chat = all_events().remove(2);
        assert!(chat.apply(Outcome { cancelled: true, mutation: Some(Mutation::ChatText("x".into())) }));
        assert!(chat.cancelled());
        assert!(matches!(&chat, Event::Chat { text, .. } if text == "x"));
        assert!(chat.apply(Outcome { cancelled: false, mutation: None }));
        assert!(!chat.cancelled());
        assert!(!chat.apply(Outcome { cancelled: false, mutation: Some(Mutation::CustomPayload(vec![])) }));

        let mut cmd = all_events().remove(3);
        assert!(cmd.apply(Outcome { cancelled: true, mutation: None }));
        assert!(cmd.cancelled());
        assert!(!cmd.apply(Outcome { cancelled: true, mutation: Some(Mutation::ChatText("x".into())) }));

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
    fn api_error_display() {
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
    }
}
