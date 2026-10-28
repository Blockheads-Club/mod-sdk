//! The mod's settings (`[settings]` of `mod.toml`) read from `settings::all()`. Missing or malformed values fall
//! back to the defaults below, which match `mod.toml` (a test checks that).

use blockheads_mod::cbor::Value;

use crate::model::{ActSet, Activation, Mode, ModeSet};

/// Proximity falloff curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// Gain falls linearly to 0 at the radius.
    Linear,
    /// Gain falls fast near the speaker and slowly further out (logarithmic).
    Log,
}

/// Voice transport (`set-transport`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// In-band, UDP when needed.
    Auto,
    /// In-band only.
    Inband,
    /// The UDP port only.
    Udp,
}

/// An admin-defined channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedChannel {
    /// Its name.
    pub name: String,
    /// Its password (`None` = open).
    pub password: Option<String>,
}

/// Which overhead icons clients show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icons {
    /// Talking.
    pub speaking: bool,
    /// No voice.
    pub no_voice: bool,
    /// Self mute.
    pub self_mute: bool,
    /// Deafen.
    pub deafen: bool,
    /// Admin mute.
    pub admin_mute: bool,
}

/// The server settings.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerConfig {
    /// The modes the admin switched on.
    pub modes: ModeSet,
    /// The activations players may use.
    pub activations: ActSet,
    /// Who may talk in each mode (in [`Mode::ALL`] order): role names and permission groups.
    pub talk: [Vec<String>; 4],
    /// Hearing radius in blocks.
    pub radius: f32,
    /// Falloff curve.
    pub curve: Curve,
    /// Stereo pan by x.
    pub pan: bool,
    /// Muffle through solid blocks.
    pub muffle: bool,
    /// Muffle per solid block, 0..=1.
    pub muffle_per_block: f32,
    /// Dead players talk in proximity.
    pub dead_can_talk: bool,
    /// Sleeping players talk in proximity.
    pub sleeping_can_talk: bool,
    /// Bitrate cap.
    pub max_bitrate: u32,
    /// Most players talking at once.
    pub max_speakers: u32,
    /// Per-player frame rate limit.
    pub max_frames_per_sec: u32,
    /// Transport mode.
    pub transport: Transport,
    /// UDP port (`None` = no port).
    pub udp_port: Option<u16>,
    /// Admin-defined channels.
    pub channels: Vec<FixedChannel>,
    /// Players may create temporary channels.
    pub player_created: bool,
    /// Temporary channels one player may own at once.
    pub max_per_player: u32,
    /// Broadcast reaches deafened players.
    pub broadcast_overrides_deafen: bool,
    /// Overhead icon visibility.
    pub icons: Icons,
    /// Players without the voice mod may stay.
    pub allow_vanilla: bool,
    /// Per-speaker gain from the config: (player id or name, percent).
    pub speaker_gain: Vec<(String, u16)>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig::from_value(&Value::Map(Vec::new()))
    }
}

fn get_bool(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn get_int(v: &Value, key: &str, default: i64, min: i64, max: i64) -> i64 {
    v.get(key).and_then(Value::as_i64).filter(|n| (min..=max).contains(n)).unwrap_or(default)
}

fn get_float(v: &Value, key: &str, default: f32, min: f32, max: f32) -> f32 {
    let f = match v.get(key) {
        Some(Value::Float(f)) => *f as f32,
        Some(Value::Int(n)) => *n as f32,
        _ => return default,
    };
    if (min..=max).contains(&f) {
        f
    } else {
        default
    }
}

fn get_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn get_roles(v: &Value, key: &str, default: &[&str]) -> Vec<String> {
    match v.get(key) {
        Some(Value::List(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => default.iter().map(|s| s.to_string()).collect(),
    }
}

/// The settings key of a mode switch (`mode_proximity`, ...).
pub fn mode_key(m: Mode) -> String {
    format!("mode_{}", m.name())
}

/// The settings key of a mode's talk roles (`talk_proximity`, ...).
pub fn talk_key(m: Mode) -> String {
    format!("talk_{}", m.name())
}

impl ServerConfig {
    /// Reads `settings::all()`.
    pub fn from_value(v: &Value) -> ServerConfig {
        let mode_defaults = [true, false, true, true];
        let mut modes = ModeSet::EMPTY;
        for (m, d) in Mode::ALL.into_iter().zip(mode_defaults) {
            if get_bool(v, &mode_key(m), d) {
                modes = modes.with(m);
            }
        }
        let mut activations = ActSet::default();
        for a in Activation::ALL {
            if get_bool(v, &format!("activation_{}", a.name()), true) {
                activations = activations.with(a);
            }
        }
        let talk_defaults: [&[&str]; 4] = [&["everyone"], &["admin"], &["everyone"], &["admin", "mod"]];
        let talk = [0, 1, 2, 3].map(|i| get_roles(v, &talk_key(Mode::ALL[i]), talk_defaults[i]));
        let channels = match v.get("channels") {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|c| {
                    let name = get_str(c, "name")?.trim();
                    let password = get_str(c, "password").filter(|p| !p.is_empty()).map(str::to_string);
                    (!name.is_empty()).then(|| FixedChannel { name: name.to_string(), password })
                })
                .collect(),
            _ => Vec::new(),
        };
        let speaker_gain = match v.get("speaker_gain") {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|g| {
                    let who = get_str(g, "player")?.trim();
                    let percent = get_int(g, "percent", 100, 0, 200) as u16;
                    (!who.is_empty()).then(|| (who.to_string(), percent))
                })
                .collect(),
            _ => Vec::new(),
        };
        ServerConfig {
            modes,
            activations,
            talk,
            radius: get_int(v, "proximity_radius", 32, 4, 256) as f32,
            curve: match get_str(v, "proximity_curve") {
                Some("linear") => Curve::Linear,
                _ => Curve::Log,
            },
            pan: get_bool(v, "proximity_pan", true),
            muffle: get_bool(v, "proximity_muffle", true),
            muffle_per_block: get_float(v, "proximity_muffle_per_block", 0.15, 0.0, 1.0),
            dead_can_talk: get_bool(v, "dead_can_talk", false),
            sleeping_can_talk: get_bool(v, "sleeping_can_talk", false),
            max_bitrate: get_int(v, "max_bitrate", 24000, 6000, 64000) as u32,
            max_speakers: get_int(v, "max_speakers", 8, 1, 64) as u32,
            max_frames_per_sec: get_int(v, "max_frames_per_sec", 60, 50, 200) as u32,
            transport: match get_str(v, "transport") {
                Some("inband") => Transport::Inband,
                Some("udp") => Transport::Udp,
                _ => Transport::Auto,
            },
            udp_port: match get_int(v, "udp_port", 0, 0, 65535) {
                0 => None,
                p => Some(p as u16),
            },
            channels,
            player_created: get_bool(v, "channels_player_created", true),
            max_per_player: get_int(v, "channels_max_per_player", 1, 0, 16) as u32,
            broadcast_overrides_deafen: get_bool(v, "broadcast_overrides_deafen", false),
            icons: Icons {
                speaking: get_bool(v, "icon_speaking", true),
                no_voice: get_bool(v, "icon_no_voice", true),
                self_mute: get_bool(v, "icon_self_mute", true),
                deafen: get_bool(v, "icon_deafen", true),
                admin_mute: get_bool(v, "icon_admin_mute", true),
            },
            allow_vanilla: get_bool(v, "allow_vanilla", true),
            speaker_gain,
        }
    }

    /// The talk roles of mode `m`.
    pub fn talk_roles(&self, m: Mode) -> &[String] {
        &self.talk[m as usize]
    }
}

/// When the on-screen push-to-talk button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonMode {
    /// On touch-screen hosts (Android).
    Auto,
    /// Always.
    On,
    /// Never.
    Off,
}

/// The player's answer to "use the microphone?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicChoice {
    /// Not asked yet: ask on the first voice server.
    Ask,
    /// Microphone on.
    On,
    /// Listen only.
    Off,
}

impl MicChoice {
    /// The settings value.
    pub fn name(self) -> &'static str {
        match self {
            MicChoice::Ask => "ask",
            MicChoice::On => "on",
            MicChoice::Off => "off",
        }
    }
}

/// The player's own settings (`scope = "client"`, stored on the device).
#[derive(Clone, Debug, PartialEq)]
pub struct ClientConfig {
    /// Microphone opt-in.
    pub mic: MicChoice,
    /// Mode of voice activation, toggle and the main push-to-talk.
    pub default_mode: Mode,
    /// Activation.
    pub activation: Activation,
    /// Voice activation threshold in dBFS.
    pub vad_threshold: f32,
    /// Master volume, percent.
    pub master_volume: u16,
    /// On-screen button.
    pub button: ButtonMode,
    /// Button diameter.
    pub button_size: u16,
    /// Button opacity, percent.
    pub button_opacity: u8,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig::from_value(&Value::Map(Vec::new()))
    }
}

impl ClientConfig {
    /// Reads `settings::all()` on the client.
    pub fn from_value(v: &Value) -> ClientConfig {
        ClientConfig {
            mic: match get_str(v, "mic") {
                Some("on") => MicChoice::On,
                Some("off") => MicChoice::Off,
                _ => MicChoice::Ask,
            },
            default_mode: get_str(v, "default_mode").and_then(Mode::parse).unwrap_or(Mode::Proximity),
            activation: get_str(v, "activation").and_then(Activation::parse).unwrap_or(Activation::Ptt),
            vad_threshold: get_float(v, "vad_threshold", -45.0, -80.0, 0.0),
            master_volume: get_int(v, "master_volume", 100, 0, 200) as u16,
            button: match get_str(v, "button") {
                Some("on") => ButtonMode::On,
                Some("off") => ButtonMode::Off,
                _ => ButtonMode::Auto,
            },
            button_size: get_int(v, "button_size", 72, 32, 160) as u16,
            button_opacity: get_int(v, "button_opacity", 70, 10, 100) as u8,
        }
    }

    /// Whether the on-screen button shows on a host named `host_name` (`host::host-name`).
    pub fn shows_button(&self, host_name: &str) -> bool {
        match self.button {
            ButtonMode::On => true,
            ButtonMode::Off => false,
            ButtonMode::Auto => host_name.to_ascii_lowercase().contains("android"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    /// `mod.toml` as TOML, its `[settings]` defaults turned into the value `settings::all()` would give.
    fn manifest() -> toml::Table {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/mod.toml")).unwrap();
        text.parse().unwrap()
    }

    fn to_value(t: &toml::Value) -> Value {
        match t {
            toml::Value::String(s) => Value::Text(s.clone()),
            toml::Value::Integer(n) => Value::Int(*n),
            toml::Value::Float(f) => Value::Float(*f),
            toml::Value::Boolean(b) => Value::Bool(*b),
            toml::Value::Array(a) => Value::List(a.iter().map(to_value).collect()),
            toml::Value::Table(t) => Value::Map(t.iter().map(|(k, v)| (k.clone(), to_value(v))).collect()),
            toml::Value::Datetime(d) => Value::Text(d.to_string()),
        }
    }

    fn defaults(scope: &str) -> Value {
        let m = manifest();
        let settings = m["settings"].as_table().unwrap();
        Value::Map(
            settings
                .iter()
                .filter(|(_, s)| s.get("scope").and_then(toml::Value::as_str).unwrap_or("server") == scope)
                .map(|(k, s)| (k.clone(), to_value(&s["default"])))
                .collect(),
        )
    }

    #[test]
    fn manifest_defaults_match_the_code() {
        let server = ServerConfig::from_value(&defaults("server"));
        assert_eq!(server, ServerConfig::default());
        let client = ClientConfig::from_value(&defaults("client"));
        assert_eq!(client, ClientConfig::default());
        // Every setting is labelled in the four UI languages.
        for (key, s) in manifest()["settings"].as_table().unwrap() {
            let label = s["label"].as_table().expect("label table");
            for lang in ["en", "tr", "ru", "kk"] {
                assert!(label.contains_key(lang), "{key}: label.{lang}");
            }
            if let Some(item) = s.get("item") {
                for (field, f) in item.as_table().unwrap() {
                    for lang in ["en", "tr", "ru", "kk"] {
                        assert!(f["label"].get(lang).is_some(), "{key}.{field}: label.{lang}");
                    }
                }
            }
        }
    }

    #[test]
    fn toml_conversion() {
        let t: toml::Table = "a = { b = 1979-05-27 }".parse().unwrap();
        let v = to_value(&toml::Value::Table(t));
        assert_eq!(v.get("a").and_then(|a| a.get("b")), Some(&text("1979-05-27")));
    }

    #[test]
    fn server_defaults() {
        let c = ServerConfig::default();
        assert_eq!(c.modes, ModeSet::of(&[Mode::Proximity, Mode::Channel, Mode::Broadcast]));
        assert_eq!(c.activations.0, 0b111);
        assert_eq!(c.talk_roles(Mode::Broadcast), ["admin", "mod"]);
        assert_eq!(c.talk_roles(Mode::Global), ["admin"]);
        assert_eq!((c.radius, c.curve, c.pan, c.muffle, c.muffle_per_block), (32.0, Curve::Log, true, true, 0.15));
        assert_eq!((c.max_bitrate, c.max_speakers, c.max_frames_per_sec), (24000, 8, 60));
        assert_eq!((c.transport, c.udp_port), (Transport::Auto, None));
        assert!(c.channels.is_empty() && c.player_created && c.max_per_player == 1);
        assert!(!c.broadcast_overrides_deafen && c.allow_vanilla && !c.dead_can_talk && !c.sleeping_can_talk);
        assert_eq!(c.icons, Icons { speaking: true, no_voice: true, self_mute: true, deafen: true, admin_mute: true });
        assert!(c.speaker_gain.is_empty());
    }

    #[test]
    fn server_values() {
        let v = map(vec![
            ("mode_global", Value::Bool(true)),
            ("mode_proximity", Value::Bool(false)),
            ("activation_vad", Value::Bool(false)),
            ("talk_global", Value::List(vec![text("everyone"), Value::Int(3)])),
            ("talk_channel", text("everyone")),
            ("proximity_radius", Value::Int(64)),
            ("proximity_curve", text("linear")),
            ("proximity_muffle_per_block", Value::Int(1)),
            ("dead_can_talk", Value::Bool(true)),
            ("max_bitrate", Value::Int(1)),
            ("max_speakers", Value::Int(16)),
            ("transport", text("udp")),
            ("udp_port", Value::Int(24001)),
            (
                "channels",
                Value::List(vec![
                    map(vec![("name", text(" Red ")), ("password", text(""))]),
                    map(vec![("name", text("Blue")), ("password", text("pw"))]),
                    map(vec![("name", text("  "))]),
                    map(vec![("password", text("x"))]),
                ]),
            ),
            ("channels_player_created", Value::Bool(false)),
            ("icon_no_voice", Value::Bool(false)),
            (
                "speaker_gain",
                Value::List(vec![
                    map(vec![("player", text("LOUDGUY")), ("percent", Value::Int(60))]),
                    map(vec![("player", text("quiet"))]),
                    map(vec![("player", text("")), ("percent", Value::Int(5))]),
                    map(vec![("percent", Value::Int(5))]),
                ]),
            ),
        ]);
        let c = ServerConfig::from_value(&v);
        assert_eq!(c.modes, ModeSet::of(&[Mode::Global, Mode::Channel, Mode::Broadcast]));
        assert_eq!(c.activations.list(), [Activation::Ptt, Activation::Toggle]);
        assert_eq!(c.talk_roles(Mode::Global), ["everyone"]);
        assert_eq!(c.talk_roles(Mode::Channel), ["everyone"], "malformed list -> default");
        assert_eq!((c.radius, c.curve, c.muffle_per_block), (64.0, Curve::Linear, 1.0));
        assert!(c.dead_can_talk);
        assert_eq!((c.max_bitrate, c.max_speakers), (24000, 16), "out of range -> default");
        assert_eq!((c.transport, c.udp_port), (Transport::Udp, Some(24001)));
        assert_eq!(
            c.channels,
            [
                FixedChannel { name: "Red".into(), password: None },
                FixedChannel { name: "Blue".into(), password: Some("pw".into()) }
            ]
        );
        assert!(!c.player_created && !c.icons.no_voice && c.icons.speaking);
        assert_eq!(c.speaker_gain, [("LOUDGUY".to_string(), 60), ("quiet".to_string(), 100)]);
        let c = ServerConfig::from_value(&map(vec![
            ("transport", text("inband")),
            ("proximity_muffle_per_block", Value::Float(0.5)),
            ("channels", Value::Int(1)),
            ("speaker_gain", Value::Int(1)),
        ]));
        assert_eq!((c.transport, c.muffle_per_block), (Transport::Inband, 0.5));
        assert!(c.channels.is_empty() && c.speaker_gain.is_empty());
        let c = ServerConfig::from_value(&map(vec![("proximity_muffle_per_block", Value::Float(2.0))]));
        assert_eq!(c.muffle_per_block, 0.15);
        assert_eq!(mode_key(Mode::Channel), "mode_channel");
        assert_eq!(talk_key(Mode::Broadcast), "talk_broadcast");
    }

    #[test]
    fn client_values() {
        let c = ClientConfig::default();
        assert_eq!(c.mic, MicChoice::Ask);
        assert_eq!((c.default_mode, c.activation, c.vad_threshold), (Mode::Proximity, Activation::Ptt, -45.0));
        assert_eq!((c.master_volume, c.button, c.button_size, c.button_opacity), (100, ButtonMode::Auto, 72, 70));
        assert!(c.shows_button("bhmod-android 0.3") && !c.shows_button("blockheads-pc 0.1"));
        let c = ClientConfig::from_value(&map(vec![
            ("mic", text("on")),
            ("default_mode", text("global")),
            ("activation", text("vad")),
            ("vad_threshold", Value::Float(-30.0)),
            ("master_volume", Value::Int(150)),
            ("button", text("on")),
        ]));
        assert_eq!((c.mic, c.default_mode, c.activation), (MicChoice::On, Mode::Global, Activation::Vad));
        assert_eq!((c.vad_threshold, c.master_volume), (-30.0, 150));
        assert!(c.shows_button("pc"));
        let c = ClientConfig::from_value(&map(vec![("mic", text("off")), ("button", text("off"))]));
        assert_eq!(c.mic, MicChoice::Off);
        assert!(!c.shows_button("android"));
        assert_eq!([MicChoice::Ask.name(), MicChoice::On.name(), MicChoice::Off.name()], ["ask", "on", "off"]);
    }
}
