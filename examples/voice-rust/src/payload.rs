//! Event payloads (canonical CBOR maps) and the small client-side UI decisions: the `info` service answer, the mode
//! picker and the volume dialog text.

use blockheads_mod::cbor::Value;

use crate::model::{Mode, ModeSet, State};

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

/// `<mod>:ui-pick` `{player}` (a tapped or clicked blockhead).
pub fn pick_event(v: &Value) -> Option<String> {
    Some(v.get("player")?.as_str()?.to_string())
}

/// `voice:mode` `{mode}` (the default mode changed).
pub fn mode_event(v: &Value) -> Option<Mode> {
    Mode::parse(v.get("mode")?.as_str()?)
}

/// The text of the volume dialog for one player.
pub fn player_text(alias: &str, percent: u16, muted: bool) -> String {
    let state = if muted { "muted" } else { "not muted" };
    format!("{alias}: volume {percent} % ({state}) on this device. Type 0-200 and press Apply.")
}

/// The modes the long-press picker offers: the allowed ones except the current default (at most three, so a Cancel
/// button fits in a four-button dialog).
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
    fn pick_mode_and_player_text() {
        assert_eq!(pick_event(&map(vec![("player", t("7"))])).as_deref(), Some("7"));
        assert_eq!(pick_event(&Value::Null), None);
        assert_eq!(mode_event(&map(vec![("mode", t("channel"))])), Some(Mode::Channel));
        assert_eq!(mode_event(&map(vec![("mode", t("x"))])), None);
        assert_eq!(mode_event(&Value::Null), None);
        assert!(player_text("Bob", 80, false).starts_with("Bob: volume 80 % (not muted)"));
        assert!(player_text("Bob", 80, true).contains("(muted)"));
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
