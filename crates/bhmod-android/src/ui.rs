//! UI commands for the Java side, as JSON objects with a `t` (type) key. The Java UI renders them with the in-game
//! kit; texts are translated there from the codes.

use bhmod_client::session::{Failure, Offer};
use bhmod_client::Refusal;
use bhmod_core::ui::UiCommand;
use bhmod_proto::{hex, RejectText};
use serde_json::{json, Value};

fn perms(o: &Offer) -> Value {
    let p = &o.info.permissions;
    json!({
        "world": p.world,
        "players": p.players,
        "netHttp": p.net_http,
        "channels": p.channels,
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
        UiCommand::ClearMod => json!({"t": "ui-clear"}),
    };
    v["mod"] = json!(mod_id);
    v.to_string()
}

/// A world opened or closed (`mods`: mods running).
pub fn world(open: bool, mods: usize) -> String {
    json!({"t": "world", "open": open, "mods": mods}).to_string()
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
        assert_eq!(v(world(true, 2))["mods"], 2);
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
}
