//! UI commands for the Java side, as JSON objects with a `t` (type) key. The Java UI renders them with the in-game
//! kit; texts are translated there from the codes.

use bhmod_client::session::{Failure, Offer, SkippedMod};
use bhmod_client::Refusal;
use bhmod_core::ui::{Button, UiCommand};
use bhmod_core::voice::Mode;
use bhmod_proto::{hex, RejectText};
use serde_json::{json, Value};

fn perms(o: &Offer) -> Value {
    let p = &o.info.permissions;
    json!({
        "world": p.world,
        "players": p.players,
        "netHttp": p.net_http,
        "channels": p.channels,
        "voice": p.voice,
        "native": p.native,
    })
}

/// The join prompt.
pub fn prompt(server: &str, required: bool, mods: &[Offer], download: u64) -> String {
    let list: Vec<Value> = mods
        .iter()
        .map(|o| {
            json!({
                "id": o.info.id,
                "version": o.info.version,
                "name": o.info.name,
                "description": o.info.description,
                "sides": o.info.sides,
                "required": o.info.client_required,
                "size": o.info.size,
                "sha256": hex(&o.info.sha256),
                "cached": o.cached,
                "client": o.client,
                "indexListed": o.info.index_listed,
                "permissions": perms(o),
            })
        })
        .collect();
    json!({"t": "prompt", "server": server, "required": required, "mods": list, "download": download}).to_string()
}

/// Download progress of one file.
pub fn progress(id: &str, index: usize, count: usize, done: u64, size: u64) -> String {
    json!({"t": "progress", "id": id, "index": index, "count": count, "done": done, "size": size}).to_string()
}

/// The join went ahead; close the join dialog.
pub fn joined() -> String {
    json!({"t": "joined"}).to_string()
}

/// The connection closed; close the join dialog.
pub fn closed() -> String {
    json!({"t": "closed"}).to_string()
}

/// The modded join stopped.
pub fn failed(server: &str, f: &Failure, loader: &str) -> String {
    let mut v = json!({"t": "failed", "server": server, "code": f.code(), "text": f.to_string(), "loader": loader});
    let id = match f {
        Failure::Refused { id, refusal } => {
            v["refusal"] = json!(refusal.code());
            if let Refusal::NeedsNewerLoader(r) | Refusal::NeedsNewerApi(r) = refusal {
                v["needs"] = json!(r);
            }
            Some(id)
        }
        Failure::Download { id, code } => {
            v["fileError"] = json!(code);
            Some(id)
        }
        Failure::Stalled(id) | Failure::HashMismatch(id) => Some(id),
        _ => None,
    };
    if let Some(id) = id {
        v["id"] = json!(id);
    }
    v.to_string()
}

/// Optional server mods this loader could not run (told once per join): `voice` is named "voice chat" by the UI.
pub fn skipped(server: &str, mods: &[SkippedMod]) -> String {
    let list: Vec<Value> = mods
        .iter()
        .map(|m| json!({"id": m.id, "version": m.version, "name": m.name, "refusal": m.refusal.code()}))
        .collect();
    json!({"t": "skipped", "server": server, "mods": list}).to_string()
}

/// The server's rejection text.
pub fn reject(server: &str, r: &RejectText) -> String {
    json!({"t": "reject", "server": server, "reason": r.reason_code, "text": r.text, "url": r.url}).to_string()
}

/// A mod's UI command (API 0.3 `ui`, already validated by the host) for the Java HUD and dialogs.
pub fn mod_ui(mod_id: &str, c: &UiCommand) -> String {
    let mut v = match c {
        UiCommand::SetWidget { id, widget } => json!({
            "t": "ui-set", "id": id, "slot": widget.slot.name(), "kind": widget.kind.name(),
            "icon": widget.icon, "title": widget.title, "text": widget.text,
        }),
        UiCommand::RemoveWidget { id } => json!({"t": "ui-remove", "id": id}),
        UiCommand::Toast { text } => json!({"t": "ui-toast", "text": text}),
        UiCommand::Notice { text, level } => json!({"t": "ui-notice", "text": text, "level": level.name()}),
        UiCommand::ShowDialog { id, dialog } => json!({
            "t": "ui-dialog", "id": id, "title": dialog.title, "text": dialog.text,
            "field": dialog.field.as_ref().map(|f| json!({"placeholder": f.placeholder, "initial": f.initial, "max": f.max_length})),
            "buttons": dialog.buttons.iter().map(|b| json!({"label": b.label, "kind": b.kind.name()})).collect::<Vec<_>>(),
        }),
        UiCommand::CloseDialog { id } => json!({"t": "ui-close", "id": id}),
        // API 0.3.4; the Java side ignores types it does not draw yet.
        UiCommand::SetOverhead { player, icon } => json!({"t": "ui-overhead", "player": player, "icon": icon.name()}),
        UiCommand::ClearOverhead { player } => json!({"t": "ui-overhead", "player": player, "icon": null}),
        UiCommand::SetButton { id, button } => return button_set(mod_id, id, button, None),
        UiCommand::RemoveButton { id } => json!({"t": "btn-remove", "id": id}),
        UiCommand::OpenSettings { mod_id } => json!({"t": "ui-settings", "target": mod_id}),
        UiCommand::ClearMod => json!({"t": "ui-clear"}),
    };
    v["mod"] = json!(mod_id);
    v.to_string()
}

/// A world opened or closed (`mods`: mods running; `settings`: the mods whose settings screen can open).
pub fn world(open: bool, mods: usize, settings: &[String]) -> String {
    json!({"t": "world", "open": open, "mods": mods, "settings": settings}).to_string()
}

/// An on-screen button (API 0.3.4) with the position the player dropped it at last (`saved`, screen fractions).
pub fn button_set(mod_id: &str, id: &str, b: &Button, saved: Option<(f32, f32)>) -> String {
    json!({
        "t": "btn-set", "mod": mod_id, "id": id, "icon": b.icon, "label": b.label, "x": b.x, "y": b.y,
        "size": b.size, "opacity": b.opacity, "saved": saved.map(|(x, y)| [x, y]),
    })
    .to_string()
}

/// The host-owned mic-on indicator.
pub fn mic_indicator(mode: Option<Mode>) -> String {
    json!({"t": "voice-mic", "on": mode.is_some(), "mode": mode.map(Mode::name)}).to_string()
}

/// The microphone is not allowed and may not be asked in a connection: allow it from the Mods screen.
pub fn mic_needs_menu() -> String {
    json!({"t": "voice-needs-mic"}).to_string()
}

/// The player refused the microphone.
pub fn mic_denied() -> String {
    json!({"t": "voice-denied"}).to_string()
}

/// A key binding row of the settings screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    /// The action id.
    pub action: String,
    /// The key now (`None`: cleared by the player).
    pub key: Option<String>,
    /// The mod's default key.
    pub default_key: String,
    /// True when the player did not rebind it.
    pub is_default: bool,
}

/// One player of the voice volume list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerVolume {
    /// The player id.
    pub id: String,
    /// The alias.
    pub name: String,
    /// 0-200 %.
    pub volume: u16,
    /// Locally muted.
    pub muted: bool,
}

/// The voice section of the settings screen (mods holding `voice`): the master volume and every speaker's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSection {
    /// 0-200 %.
    pub master: u16,
    /// Players with a voice session.
    pub players: Vec<PlayerVolume>,
}

/// A settings form: `({"fields": [...]}, {key: value})`.
pub type Form = (Value, Value);

fn form(f: &Form) -> Value {
    json!({"fields": f.0.get("fields").cloned().unwrap_or_else(|| json!([])), "values": f.1})
}

/// The settings screen of a mod: its client settings, key bindings, the voice master volume (mods holding `voice`)
/// and, for an admin, the server's settings of it.
pub fn settings_screen(
    mod_id: &str,
    name: Option<&std::collections::BTreeMap<String, String>>,
    client: Option<&Form>,
    keys: &[KeyRow],
    voice: Option<&VoiceSection>,
    server: Option<&Form>,
) -> String {
    let keys: Vec<Value> = keys
        .iter()
        .map(|k| json!({"action": k.action, "key": k.key, "default": k.default_key, "isDefault": k.is_default}))
        .collect();
    json!({
        "t": "settings", "mod": mod_id, "name": name.map_or_else(|| json!(mod_id), |n| json!(n)),
        "client": client.map(form), "server": server.map(form), "keys": keys,
        "voice": voice.map(|v| json!({
            "master": v.master,
            "players": v.players.iter().map(|p| json!({"id": p.id, "name": p.name, "volume": p.volume, "muted": p.muted})).collect::<Vec<_>>(),
        })),
    })
    .to_string()
}

/// The answer to a setting change of `key` (`message` shown when it failed; `restart`: applies after a restart).
pub fn settings_result(mod_id: &str, key: Option<&str>, ok: bool, message: Option<&str>, restart: bool) -> String {
    json!({"t": "settings-result", "mod": mod_id, "key": key, "ok": ok, "message": message, "restart": restart})
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bhmod_proto::{ModInfo, ModPermissions};

    fn v(s: String) -> Value {
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn shapes() {
        let info = ModInfo {
            id: "demo".into(),
            version: "1.0.0".into(),
            name: [("en".to_string(), "Demo".to_string())].into(),
            description: Default::default(),
            sides: vec!["client".into()],
            client_required: true,
            sha256: [1; 32],
            size: 10,
            permissions: ModPermissions { world: "read".into(), channels: true, ..Default::default() },
            source_url: None,
            index_listed: false,
        };
        let o = Offer { info, cached: false, client: true };
        let p = v(prompt("h:1", true, &[o], 10));
        assert_eq!(p["mods"][0]["permissions"]["world"], "read");
        assert_eq!(p["mods"][0]["name"]["en"], "Demo");
        assert_eq!(v(progress("demo", 0, 1, 5, 10))["done"], 5);
        assert_eq!(v(joined())["t"], "joined");
        assert_eq!(v(closed())["t"], "closed");
        assert_eq!(v(world(true, 2, &["m".to_string()]))["settings"][0], "m");
        let r = RejectText { reason_code: 3, text: "x".into(), url: Some("https://u".into()) };
        assert_eq!(v(reject("h", &r))["url"], "https://u");
        let f = |f: Failure| v(failed("h", &f, "0.1.0"));
        let x = f(Failure::Refused { id: "a".into(), refusal: Refusal::NeedsNewerLoader("^9".into()) });
        assert_eq!(
            (x["id"].as_str(), x["needs"].as_str(), x["refusal"].as_str()),
            (Some("a"), Some("^9"), Some("needs-newer-loader"))
        );
        let x = f(Failure::Refused { id: "a".into(), refusal: Refusal::Native });
        assert!(x.get("needs").is_none());
        assert_eq!(f(Failure::Download { id: "a".into(), code: 1 })["fileError"], 1);
        assert_eq!(f(Failure::HashMismatch("a".into()))["id"], "a");
        assert!(f(Failure::Declined).get("id").is_none());
    }

    #[test]
    fn mod_ui_shapes() {
        use bhmod_core::ui::{ButtonKind, Dialog, DialogButton, UiCommand};
        let close = v(mod_ui("m", &UiCommand::CloseDialog { id: 3 }));
        assert_eq!(
            (close["t"].as_str(), close["id"].as_u64(), close["mod"].as_str()),
            (Some("ui-close"), Some(3), Some("m"))
        );
        assert_eq!(v(mod_ui("m", &UiCommand::ClearMod))["t"], "ui-clear");
        let p = || "p1".to_string();
        let icon = bhmod_core::ui::OverheadIcon::NoVoice;
        assert_eq!(v(mod_ui("m", &UiCommand::SetOverhead { player: p(), icon }))["icon"], "no-voice");
        assert!(v(mod_ui("m", &UiCommand::ClearOverhead { player: p() }))["icon"].is_null());
        let button = bhmod_core::ui::Button { icon: "mic".into(), label: None, x: 0.5, y: 0.5, size: 64, opacity: 80 };
        let set = v(mod_ui("m", &UiCommand::SetButton { id: "ptt".into(), button: button.clone() }));
        assert_eq!(
            (set["t"].as_str(), set["size"].as_u64(), set["mod"].as_str()),
            (Some("btn-set"), Some(64), Some("m"))
        );
        assert!(set["saved"].is_null());
        assert_eq!(v(button_set("m", "ptt", &button, Some((0.25, 0.5))))["saved"][1], 0.5);
        assert_eq!(v(mod_ui("m", &UiCommand::RemoveButton { id: "ptt".into() }))["t"], "btn-remove");
        assert_eq!(v(mod_ui("m", &UiCommand::OpenSettings { mod_id: "voice".into() }))["target"], "voice");
        let d = Dialog {
            title: "T".into(),
            text: "x".into(),
            field: None,
            buttons: vec![DialogButton { label: "L".into(), kind: ButtonKind::Danger }],
        };
        let dlg = v(mod_ui("m", &UiCommand::ShowDialog { id: 1, dialog: d }));
        assert!(dlg["field"].is_null());
        assert_eq!(dlg["buttons"][0]["kind"], "danger");
    }

    #[test]
    fn skipped_shape() {
        let m = SkippedMod {
            id: "voice".into(),
            version: "1.0.0".into(),
            name: [("en".to_string(), "Voice".to_string())].into(),
            refusal: Refusal::NeedsNewerApi("^0.4".into()),
        };
        let x = v(skipped("h:1", &[m]));
        assert_eq!(
            (x["mods"][0]["id"].as_str(), x["mods"][0]["refusal"].as_str()),
            (Some("voice"), Some("needs-newer-api"))
        );
        assert_eq!(x["mods"][0]["name"]["en"], "Voice");
    }

    #[test]
    fn voice_and_settings_shapes() {
        assert_eq!(v(mic_indicator(Some(Mode::Global)))["mode"], "global");
        assert_eq!(v(mic_indicator(None))["on"], false);
        assert_eq!(v(mic_denied())["t"], "voice-denied");
        assert_eq!(v(mic_needs_menu())["t"], "voice-needs-mic");
        let form: Form = (json!({"fields": [{"key": "a"}]}), json!({"a": 1}));
        let keys = [KeyRow { action: "talk".into(), key: None, default_key: "v".into(), is_default: false }];
        let pv = PlayerVolume { id: "p1".into(), name: "ALICE".into(), volume: 50, muted: true };
        let vs = VoiceSection { master: 120, players: vec![pv] };
        let names = [("en".to_string(), "Voice chat".to_string())].into_iter().collect();
        let s = v(settings_screen("voice", Some(&names), Some(&form), &keys, Some(&vs), None));
        assert_eq!(s["name"]["en"], "Voice chat");
        assert_eq!(s["voice"]["players"][0]["name"], "ALICE");
        assert_eq!(s["client"]["fields"][0]["key"], "a");
        assert_eq!(s["client"]["values"]["a"], 1);
        assert!(s["server"].is_null());
        assert!(s["keys"][0]["key"].is_null());
        assert_eq!(s["voice"]["master"], 120);
        let s = v(settings_screen("m", None, None, &[], None, Some(&(json!({}), json!({})))));
        assert_eq!(s["name"], "m");
        assert_eq!(s["server"]["fields"], json!([]));
        assert!(s["voice"].is_null());
        let r = v(settings_result("m", Some("k"), false, Some("bad"), false));
        assert_eq!(r["key"], "k");
        assert_eq!((r["ok"].as_bool(), r["message"].as_str()), (Some(false), Some("bad")));
    }
}
