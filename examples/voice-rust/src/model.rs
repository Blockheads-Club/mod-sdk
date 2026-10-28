//! Plain types shared by the policy modules: talk modes, activations, roles, voice states and overhead icons. They
//! mirror the WIT enums of `blockheads:api@0.3.4`; the WASM glue maps between the two.

/// A talk mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    /// Players nearby.
    Proximity,
    /// Everyone.
    Global,
    /// The speaker's channel.
    Channel,
    /// Admin broadcast.
    Broadcast,
}

impl Mode {
    /// Every mode, in bit order.
    pub const ALL: [Mode; 4] = [Mode::Proximity, Mode::Global, Mode::Channel, Mode::Broadcast];

    /// The WIT / settings name.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Proximity => "proximity",
            Mode::Global => "global",
            Mode::Channel => "channel",
            Mode::Broadcast => "broadcast",
        }
    }

    /// Reads a mode name (`channels` is accepted for `channel`).
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "proximity" => Some(Mode::Proximity),
            "global" => Some(Mode::Global),
            "channel" | "channels" => Some(Mode::Channel),
            "broadcast" => Some(Mode::Broadcast),
            _ => None,
        }
    }

    /// The bit of this mode in a [`ModeSet`] (and in `VOICE_STATE.modes`).
    pub fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// The push-to-talk key binding action of this mode.
    pub fn action(self) -> &'static str {
        match self {
            Mode::Proximity => "talk-proximity",
            Mode::Global => "talk-global",
            Mode::Channel => "talk-channel",
            Mode::Broadcast => "talk-broadcast",
        }
    }

    /// The mode of a push-to-talk action.
    pub fn from_action(action: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.action() == action)
    }

    /// The default PC key (`H` is the game's "hide buttons", so broadcast is `J`).
    pub fn default_key(self) -> &'static str {
        match self {
            Mode::Proximity => "v",
            Mode::Global => "g",
            Mode::Channel => "b",
            Mode::Broadcast => "j",
        }
    }

    /// A short English label (dialog buttons, at most 24 bytes).
    pub fn label(self) -> &'static str {
        match self {
            Mode::Proximity => "Nearby",
            Mode::Global => "Everyone",
            Mode::Channel => "Channel",
            Mode::Broadcast => "Broadcast",
        }
    }
}

/// A set of modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeSet(pub u8);

impl ModeSet {
    /// No mode.
    pub const EMPTY: ModeSet = ModeSet(0);

    /// True when `m` is in the set.
    pub fn has(self, m: Mode) -> bool {
        self.0 & m.bit() != 0
    }

    /// Adds `m`.
    pub fn with(self, m: Mode) -> ModeSet {
        ModeSet(self.0 | m.bit())
    }

    /// True for the empty set.
    pub fn is_empty(self) -> bool {
        self.0 & 0x0f == 0
    }

    /// The modes in bit order.
    pub fn modes(self) -> Vec<Mode> {
        Mode::ALL.into_iter().filter(|m| self.has(*m)).collect()
    }

    /// The set of the given modes.
    pub fn of(modes: &[Mode]) -> ModeSet {
        modes.iter().fold(ModeSet::EMPTY, |s, m| s.with(*m))
    }
}

/// How a player starts talking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// Push to talk.
    Ptt,
    /// Voice activation.
    Vad,
    /// One press on, the next off.
    Toggle,
}

impl Activation {
    /// Every activation, in bit order.
    pub const ALL: [Activation; 3] = [Activation::Ptt, Activation::Vad, Activation::Toggle];

    /// The WIT / settings name.
    pub fn name(self) -> &'static str {
        match self {
            Activation::Ptt => "ptt",
            Activation::Vad => "vad",
            Activation::Toggle => "toggle",
        }
    }

    /// Reads an activation name.
    pub fn parse(s: &str) -> Option<Activation> {
        Activation::ALL.into_iter().find(|a| a.name() == s.trim().to_ascii_lowercase())
    }

    /// The bit of this activation in an [`ActSet`].
    pub fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// A set of activations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActSet(pub u8);

impl ActSet {
    /// True when `a` is in the set.
    pub fn has(self, a: Activation) -> bool {
        self.0 & a.bit() != 0
    }

    /// Adds `a`.
    pub fn with(self, a: Activation) -> ActSet {
        ActSet(self.0 | a.bit())
    }

    /// The activations in bit order.
    pub fn list(self) -> Vec<Activation> {
        Activation::ALL.into_iter().filter(|a| self.has(*a)).collect()
    }
}

/// A vanilla server role, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Everyone.
    Player,
    /// In the moderator list.
    Moderator,
    /// In the admin list.
    Admin,
    /// The owner or the console.
    Owner,
}

/// A player's voice state (`VOICE_STATE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No voice (vanilla client, voice not loaded, no session yet).
    None,
    /// Can talk and hear.
    Ready,
    /// Talking now.
    Talking,
    /// Muted their own microphone.
    SelfMuted,
    /// Hears nobody.
    Deafened,
    /// Muted by an admin.
    AdminMuted,
}

impl State {
    /// Reads the state name of a `voice:state` event (`self-muted`, ...).
    pub fn parse(s: &str) -> Option<State> {
        match s {
            "none" => Some(State::None),
            "ready" => Some(State::Ready),
            "talking" => Some(State::Talking),
            "self-muted" => Some(State::SelfMuted),
            "deafened" => Some(State::Deafened),
            "admin-muted" => Some(State::AdminMuted),
            _ => None,
        }
    }

    /// The overhead icon for this state (`None` = no icon).
    pub fn icon(self) -> Option<Icon> {
        match self {
            State::None => Some(Icon::NoVoice),
            State::Ready => None,
            State::Talking => Some(Icon::Speaking),
            State::SelfMuted => Some(Icon::SelfMute),
            State::Deafened => Some(Icon::Deafen),
            State::AdminMuted => Some(Icon::AdminMute),
        }
    }
}

/// An overhead icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Talking.
    Speaking,
    /// No voice chat (crossed headphones).
    NoVoice,
    /// Self mute.
    SelfMute,
    /// Deafened.
    Deafen,
    /// Admin mute.
    AdminMute,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.name()), Some(m));
            assert_eq!(Mode::from_action(m.action()), Some(m));
            assert_eq!(m.default_key().len(), 1);
            assert!(m.label().len() <= 24);
        }
        assert_eq!(Mode::parse(" Channels "), Some(Mode::Channel));
        assert_eq!(Mode::parse("team"), None);
        assert_eq!(Mode::from_action("menu"), None);
        let keys: Vec<_> = Mode::ALL.iter().map(|m| m.default_key()).collect();
        assert_eq!(keys, ["v", "g", "b", "j"]);
    }

    #[test]
    fn mode_sets() {
        let s = ModeSet::of(&[Mode::Global, Mode::Broadcast]);
        assert_eq!(s.0, 0b1010);
        assert!(s.has(Mode::Global) && !s.has(Mode::Channel));
        assert_eq!(s.modes(), [Mode::Global, Mode::Broadcast]);
        assert!(ModeSet::EMPTY.is_empty() && !s.is_empty());
        assert!(ModeSet(0x10).is_empty());
    }

    #[test]
    fn activations() {
        for a in Activation::ALL {
            assert_eq!(Activation::parse(a.name()), Some(a));
        }
        assert_eq!(Activation::parse(" PTT"), Some(Activation::Ptt));
        assert_eq!(Activation::parse("hold"), None);
        let s = ActSet::default().with(Activation::Ptt).with(Activation::Toggle);
        assert_eq!(s.0, 0b101);
        assert!(!s.has(Activation::Vad));
        assert_eq!(s.list(), [Activation::Ptt, Activation::Toggle]);
    }

    #[test]
    fn states_and_icons() {
        let all = [
            ("none", State::None, Some(Icon::NoVoice)),
            ("ready", State::Ready, None),
            ("talking", State::Talking, Some(Icon::Speaking)),
            ("self-muted", State::SelfMuted, Some(Icon::SelfMute)),
            ("deafened", State::Deafened, Some(Icon::Deafen)),
            ("admin-muted", State::AdminMuted, Some(Icon::AdminMute)),
        ];
        for (name, state, icon) in all {
            assert_eq!(State::parse(name), Some(state));
            assert_eq!(state.icon(), icon);
        }
        assert_eq!(State::parse("self_muted"), None);
        assert!(Role::Owner > Role::Admin && Role::Admin > Role::Moderator && Role::Moderator > Role::Player);
    }
}
