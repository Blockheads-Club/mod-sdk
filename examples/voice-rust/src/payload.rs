//! Event payloads (canonical CBOR maps) and the small client-side UI decisions: the `info` service answer, the mode
//! picker and the volume dialog text.

use blockheads_mod::cbor::Value;

use crate::model::{ActSet, Activation, Mode, ModeSet, State};

/// `voice:state` `{player, state}`.
pub fn state_event(v: &Value) -> Option<(String, State)> {
    let player = v.get("player")?.as_str()?.to_string();
    Some((player, State::parse(v.get("state")?.as_str()?)?))
}

/// `voice:talk-start` / `voice:talk-stop` `{player, mode}`.
pub fn talk_event(v: &Value) -> Option<(String, Mode)> {
    let player = v.get("player")?.as_str()?.to_string();
    Some((player, Mode::parse(v.get("mode")?.as_str()?)?))
}

/// `<mod>:input` `{action, pressed}`.
pub fn input_event(v: &Value) -> Option<(String, bool)> {
    Some((v.get("action")?.as_str()?.to_string(), v.get("pressed")?.as_bool()?))
}

/// How an on-screen button was touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonAction {
    /// Touch down.
    Press,
    /// Held 500 ms (still down).
    LongPress,
    /// Touch up.
    Release,
}

/// `<mod>:ui-button` `{id, action}`.
pub fn button_event(v: &Value) -> Option<(String, ButtonAction)> {
    let action = match v.get("action")?.as_str()? {
        "press" => ButtonAction::Press,
        "long-press" => ButtonAction::LongPress,
        "release" => ButtonAction::Release,
        _ => return None,
    };
    Some((v.get("id")?.as_str()?.to_string(), action))
}

/// `voice:mic` `{state}`.
pub fn mic_event(v: &Value) -> Option<String> {
    Some(v.get("state")?.as_str()?.to_string())
}

/// `<mod>:settings-changed` `{keys}`.
pub fn changed_keys(v: &Value) -> Vec<String> {
    match v.get("keys") {
        Some(Value::List(k)) => k.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// What the server tells a client through the remote `voice` service method `info`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// The modes this player may talk in.
    pub modes: ModeSet,
    /// The activations the server allows.
    pub activations: ActSet,
    /// The player's channel (display name).
    pub channel: Option<String>,
}

impl Info {
    /// The service answer.
    pub fn to_value(&self) -> Value {
        let names = |v: Vec<&str>| Value::List(v.into_iter().map(Value::from).collect());
        Value::Map(vec![
            ("activations".into(), names(self.activations.list().into_iter().map(Activation::name).collect())),
            ("channel".into(), self.channel.clone().map_or(Value::Null, Value::Text)),
            ("modes".into(), names(self.modes.modes().into_iter().map(Mode::name).collect())),
        ])
    }

    /// Reads the service answer.
    pub fn from_value(v: &Value) -> Info {
        let list = |k: &str| match v.get(k) {
            Some(Value::List(l)) => l.iter().filter_map(Value::as_str).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        Info {
            modes: list("modes").into_iter().filter_map(Mode::parse).fold(ModeSet::EMPTY, ModeSet::with),
            activations: list("activations")
                .into_iter()
                .filter_map(Activation::parse)
                .fold(ActSet::default(), ActSet::with),
            channel: v.get("channel").and_then(Value::as_str).map(str::to_string),
        }
    }
}

/// The modes the long-press picker offers: the allowed ones except the current default (a dialog has at most three
/// buttons, and there are four modes).
pub fn picker_modes(allowed: ModeSet, current: Mode) -> Vec<Mode> {
    allowed.modes().into_iter().filter(|m| *m != current).take(3).collect()
}

/// One line of the volume dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeLine {
    /// Alias.
    pub alias: String,
    /// Percent.
    pub percent: u16,
    /// Muted locally.
    pub muted: bool,
}

/// The volume dialog's text (at most 512 bytes; players that do not fit are counted).
pub fn volume_text(master: u16, players: &[VolumeLine]) -> String {
    let mut out = format!("Master {master}%.");
    if players.is_empty() {
        out.push_str(" Nobody else is here.");
    }
    for (i, p) in players.iter().enumerate() {
        let line = if p.muted { format!("\n{}: muted", p.alias) } else { format!("\n{}: {}%", p.alias, p.percent) };
        let rest = players.len() - i;
        if out.len() + line.len() + 24 > 512 {
            out.push_str(&format!("\n... and {rest} more"));
            break;
        }
        out.push_str(&line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(e: Vec<(&str, Value)>) -> Value {
        Value::Map(e.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn t(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn events() {
        assert_eq!(
            state_event(&map(vec![("player", t("7")), ("state", t("talking"))])),
            Some(("7".into(), State::Talking))
        );
        assert_eq!(state_event(&map(vec![("player", t("7")), ("state", t("dancing"))])), None);
        assert_eq!(state_event(&map(vec![("player", t("7"))])), None);
        assert_eq!(state_event(&Value::Null), None);
        assert_eq!(talk_event(&map(vec![("player", t("7")), ("mode", t("global"))])), Some(("7".into(), Mode::Global)));
        assert_eq!(talk_event(&map(vec![("mode", t("global"))])), None);
        assert_eq!(talk_event(&map(vec![("player", t("7"))])), None);
        assert_eq!(talk_event(&map(vec![("player", t("7")), ("mode", t("x"))])), None);
        assert_eq!(
            input_event(&map(vec![("action", t("talk-global")), ("pressed", Value::Bool(true))])),
            Some(("talk-global".into(), true))
        );
        assert_eq!(input_event(&map(vec![("action", t("a"))])), None);
        assert_eq!(input_event(&map(vec![("pressed", Value::Bool(true))])), None);
        for (name, a) in [
            ("press", ButtonAction::Press),
            ("long-press", ButtonAction::LongPress),
            ("release", ButtonAction::Release),
        ] {
            assert_eq!(button_event(&map(vec![("id", t("ptt")), ("action", t(name))])), Some(("ptt".into(), a)));
        }
        assert_eq!(button_event(&map(vec![("id", t("ptt")), ("action", t("drag"))])), None);
        assert_eq!(button_event(&map(vec![("action", t("press"))])), None);
        assert_eq!(button_event(&map(vec![("id", t("ptt"))])), None);
        assert_eq!(mic_event(&map(vec![("state", t("denied"))])).as_deref(), Some("denied"));
        assert_eq!(mic_event(&Value::Null), None);
        assert_eq!(changed_keys(&map(vec![("keys", Value::List(vec![t("a"), Value::Int(1)]))])), ["a"]);
        assert!(changed_keys(&Value::Null).is_empty());
    }

    #[test]
    fn info_round_trip() {
        let i = Info {
            modes: ModeSet::of(&[Mode::Proximity, Mode::Broadcast]),
            activations: ActSet::default().with(Activation::Ptt),
            channel: Some("Red".into()),
        };
        let v = Value::decode(&i.to_value().encode()).unwrap();
        assert_eq!(Info::from_value(&v), i);
        let none = Info::default();
        assert_eq!(Info::from_value(&none.to_value()), none);
        assert_eq!(Info::from_value(&Value::Null), none);
    }

    #[test]
    fn picker() {
        let all = ModeSet(0b1111);
        assert_eq!(picker_modes(all, Mode::Proximity), [Mode::Global, Mode::Channel, Mode::Broadcast]);
        assert_eq!(picker_modes(ModeSet::of(&[Mode::Proximity]), Mode::Proximity), []);
        assert_eq!(picker_modes(all, Mode::Global).len(), 3);
    }

    #[test]
    fn volume_dialog() {
        assert_eq!(volume_text(100, &[]), "Master 100%. Nobody else is here.");
        let lines = vec![
            VolumeLine { alias: "Bob".into(), percent: 80, muted: false },
            VolumeLine { alias: "Eve".into(), percent: 100, muted: true },
        ];
        assert_eq!(volume_text(150, &lines), "Master 150%.\nBob: 80%\nEve: muted");
        let many: Vec<_> =
            (0..80).map(|i| VolumeLine { alias: format!("Player{i}"), percent: 100, muted: false }).collect();
        let text = volume_text(100, &many);
        assert!(text.len() <= 512, "{}", text.len());
        assert!(text.ends_with("more"));
    }
}
