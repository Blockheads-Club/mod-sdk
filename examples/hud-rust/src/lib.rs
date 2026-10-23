//! HUD demo (Rust twin of `hud-lua`): widgets in two slots, a notice, a dialog and its answer.

use std::cell::Cell;

use blockheads_mod::prelude::*;
use blockheads_mod::ui::{self, ButtonKind, Dialog, DialogButton, Field, NoticeLevel, Slot};

thread_local! {
    static COINS: Cell<i64> = const { Cell::new(1250) };
}

fn show_coins() -> Result<(), ApiError> {
    ui::counter("coins", Slot::TopRight, "coin", &COINS.get().to_string())
}

struct Hud;

impl Mod for Hud {
    fn on_enable() -> Result<(), String> {
        show_coins()?;
        ui::text("where", Slot::TopLeft, "X 1042 Y 88")?;
        ui::notice("HUD demo is on.", NoticeLevel::Success)?;

        // The position text follows the world step.
        events::on(EventKind::Tick, |ev| {
            if let events::Event::Tick(t) = ev.event() {
                if t.tick % 5 == 0 {
                    let _ = ui::text("where", Slot::TopLeft, &format!("tick {}", t.tick));
                }
            }
        })?;

        // Answers to dialogs of this mod.
        ui::on_answer(|answer| {
            if answer.button == 1 {
                let n: i64 = answer.text.as_deref().and_then(|t| t.parse().ok()).unwrap_or(0);
                COINS.set(COINS.get() - n);
                let _ = show_coins();
                let _ = ui::toast(&format!("You sent {n} coins."));
                // A dialog can also be closed from the mod's side.
                if let Ok(done) = ui::show_dialog(&Dialog {
                    title: "Done".into(),
                    text: String::new(),
                    field: None,
                    buttons: vec![DialogButton { label: "OK".into(), kind: ButtonKind::Secondary }],
                }) {
                    ui::close_dialog(done);
                }
            } else {
                let _ = ui::toast("Nothing was sent.");
                ui::remove_widget("where");
            }
        })?;

        ui::show_dialog(&Dialog {
            title: "Send coins".into(),
            text: "How many coins do you want to send?".into(),
            field: Some(Field { placeholder: "50".into(), initial: "10".into(), max_length: 6 }),
            buttons: vec![
                DialogButton { label: "Cancel".into(), kind: ButtonKind::Secondary },
                DialogButton { label: "Send".into(), kind: ButtonKind::Primary },
            ],
        })?;
        Ok(())
    }
}

blockheads_mod::export_mod!(Hud);
