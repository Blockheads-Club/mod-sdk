//! Voice chat (`blockheads:api@0.3.4` `voice`, permission `voice`): the types of the WIT interface and the
//! [`VoiceHost`] seam a host program implements (the server with `bhmod_voice::server`, the clients with
//! `bhmod_voice::client`). Mods only set policy; the host does audio, Opus, crypto and the relay.
//!
//! The mod host checks the permission, the side, ranges and that players are online before it calls a
//! [`VoiceHost`]; every method defaults to [`ApiError::Unsupported`], so a host without voice implements nothing.
//! Events from the host (talk start/stop, state changes, the microphone) come back through
//! [`crate::ModHost::voice_event`] and reach mods as the custom events `voice:*`.

use crate::types::ApiError;
use crate::value::Value;

/// Most routes per speaker in one `set-routes`.
pub const MAX_ROUTES: usize = 256;
/// Most server speaker gain / client volume, percent.
pub const MAX_PERCENT: u16 = 200;
/// The bitrate cap range (bits per second).
pub const BITRATE: (u32, u32) = (6_000, 64_000);
/// The max-speakers range.
pub const SPEAKERS: (u32, u32) = (1, 64);
/// The frame rate limit range (frames per second).
pub const FRAME_RATE: (u32, u32) = (50, 200);
/// The VAD threshold range (dBFS).
pub const VAD_DB: (f32, f32) = (-80.0, 0.0);
/// The mod id that owns the `voice:*` event names.
pub const EVENT_PREFIX: &str = "voice";

macro_rules! named {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($(#[$vm])* $v),+
        }

        impl $name {
            /// Every case, in WIT order.
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            /// The WIT name.
            pub fn name(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }

            /// Parses a WIT name.
            pub fn parse(s: &str) -> Option<$name> {
                $name::ALL.iter().copied().find(|c| c.name() == s)
            }

            /// The bit of this case in a mask (WIT order).
            pub fn bit(self) -> u8 {
                1 << (self as u8)
            }

            /// A mask of several cases.
            pub fn mask(cases: &[$name]) -> u8 {
                cases.iter().fold(0, |m, c| m | c.bit())
            }

            /// The cases of a mask, in WIT order.
            pub fn from_mask(mask: u8) -> Vec<$name> {
                $name::ALL.iter().copied().filter(|c| mask & c.bit() != 0).collect()
            }
        }
    };
}

named! {
    /// `voice.mode`; the order is the wire `mode` byte.
    Mode {
        /// Players nearby.
        Proximity = "proximity",
        /// Everyone.
        Global = "global",
        /// The speaker's channel.
        Channel = "channel",
        /// Admin broadcast.
        Broadcast = "broadcast",
    }
}

named! {
    /// `voice.activation`.
    Activation {
        /// Push to talk.
        Ptt = "ptt",
        /// Voice activation.
        Vad = "vad",
        /// Toggle.
        Toggle = "toggle",
    }
}

named! {
    /// `voice.voice-state`.
    VoiceState {
        /// No voice.
        None = "none",
        /// Can talk and hear.
        Ready = "ready",
        /// Talking now.
        Talking = "talking",
        /// Muted their microphone.
        SelfMuted = "self-muted",
        /// Hears nobody.
        Deafened = "deafened",
        /// Admin mute.
        AdminMuted = "admin-muted",
    }
}

impl VoiceState {
    /// The `VOICE_STATE` wire text (`self_muted`, ...).
    pub fn wire(self) -> &'static str {
        match self {
            VoiceState::SelfMuted => "self_muted",
            VoiceState::AdminMuted => "admin_muted",
            other => other.name(),
        }
    }

    /// Parses a `VOICE_STATE` wire text.
    pub fn from_wire(s: &str) -> Option<VoiceState> {
        VoiceState::ALL.iter().copied().find(|c| c.wire() == s)
    }
}

named! {
    /// `voice.transport`. Kept for 0.3.x compatibility and ignored: voice frames always travel over the voice UDP
    /// port (maintainer decision 2026-10-28); only [`TransportConfig::udp_port`] matters.
    Transport {
        /// Same as `udp`.
        Auto = "auto",
        /// Same as `udp` (in-band voice frames were retired).
        Inband = "inband",
        /// The voice UDP port.
        Udp = "udp",
    }
}

named! {
    /// The state of the client microphone (`voice:mic`).
    MicState {
        /// Closed.
        Off = "off",
        /// Open.
        On = "on",
        /// The OS refused the permission.
        Denied = "denied",
        /// No voice: the server has no voice UDP port, or the device has no microphone.
        Unavailable = "unavailable",
    }
}

/// `voice.route`, with the listener as a player id.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// Who hears (vanilla `playerID`).
    pub listener: String,
    /// [`Mode`] mask; 0 = every mode.
    pub modes: u8,
    /// 0..=1.
    pub gain: f32,
    /// -1..=1.
    pub pan: f32,
    /// 0..=1.
    pub muffle: f32,
}

impl Route {
    /// The mask the relay uses: 0 (every mode) becomes all four bits.
    pub fn mode_mask(&self) -> u8 {
        if self.modes == 0 {
            0x0f
        } else {
            self.modes
        }
    }

    /// Clamps gain, pan and muffle into range (non-finite values become 0).
    pub fn clamped(mut self) -> Route {
        let c = |x: f32, lo: f32, hi: f32| if x.is_finite() { x.clamp(lo, hi) } else { 0.0 };
        self.gain = c(self.gain, 0.0, 1.0);
        self.pan = c(self.pan, -1.0, 1.0);
        self.muffle = c(self.muffle, 0.0, 1.0);
        self
    }
}

/// `voice.caps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Bits per second.
    pub max_bitrate: u32,
    /// Speakers at once.
    pub max_speakers: u32,
    /// Frames per second per player.
    pub max_frames_per_sec: u32,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps { max_bitrate: 24_000, max_speakers: 8, max_frames_per_sec: 60 }
    }
}

impl Caps {
    /// Checks the ranges.
    pub fn check(&self) -> Result<(), ApiError> {
        range("max-bitrate", self.max_bitrate, BITRATE)?;
        range("max-speakers", self.max_speakers, SPEAKERS)?;
        range("max-frames-per-sec", self.max_frames_per_sec, FRAME_RATE)
    }
}

fn range(what: &str, v: u32, (lo, hi): (u32, u32)) -> Result<(), ApiError> {
    if v < lo || v > hi {
        return Err(ApiError::InvalidArgument(format!("{what} must be {lo} to {hi}")));
    }
    Ok(())
}

/// Checks a percent (0..=200).
pub fn check_percent(what: &str, p: u16) -> Result<(), ApiError> {
    if p > MAX_PERCENT {
        return Err(ApiError::InvalidArgument(format!("{what} must be 0 to {MAX_PERCENT}")));
    }
    Ok(())
}

/// Checks a VAD threshold.
pub fn check_vad_db(db: f32) -> Result<(), ApiError> {
    if !(VAD_DB.0..=VAD_DB.1).contains(&db) {
        return Err(ApiError::InvalidArgument("vad threshold must be -80.0 to 0.0 dBFS".into()));
    }
    Ok(())
}

/// `voice.allowance` (client): what the server allows this player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowance {
    /// [`Mode`] mask.
    pub modes: u8,
    /// [`Activation`] mask.
    pub activations: u8,
    /// The encoder bitrate.
    pub max_bitrate: u32,
}

impl Default for Allowance {
    fn default() -> Allowance {
        Allowance { modes: 0, activations: 0x07, max_bitrate: 24_000 }
    }
}

/// `voice.transport-config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportConfig {
    /// Ignored (see [`Transport`]).
    pub mode: Transport,
    /// The voice UDP port; `None`: the host's default (the server's configured voice port, game port + 1).
    pub udp_port: Option<u16>,
}

impl Default for TransportConfig {
    fn default() -> TransportConfig {
        TransportConfig { mode: Transport::Udp, udp_port: None }
    }
}

/// `voice.icon-visibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icons {
    /// Talking.
    pub speaking: bool,
    /// No voice chat.
    pub no_voice: bool,
    /// Self mute.
    pub self_mute: bool,
    /// Deafened.
    pub deafen: bool,
    /// Admin mute.
    pub admin_mute: bool,
}

impl Default for Icons {
    fn default() -> Icons {
        Icons { speaking: true, no_voice: true, self_mute: true, deafen: true, admin_mute: true }
    }
}

impl Icons {
    /// The `VOICE_STATE.icons` wire keys and values.
    pub fn wire(&self) -> [(&'static str, bool); 5] {
        [
            ("speaking", self.speaking),
            ("no_voice", self.no_voice),
            ("self_mute", self.self_mute),
            ("deafen", self.deafen),
            ("admin_mute", self.admin_mute),
        ]
    }

    /// Applies `VOICE_STATE.icons` wire entries (unknown keys are ignored).
    pub fn apply_wire<'a>(&mut self, entries: impl IntoIterator<Item = (&'a str, bool)>) {
        for (k, v) in entries {
            match k {
                "speaking" => self.speaking = v,
                "no_voice" => self.no_voice = v,
                "self_mute" => self.self_mute = v,
                "deafen" => self.deafen = v,
                "admin_mute" => self.admin_mute = v,
                _ => {}
            }
        }
    }
}

/// Something the voice host reports; [`crate::ModHost::voice_event`] turns it into a `voice:*` custom event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceEvent {
    /// `voice:talk-start`.
    TalkStart {
        /// The player id.
        player: String,
        /// The mode.
        mode: Mode,
    },
    /// `voice:talk-stop`.
    TalkStop {
        /// The player id.
        player: String,
        /// The mode of the spurt.
        mode: Mode,
    },
    /// `voice:state`.
    State {
        /// The player id.
        player: String,
        /// The new state.
        state: VoiceState,
    },
    /// `voice:mic` (clients).
    Mic(MicState),
    /// `voice:allowed` (clients).
    Allowed(Allowance),
    /// `voice:mode` (clients): the default mode changed.
    Mode(Mode),
}

impl VoiceEvent {
    /// The custom event name and its payload (canonical CBOR).
    pub fn custom(&self) -> (String, Vec<u8>) {
        let (name, fields) = match self {
            VoiceEvent::TalkStart { player, mode } => {
                ("talk-start", vec![("player", Value::text(player)), ("mode", Value::text(mode.name()))])
            }
            VoiceEvent::TalkStop { player, mode } => {
                ("talk-stop", vec![("player", Value::text(player)), ("mode", Value::text(mode.name()))])
            }
            VoiceEvent::State { player, state } => {
                ("state", vec![("player", Value::text(player)), ("state", Value::text(state.name()))])
            }
            VoiceEvent::Mic(s) => ("mic", vec![("state", Value::text(s.name()))]),
            VoiceEvent::Mode(m) => ("mode", vec![("mode", Value::text(m.name()))]),
            VoiceEvent::Allowed(a) => {
                let names = |v: Vec<&str>| Value::List(v.into_iter().map(Value::text).collect());
                let modes = names(Mode::from_mask(a.modes).into_iter().map(Mode::name).collect());
                let acts = names(Activation::from_mask(a.activations).into_iter().map(Activation::name).collect());
                let rate = Value::Int(i64::from(a.max_bitrate));
                ("allowed", vec![("modes", modes), ("activations", acts), ("max-bitrate", rate)])
            }
        };
        let payload = Value::Map(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect()).encode();
        (format!("{EVENT_PREFIX}:{name}"), payload)
    }
}

/// One `voice` function call of a mod (players by id), as the runtimes hand it to [`crate::HostApi::voice`].
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceCall {
    /// `state`.
    State(String),
    /// `icons`.
    Icons,
    /// `set-routes`.
    SetRoutes(String, Vec<Route>),
    /// `mute`.
    Mute(String, Option<u64>),
    /// `unmute`.
    Unmute(String),
    /// `set-speaker-gain`.
    SetSpeakerGain(String, u16),
    /// `set-caps`.
    SetCaps(Caps),
    /// `set-transport`.
    SetTransport(TransportConfig),
    /// `set-allowed` ([`Mode`] and [`Activation`] masks).
    SetAllowed(String, u8, u8),
    /// `set-icons`.
    SetIcons(Icons),
    /// `set-broadcast-overrides-deafen`.
    SetBroadcastOverridesDeafen(bool),
    /// `set-default-mode`.
    SetDefaultMode(Mode),
    /// `set-activation`.
    SetActivation(Activation),
    /// `talk`.
    Talk(Option<Mode>, bool),
    /// `set-mic`.
    SetMic(bool),
    /// `set-self-mute`.
    SetSelfMute(bool),
    /// `set-deafen`.
    SetDeafen(bool),
    /// `set-vad-threshold`.
    SetVadThreshold(f32),
    /// `set-master`.
    SetMaster(u16),
    /// `master`.
    Master,
    /// `set-player-volume`.
    SetPlayerVolume(String, u16),
    /// `player-volume`.
    PlayerVolume(String),
    /// `mute-player`.
    MutePlayer(String, bool),
    /// `player-muted`.
    PlayerMuted(String),
    /// `allowed`.
    Allowed,
    /// `self-muted`.
    SelfMuted,
    /// `deafened`.
    Deafened,
    /// `mic`.
    Mic,
}

impl VoiceCall {
    /// The side the function works on; `None` for both.
    pub fn side(&self) -> Option<crate::types::Side> {
        use crate::types::Side;
        match self {
            VoiceCall::State(_) | VoiceCall::Icons => None,
            VoiceCall::SetRoutes(..)
            | VoiceCall::Mute(..)
            | VoiceCall::Unmute(_)
            | VoiceCall::SetSpeakerGain(..)
            | VoiceCall::SetCaps(_)
            | VoiceCall::SetTransport(_)
            | VoiceCall::SetAllowed(..)
            | VoiceCall::SetIcons(_)
            | VoiceCall::SetBroadcastOverridesDeafen(_) => Some(Side::Server),
            _ => Some(Side::Client),
        }
    }

    /// The players the call names (they must be online).
    pub fn players(&self) -> Vec<&str> {
        match self {
            VoiceCall::SetRoutes(p, routes) => {
                std::iter::once(p.as_str()).chain(routes.iter().map(|r| r.listener.as_str())).collect()
            }
            VoiceCall::State(p)
            | VoiceCall::Mute(p, _)
            | VoiceCall::Unmute(p)
            | VoiceCall::SetSpeakerGain(p, _)
            | VoiceCall::SetAllowed(p, ..)
            | VoiceCall::SetPlayerVolume(p, _)
            | VoiceCall::PlayerVolume(p)
            | VoiceCall::MutePlayer(p, _)
            | VoiceCall::PlayerMuted(p) => vec![p.as_str()],
            _ => Vec::new(),
        }
    }

    /// Checks ranges and counts.
    pub fn check(&self) -> Result<(), ApiError> {
        match self {
            VoiceCall::SetRoutes(_, r) if r.len() > MAX_ROUTES => {
                Err(ApiError::LimitExceeded(format!("at most {MAX_ROUTES} routes per speaker")))
            }
            VoiceCall::SetSpeakerGain(_, p) => check_percent("speaker gain", *p),
            VoiceCall::SetCaps(c) => c.check(),
            VoiceCall::SetVadThreshold(db) => check_vad_db(*db),
            VoiceCall::SetMaster(p) => check_percent("master volume", *p),
            VoiceCall::SetPlayerVolume(_, p) => check_percent("player volume", *p),
            _ => Ok(()),
        }
    }

    /// Runs the call on a host.
    pub fn run(self, h: &dyn VoiceHost) -> Result<VoiceReply, ApiError> {
        use VoiceReply::{Bool, Percent, Unit};
        Ok(match self {
            VoiceCall::State(p) => VoiceReply::State(h.state(&p)?),
            VoiceCall::Icons => VoiceReply::Icons(h.icons()?),
            VoiceCall::SetRoutes(p, r) => {
                let r: Vec<Route> = r.into_iter().map(Route::clamped).collect();
                h.set_routes(&p, &r).map(|_| Unit)?
            }
            VoiceCall::Mute(p, until) => h.mute(&p, until).map(|_| Unit)?,
            VoiceCall::Unmute(p) => Bool(h.unmute(&p)?),
            VoiceCall::SetSpeakerGain(p, g) => h.set_speaker_gain(&p, g).map(|_| Unit)?,
            VoiceCall::SetCaps(c) => h.set_caps(c).map(|_| Unit)?,
            VoiceCall::SetTransport(t) => h.set_transport(t).map(|_| Unit)?,
            VoiceCall::SetAllowed(p, m, a) => h.set_allowed(&p, m & 0x0f, a & 0x07).map(|_| Unit)?,
            VoiceCall::SetIcons(i) => h.set_icons(i).map(|_| Unit)?,
            VoiceCall::SetBroadcastOverridesDeafen(on) => h.set_broadcast_overrides_deafen(on).map(|_| Unit)?,
            VoiceCall::SetDefaultMode(m) => h.set_default_mode(m).map(|_| Unit)?,
            VoiceCall::SetActivation(a) => h.set_activation(a).map(|_| Unit)?,
            VoiceCall::Talk(m, pressed) => h.talk(m, pressed).map(|_| Unit)?,
            VoiceCall::SetMic(on) => h.set_mic(on).map(|_| Unit)?,
            VoiceCall::SetSelfMute(on) => h.set_self_mute(on).map(|_| Unit)?,
            VoiceCall::SetDeafen(on) => h.set_deafen(on).map(|_| Unit)?,
            VoiceCall::SetVadThreshold(db) => h.set_vad_threshold(db).map(|_| Unit)?,
            VoiceCall::SetMaster(p) => h.set_master(p).map(|_| Unit)?,
            VoiceCall::Master => Percent(h.master()?),
            VoiceCall::SetPlayerVolume(p, v) => h.set_player_volume(&p, v).map(|_| Unit)?,
            VoiceCall::PlayerVolume(p) => Percent(h.player_volume(&p)?),
            VoiceCall::MutePlayer(p, m) => h.mute_player(&p, m).map(|_| Unit)?,
            VoiceCall::PlayerMuted(p) => Bool(h.player_muted(&p)?),
            VoiceCall::Allowed => VoiceReply::Allowed(h.allowed()?),
            VoiceCall::SelfMuted => Bool(h.self_muted()?),
            VoiceCall::Deafened => Bool(h.deafened()?),
            VoiceCall::Mic => VoiceReply::Mic(h.mic()?),
        })
    }
}

/// What a [`VoiceCall`] returns. The runtimes read it with the accessor matching the function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceReply {
    /// No value.
    Unit,
    /// `unmute`, `player-muted`.
    Bool(bool),
    /// `master`, `player-volume`.
    Percent(u16),
    /// `state`.
    State(VoiceState),
    /// `icons`.
    Icons(Icons),
    /// `allowed`.
    Allowed(Allowance),
    /// `mic`.
    Mic(MicState),
}

impl VoiceReply {
    /// The bool of `unmute` / `player-muted` (false for any other reply).
    pub fn as_bool(self) -> bool {
        matches!(self, VoiceReply::Bool(true))
    }

    /// The percent of `master` / `player-volume` (100 for any other reply).
    pub fn as_percent(self) -> u16 {
        if let VoiceReply::Percent(p) = self {
            p
        } else {
            100
        }
    }

    /// The state of `state` (`none` for any other reply).
    pub fn as_state(self) -> VoiceState {
        if let VoiceReply::State(s) = self {
            s
        } else {
            VoiceState::None
        }
    }

    /// The allowance of `allowed` (the defaults for any other reply).
    pub fn as_allowance(self) -> Allowance {
        if let VoiceReply::Allowed(a) = self {
            a
        } else {
            Allowance::default()
        }
    }

    /// The state of `mic` (`off` for any other reply).
    pub fn as_mic(self) -> MicState {
        if let VoiceReply::Mic(m) = self {
            m
        } else {
            MicState::Off
        }
    }

    /// The icons of `icons` (the defaults for any other reply).
    pub fn as_icons(self) -> Icons {
        if let VoiceReply::Icons(i) = self {
            i
        } else {
            Icons::default()
        }
    }
}

/// The voice side of a host program. Every method is called on the game's main thread while a mod call runs, so it
/// must not call back into the [`crate::ModHost`]; the arguments are already checked (permission, side, ranges,
/// players online). All default to [`ApiError::Unsupported`].
#[allow(unused_variables)]
pub trait VoiceHost {
    /// `voice.state` (both sides).
    fn state(&self, player: &str) -> Result<VoiceState, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.icons` (both sides).
    fn icons(&self) -> Result<Icons, ApiError> {
        Err(ApiError::Unsupported)
    }

    /// `voice.set-routes` (server). Routes are clamped; listeners are online players.
    fn set_routes(&self, speaker: &str, routes: &[Route]) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.mute` (server): until Unix time `until_ms`, or until unmuted.
    fn mute(&self, player: &str, until_ms: Option<u64>) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.unmute` (server): false when not muted.
    fn unmute(&self, player: &str) -> Result<bool, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-speaker-gain` (server), 0..=200.
    fn set_speaker_gain(&self, player: &str, percent: u16) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-caps` (server), in range.
    fn set_caps(&self, caps: Caps) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-transport` (server).
    fn set_transport(&self, config: TransportConfig) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-allowed` (server): [`Mode`] and [`Activation`] masks.
    fn set_allowed(&self, player: &str, modes: u8, activations: u8) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-icons` (server).
    fn set_icons(&self, icons: Icons) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-broadcast-overrides-deafen` (server).
    fn set_broadcast_overrides_deafen(&self, on: bool) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }

    /// `voice.set-default-mode` (client).
    fn set_default_mode(&self, mode: Mode) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-activation` (client); `invalid-argument` when the server does not allow it.
    fn set_activation(&self, activation: Activation) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.talk` (client): `None` = the default mode.
    fn talk(&self, mode: Option<Mode>, pressed: bool) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-mic` (client).
    fn set_mic(&self, on: bool) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-self-mute` (client).
    fn set_self_mute(&self, on: bool) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-deafen` (client).
    fn set_deafen(&self, on: bool) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-vad-threshold` (client), in range.
    fn set_vad_threshold(&self, db: f32) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-master` (client), 0..=200.
    fn set_master(&self, percent: u16) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.master` (client).
    fn master(&self) -> Result<u16, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.set-player-volume` (client), 0..=200.
    fn set_player_volume(&self, player: &str, percent: u16) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.player-volume` (client).
    fn player_volume(&self, player: &str) -> Result<u16, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.mute-player` (client).
    fn mute_player(&self, player: &str, muted: bool) -> Result<(), ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.player-muted` (client).
    fn player_muted(&self, player: &str) -> Result<bool, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.allowed` (client).
    fn allowed(&self) -> Result<Allowance, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.self-muted` (client).
    fn self_muted(&self) -> Result<bool, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.deafened` (client).
    fn deafened(&self) -> Result<bool, ApiError> {
        Err(ApiError::Unsupported)
    }
    /// `voice.mic` (client).
    fn mic(&self) -> Result<MicState, ApiError> {
        Err(ApiError::Unsupported)
    }
}

/// Most tiles `terrain.solid-count` walks.
pub const MAX_LINE: u32 = 256;

/// `terrain.solid-count`: solid tiles on the Bresenham line between two tiles, both ends excluded, `x` wrapping the
/// short way round a world `width` tiles wide (when known), cut after [`MAX_LINE`] tiles. `solid` answers one tile
/// (`None` = not loaded = not solid).
pub fn solid_count(
    (x1, y1): (i32, i32),
    (x2, y2): (i32, i32),
    width: Option<i32>,
    solid: &dyn Fn(i32, i32) -> Option<bool>,
) -> u32 {
    let mut dx = i64::from(x2) - i64::from(x1);
    if let Some(w) = width.filter(|w| *w > 0).map(i64::from) {
        dx = dx.rem_euclid(w);
        if dx > w / 2 {
            dx -= w;
        }
    }
    let dy = i64::from(y2) - i64::from(y1);
    let (sx, sy) = (dx.signum(), dy.signum());
    let (ax, ay) = (dx.abs(), -dy.abs());
    let (mut x, mut y) = (i64::from(x1), i64::from(y1));
    let mut err = ax + ay;
    let mut count = 0;
    let wrap = |x: i64| match width.filter(|w| *w > 0) {
        Some(w) => x.rem_euclid(i64::from(w)) as i32,
        None => x as i32,
    };
    for _ in 0..MAX_LINE {
        let e2 = 2 * err;
        if e2 >= ay {
            err += ay;
            x += sx;
        }
        if e2 <= ax {
            err += ax;
            y += sy;
        }
        if x == i64::from(x1) + dx && y == i64::from(y2) {
            break;
        }
        if solid(wrap(x), y as i32) == Some(true) {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_masks_and_wire() {
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.name()), Some(*m));
        }
        assert_eq!(Mode::mask(&[Mode::Proximity, Mode::Channel]), 0b0101);
        assert_eq!(Mode::from_mask(0b1010), vec![Mode::Global, Mode::Broadcast]);
        assert_eq!(Activation::mask(Activation::ALL), 0b111);
        assert_eq!(Activation::parse("vad"), Some(Activation::Vad));
        assert_eq!(Transport::parse("inband"), Some(Transport::Inband));
        assert_eq!(MicState::parse("denied"), Some(MicState::Denied));
        assert_eq!(MicState::parse("unavailable"), Some(MicState::Unavailable));
        assert!(Mode::parse("shout").is_none());
        for s in VoiceState::ALL {
            assert_eq!(VoiceState::from_wire(s.wire()), Some(*s));
        }
        assert_eq!(VoiceState::SelfMuted.wire(), "self_muted");
        assert_eq!(VoiceState::AdminMuted.name(), "admin-muted");
        assert_eq!(VoiceState::from_wire("gone"), None);
    }

    #[test]
    fn routes_and_ranges() {
        let r = Route { listener: "a".into(), modes: 0, gain: 2.0, pan: f32::NAN, muffle: -1.0 }.clamped();
        assert_eq!((r.gain, r.pan, r.muffle, r.mode_mask()), (1.0, 0.0, 0.0, 0x0f));
        assert_eq!(Route { modes: 2, ..r }.mode_mask(), 2);
        assert!(Caps::default().check().is_ok());
        for bad in [
            Caps { max_bitrate: 5_999, ..Caps::default() },
            Caps { max_speakers: 65, ..Caps::default() },
            Caps { max_frames_per_sec: 49, ..Caps::default() },
        ] {
            assert!(matches!(bad.check(), Err(ApiError::InvalidArgument(_))));
        }
        assert!(check_percent("x", 200).is_ok());
        assert!(check_percent("x", 201).is_err());
        assert!(check_vad_db(-45.0).is_ok());
        assert!(check_vad_db(1.0).is_err());
        assert!(check_vad_db(f32::NAN).is_err());
        assert_eq!(TransportConfig::default().mode, Transport::Udp);
    }

    #[test]
    fn icons_wire() {
        let mut i = Icons::default();
        i.apply_wire([("deafen", false), ("no_voice", false), ("bogus", false)]);
        i.apply_wire([("speaking", false), ("self_mute", false), ("admin_mute", false)]);
        assert_eq!(i.wire().iter().filter(|(_, v)| *v).count(), 0);
        assert_eq!(Icons::default().wire()[1], ("no_voice", true));
    }

    #[test]
    fn event_payloads() {
        let cases = [
            (VoiceEvent::TalkStart { player: "p".into(), mode: Mode::Global }, "voice:talk-start", "mode", "global"),
            (VoiceEvent::TalkStop { player: "p".into(), mode: Mode::Channel }, "voice:talk-stop", "mode", "channel"),
            (
                VoiceEvent::State { player: "p".into(), state: VoiceState::SelfMuted },
                "voice:state",
                "state",
                "self-muted",
            ),
            (VoiceEvent::Mic(MicState::On), "voice:mic", "state", "on"),
            (VoiceEvent::Mode(Mode::Broadcast), "voice:mode", "mode", "broadcast"),
        ];
        for (ev, name, key, val) in cases {
            let (n, p) = ev.custom();
            assert_eq!(n, name);
            assert_eq!(Value::decode(&p).unwrap().get(key), Some(&Value::text(val)));
        }
    }

    struct Nothing;
    impl VoiceHost for Nothing {}

    #[test]
    fn reply_accessors_and_runs() {
        let u = VoiceReply::Unit;
        assert_eq!(
            (u.as_bool(), u.as_percent(), u.as_state(), u.as_icons()),
            (false, 100, VoiceState::None, Icons::default())
        );
        assert_eq!(VoiceReply::Percent(5).as_percent(), 5);
        assert!(VoiceReply::Bool(true).as_bool());
        assert_eq!(VoiceReply::State(VoiceState::Talking).as_state(), VoiceState::Talking);
        assert!(!VoiceReply::Icons(Icons { deafen: false, ..Icons::default() }).as_icons().deafen);
        assert_eq!(VoiceCall::Master.run(&Nothing), Err(ApiError::Unsupported));
        assert_eq!((u.as_allowance(), u.as_mic()), (Allowance::default(), MicState::Off));
        assert_eq!(VoiceReply::Mic(MicState::On).as_mic(), MicState::On);
        let a = Allowance { modes: 5, activations: 1, max_bitrate: 16_000 };
        assert_eq!(VoiceReply::Allowed(a).as_allowance(), a);
        let (n, p) = VoiceEvent::Allowed(a).custom();
        let v = Value::decode(&p).unwrap();
        assert_eq!(n, "voice:allowed");
        assert_eq!(v.get("modes"), Some(&Value::List(vec![Value::text("proximity"), Value::text("channel")])));
        assert_eq!(v.get("max-bitrate"), Some(&Value::Int(16_000)));
    }

    #[test]
    fn defaults_are_unsupported() {
        let n = Nothing;
        let u = Err::<(), _>(ApiError::Unsupported);
        assert_eq!(n.state("p"), Err(ApiError::Unsupported));
        assert_eq!(n.icons(), Err(ApiError::Unsupported));
        assert_eq!(n.set_routes("p", &[]), u);
        assert_eq!(n.mute("p", None), u);
        assert_eq!(n.unmute("p"), Err(ApiError::Unsupported));
        assert_eq!(n.set_speaker_gain("p", 1), u);
        assert_eq!(n.set_caps(Caps::default()), u);
        assert_eq!(n.set_transport(TransportConfig::default()), u);
        assert_eq!(n.set_allowed("p", 1, 1), u);
        assert_eq!(n.set_icons(Icons::default()), u);
        assert_eq!(n.set_broadcast_overrides_deafen(true), u);
        assert_eq!(n.set_default_mode(Mode::Global), u);
        assert_eq!(n.set_activation(Activation::Ptt), u);
        assert_eq!(n.talk(None, true), u);
        assert_eq!(n.set_mic(true), u);
        assert_eq!(n.set_self_mute(true), u);
        assert_eq!(n.set_deafen(true), u);
        assert_eq!(n.set_vad_threshold(-1.0), u);
        assert_eq!(n.set_master(1), u);
        assert_eq!(n.master(), Err(ApiError::Unsupported));
        assert_eq!(n.set_player_volume("p", 1), u);
        assert_eq!(n.player_volume("p"), Err(ApiError::Unsupported));
        assert_eq!(n.mute_player("p", true), u);
        assert_eq!(n.player_muted("p"), Err(ApiError::Unsupported));
        assert_eq!(n.allowed(), Err(ApiError::Unsupported));
        assert_eq!(n.self_muted(), Err(ApiError::Unsupported));
        assert_eq!(n.deafened(), Err(ApiError::Unsupported));
        assert_eq!(n.mic(), Err(ApiError::Unsupported));
    }

    #[test]
    fn lines() {
        let wall = |x: i32, _y: i32| Some(x == 5);
        assert_eq!(solid_count((0, 0), (10, 0), None, &wall), 1);
        assert_eq!(solid_count((0, 0), (5, 0), None, &wall), 0, "end tile excluded");
        assert_eq!(solid_count((3, 3), (3, 3), None, &wall), 0);
        // vertical and diagonal lines over a solid world: every tile between counts
        assert_eq!(solid_count((0, 0), (0, 10), None, &|_, _| Some(true)), 9);
        assert_eq!(solid_count((0, 0), (4, -4), None, &|_, _| Some(true)), 3);
        assert_eq!(solid_count((0, 0), (10, 3), None, &|_, _| None), 0, "unloaded tiles are not solid");
        // the short way round a 100-wide world: 98 -> 2 crosses x = 99, 0, 1
        let seen = std::cell::RefCell::new(Vec::new());
        let n = solid_count((98, 0), (2, 0), Some(100), &|x, _| {
            seen.borrow_mut().push(x);
            Some(true)
        });
        assert_eq!((n, seen.into_inner()), (3, vec![99, 0, 1]));
        assert_eq!(solid_count((2, 0), (98, 0), Some(100), &|_, _| Some(true)), 3);
        assert_eq!(solid_count((0, 0), (10, 0), Some(0), &|_, _| Some(true)), 9, "width 0 = unknown");
        // capped
        assert_eq!(solid_count((0, 0), (10_000, 0), None, &|_, _| Some(true)), MAX_LINE);
    }
}
