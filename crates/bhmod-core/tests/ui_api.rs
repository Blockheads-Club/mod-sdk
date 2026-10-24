//! The 0.3 `ui` interface through the Luau mapping: commands reach the bridge validated, limits and errors are
//! typed, dialog answers come back as the owner's `<mod-id>:ui-action` custom event and nobody else's, and a mod
//! that goes away takes its widgets with it.

mod common;

use std::rc::Rc;

use bhmod_core::types::Side;
use bhmod_core::ui::{ButtonKind, Dialog, DialogButton, Field, NoticeLevel, Slot, UiCommand, Widget, WidgetKind};
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;

const MAIN: &str = r#"
local mod = {}
function mod.on_enable()
    bh.ui.set_widget("coins", { slot = "top-right", kind = "counter", icon = "coin", text = "1 250" })
    bh.ui.set_widget("pos", { text = "X 1042 Y 88" })
    bh.ui.set_widget("claims", { kind = "panel", title = "Land", text = "Home\nFarm" })
    assert(bh.ui.remove_widget("pos") == true and bh.ui.remove_widget("pos") == false)
    bh.ui.toast("Ben sent you 50 coins.")
    bh.ui.notice("Building is off here.", "warning")
    bh.ui.notice("plain")
    local id = bh.ui.show_dialog({
        title = "Send coins",
        text = "How many?",
        field = { placeholder = "50", initial = "10", max_length = 6 },
        buttons = { { label = "Cancel" }, { label = "Send", kind = "primary" } },
    })
    assert(id == 1)
    assert(not pcall(bh.ui.show_dialog, { title = "second", buttons = { { label = "x" } } }), "one dialog at a time")
    bh.events.on_custom("ui:ui-action", function(ev)
        print("action", ev.payload.dialog, ev.payload.button, tostring(ev.payload.text))
    end)

    -- errors
    assert(not pcall(bh.ui.set_widget, "Bad Id", { text = "x" }))
    assert(not pcall(bh.ui.set_widget, "a", { slot = "middle" }))
    assert(not pcall(bh.ui.set_widget, "a", { kind = "blink" }))
    assert(not pcall(bh.ui.set_widget, "a", { text = string.rep("x", 257) }))
    assert(not pcall(bh.ui.notice, "x", "loud"))
    assert(not pcall(bh.ui.toast, string.rep("x", 201)))
    assert(not pcall(bh.ui.notice, string.rep("x", 201)))
    assert(not pcall(bh.ui.show_dialog, { buttons = { { label = "x" } } }), "title is required")
    assert(not pcall(bh.ui.show_dialog, { title = "t", buttons = { { kind = "primary" } } }), "label is required")
    assert(not pcall(bh.ui.show_dialog, { title = "t", buttons = { { label = "x", kind = "huge" } } }))
    assert(not pcall(bh.ui.show_dialog, { title = "t" }), "buttons are required")
    assert(not pcall(bh.ui.close_dialog, "one"))
    for i = 1, 14 do bh.ui.set_widget("w" .. i, { text = "x" }) end
    local ok, err = pcall(bh.ui.set_widget, "overflow", { text = "x" })
    assert(not ok and string.find(tostring(err), "limit exceeded", 1, true), tostring(err))
end
return mod
"#;

const OTHER: &str = r#"
local mod = {}
function mod.on_enable()
    bh.events.on_custom("ui:ui-action", function(ev) print("EAVESDROP", ev.payload.text) end)
    assert(bh.ui.close_dialog(1) == false, "not its dialog")
    bh.ui.toast("from other")
end
return mod
"#;

/// Writes an unpacked client-side Luau mod.
fn client_mod(mods: &std::path::Path, id: &str, main: &str) {
    let d = mods.join(id);
    std::fs::create_dir_all(d.join("client")).unwrap();
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = {{ en = \"N\" }}\nauthors = [\"a\"]\nlicense = \"MIT\"\n\
         api = \"^0.3\"\n[sides]\nserver = false\nclient = true\n[entry]\nclient = {{ runtime = \"lua\", path = \"client/main.luau\" }}\n"
    );
    std::fs::write(d.join("mod.toml"), toml).unwrap();
    std::fs::write(d.join("client/main.luau"), main).unwrap();
}

fn host(no_ui: bool, mods: &[(&str, &str)]) -> (tempfile::TempDir, Rc<FakeBridge>, ModHost) {
    let tmp = tempfile::tempdir().unwrap();
    for (id, main) in mods {
        client_mod(tmp.path(), id, main);
    }
    let b = FakeBridge::new();
    b.no_ui.set(no_ui);
    let h = ModHost::load(HostOptions::new(tmp.path(), Side::Client), b.clone());
    (tmp, b, h)
}

/// What a mod sees on a host without UI (the server): every call says `unsupported`, nothing stays open.
const PROBE: &str = r#"
local mod = {}
local function unsupported(f, ...)
    local ok, err = pcall(f, ...)
    assert(not ok and string.find(tostring(err), "unsupported", 1, true), tostring(err))
end
function mod.on_enable()
    unsupported(bh.ui.set_widget, "a", { text = "x" })
    unsupported(bh.ui.toast, "x")
    unsupported(bh.ui.notice, "x")
    local d = { title = "t", buttons = { { label = "x" } } }
    unsupported(bh.ui.show_dialog, d)
    unsupported(bh.ui.show_dialog, d)
    assert(bh.ui.remove_widget("a") == false and bh.ui.close_dialog(1) == false)
    print("probed")
end
return mod
"#;
#[test]
fn commands_limits_and_answers() {
    let (_tmp, b, mut h) = host(false, &[("ui", MAIN), ("other", OTHER)]);
    h.enable("World", "wid");
    assert_eq!(
        h.mods().iter().map(|m| m.state.clone()).collect::<Vec<_>>(),
        vec![ModState::Enabled; 2],
        "{}",
        b.log_text()
    );
    let all = b.take_ui();
    let mine: Vec<_> = all.iter().filter(|(m, _)| m == "ui").map(|(_, c)| c.clone()).collect();
    assert_eq!(
        mine[..8],
        [
            UiCommand::SetWidget {
                id: "coins".into(),
                widget: Widget {
                    slot: Slot::TopRight,
                    kind: WidgetKind::Counter,
                    icon: Some("coin".into()),
                    title: None,
                    text: "1 250".into()
                }
            },
            UiCommand::SetWidget {
                id: "pos".into(),
                widget: Widget {
                    slot: Slot::TopLeft,
                    kind: WidgetKind::Text,
                    icon: None,
                    title: None,
                    text: "X 1042 Y 88".into()
                }
            },
            UiCommand::SetWidget {
                id: "claims".into(),
                widget: Widget {
                    slot: Slot::TopLeft,
                    kind: WidgetKind::Panel,
                    icon: None,
                    title: Some("Land".into()),
                    text: "Home\nFarm".into()
                }
            },
            UiCommand::RemoveWidget { id: "pos".into() },
            UiCommand::Toast { text: "Ben sent you 50 coins.".into() },
            UiCommand::Notice { text: "Building is off here.".into(), level: NoticeLevel::Warning },
            UiCommand::Notice { text: "plain".into(), level: NoticeLevel::Info },
            UiCommand::ShowDialog {
                id: 1,
                dialog: Dialog {
                    title: "Send coins".into(),
                    text: "How many?".into(),
                    field: Some(Field { placeholder: "50".into(), initial: "10".into(), max_length: 6 }),
                    buttons: vec![
                        DialogButton { label: "Cancel".into(), kind: ButtonKind::Secondary },
                        DialogButton { label: "Send".into(), kind: ButtonKind::Primary },
                    ],
                },
            },
        ]
    );
    // 14 more widgets fit (coins and claims make 16), the overflow was refused before reaching the bridge.
    assert_eq!(mine.len(), 8 + 14);
    assert!(all.iter().any(|(m, c)| m == "other" && *c == UiCommand::Toast { text: "from other".into() }));

    // The answer reaches the dialog's owner only.
    assert!(h.ui_result(1, 1, Some("25".into())));
    assert!(!h.ui_result(1, 1, None), "answered once");
    assert!(b.has_log("action\t1\t1\t25"), "{}", b.log_text());
    assert!(!b.has_log("EAVESDROP"));
    assert!(!h.ui_result(2, 0, None), "unknown dialog");

    // A mod that goes away takes its widgets: the host sends ClearMod once.
    h.disable_all();
    let cleared = b.take_ui();
    assert_eq!(cleared.iter().filter(|(m, c)| m == "ui" && *c == UiCommand::ClearMod).count(), 1);
    assert_eq!(cleared.iter().filter(|(m, c)| m == "other" && *c == UiCommand::ClearMod).count(), 0);
}

#[test]
fn a_bridge_without_ui_says_unsupported() {
    let (_tmp, b, mut h) = host(true, &[("probe", PROBE)]);
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    assert!(b.has_log("probed"), "{}", b.log_text());
    assert!(b.take_ui().is_empty());
    h.disable_all();
    assert!(b.take_ui().is_empty(), "no ClearMod without UI commands");
}
