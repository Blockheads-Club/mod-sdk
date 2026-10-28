//! Mod UI (`blockheads:api@0.3.0` `ui`): HUD widgets, toasts, notices and dialogs of client mods; since 0.3.4 also
//! overhead icons, on-screen buttons and the settings screen.
//!
//! Mods send data; the host program draws it with its own kit. This module holds the data types, the validation of
//! what a mod may send (ids, lengths, counts) and the bookkeeping of what each mod has on screen, so the Android
//! loader and the PC client enforce the same limits. The drawing side receives validated [`UiCommand`]s through
//! [`crate::GameBridge::ui`].

use std::collections::{BTreeMap, BTreeSet};

use crate::types::ApiError;

/// Most widgets one mod may have.
pub const MAX_WIDGETS: usize = 16;
/// Longest widget id in characters.
pub const MAX_ID: usize = 32;
/// Longest widget text in bytes.
pub const MAX_WIDGET_TEXT: usize = 256;
/// Longest widget title in bytes.
pub const MAX_TITLE: usize = 48;
/// Longest toast or notice in bytes.
pub const MAX_NOTICE: usize = 200;
/// Longest dialog title in bytes.
pub const MAX_DIALOG_TITLE: usize = 64;
/// Longest dialog text in bytes.
pub const MAX_DIALOG_TEXT: usize = 512;
/// Longest dialog button label in bytes.
pub const MAX_BUTTON: usize = 24;
/// Longest dialog field in characters.
pub const MAX_FIELD: u32 = 200;
/// Most buttons of a dialog (4 since API 0.3.4).
pub const MAX_BUTTONS: usize = 4;
/// The `button` of a `ui-action` event when the dialog was closed without an answer (back, leaving the world).
pub const CLOSED: u32 = u32::MAX;
/// Most on-screen buttons one mod may have (0.3.4).
pub const MAX_HUD_BUTTONS: usize = 4;
/// On-screen button diameter range in density-independent points (0.3.4).
pub const BUTTON_SIZE: (u16, u16) = (32, 160);
/// On-screen button opacity range in percent (0.3.4).
pub const BUTTON_OPACITY: (u8, u8) = (10, 100);

macro_rules! named_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vm])* $v),+
        }

        impl $name {
            /// The WIT name.
            pub fn name(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }

            /// Parses a WIT name.
            pub fn parse(s: &str) -> Option<$name> {
                match s { $($s => Some($name::$v),)+ _ => None }
            }
        }
    };
}

named_enum! {
    /// `ui.slot`.
    Slot {
        /// Under the game's HUD, top left.
        TopLeft = "top-left",
        /// Top right.
        TopRight = "top-right",
    }
}

named_enum! {
    /// `ui.widget-kind`.
    WidgetKind {
        /// One line of text.
        Text = "text",
        /// An icon and a number.
        Counter = "counter",
        /// A title and up to four lines.
        Panel = "panel",
    }
}

named_enum! {
    /// `ui.notice-level`.
    NoticeLevel {
        /// Neutral.
        Info = "info",
        /// Something to watch.
        Warning = "warning",
        /// It worked.
        Success = "success",
        /// It failed.
        Error = "error",
    }
}

named_enum! {
    /// `ui.button-kind`.
    ButtonKind {
        /// The main answer.
        Primary = "primary",
        /// Another answer.
        Secondary = "secondary",
        /// A destructive answer.
        Danger = "danger",
    }
}

named_enum! {
    /// `ui.overhead-icon` (0.3.4).
    OverheadIcon {
        /// Talking.
        Speaking = "speaking",
        /// No voice chat.
        NoVoice = "no-voice",
        /// Muted their microphone.
        SelfMute = "self-mute",
        /// Hears nobody.
        Deafen = "deafen",
        /// Muted by an admin.
        AdminMute = "admin-mute",
    }
}

named_enum! {
    /// The `action` of a `<mod-id>:ui-button` event (0.3.4).
    ButtonAction {
        /// Touch down / mouse down.
        Press = "press",
        /// Held 500 ms without moving (still pressed).
        LongPress = "long-press",
        /// Touch up, or a drag started.
        Release = "release",
    }
}

/// `ui.button` (0.3.4).
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    /// A kit icon name.
    pub icon: String,
    /// A label under the icon.
    pub label: Option<String>,
    /// Default centre, fraction of the screen width.
    pub x: f32,
    /// Default centre, fraction of the screen height from the top.
    pub y: f32,
    /// Diameter in dp.
    pub size: u16,
    /// Opacity in percent.
    pub opacity: u8,
}

/// `ui.widget`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widget {
    /// Where it goes.
    pub slot: Slot,
    /// What it looks like.
    pub kind: WidgetKind,
    /// A kit icon name.
    pub icon: Option<String>,
    /// The panel title.
    pub title: Option<String>,
    /// The text.
    pub text: String,
}

/// `ui.dialog-button`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogButton {
    /// The label.
    pub label: String,
    /// The look.
    pub kind: ButtonKind,
}

/// `ui.field`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Shown while empty.
    pub placeholder: String,
    /// The starting text.
    pub initial: String,
    /// Most characters.
    pub max_length: u32,
}

/// `ui.dialog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    /// The title.
    pub title: String,
    /// The text.
    pub text: String,
    /// The text field, if any.
    pub field: Option<Field>,
    /// One to three buttons.
    pub buttons: Vec<DialogButton>,
}

/// A validated request to the drawing side. `mod_id` goes along in [`crate::GameBridge::ui`].
#[derive(Debug, Clone, PartialEq)]
pub enum UiCommand {
    /// Show or replace a widget.
    SetWidget {
        /// The widget id, unique within the mod.
        id: String,
        /// The widget.
        widget: Widget,
    },
    /// Remove a widget.
    RemoveWidget {
        /// The widget id.
        id: String,
    },
    /// Show a toast.
    Toast {
        /// The text.
        text: String,
    },
    /// Show a notice.
    Notice {
        /// The text.
        text: String,
        /// The look.
        level: NoticeLevel,
    },
    /// Open a dialog.
    ShowDialog {
        /// The dialog id (answers come back with it).
        id: u32,
        /// The dialog.
        dialog: Dialog,
    },
    /// Close a dialog without an answer.
    CloseDialog {
        /// The dialog id.
        id: u32,
    },
    /// (0.3.4) Show an icon above a player's blockhead (replaces the mod's icon on that player).
    SetOverhead {
        /// The player id.
        player: String,
        /// The icon.
        icon: OverheadIcon,
    },
    /// (0.3.4) Remove the mod's icon from a player.
    ClearOverhead {
        /// The player id.
        player: String,
    },
    /// (0.3.4) Show or update an on-screen button.
    SetButton {
        /// The button id, unique within the mod.
        id: String,
        /// The button.
        button: Button,
    },
    /// (0.3.4) Remove an on-screen button.
    RemoveButton {
        /// The button id.
        id: String,
    },
    /// (0.3.4) Open the settings screen of mod `mod_id` (client settings, key bindings, server settings for admins).
    OpenSettings {
        /// The mod whose settings to show.
        mod_id: String,
    },
    /// The mod is gone: remove every widget, dialog, overhead icon and button of it.
    ClearMod,
}

fn invalid(what: &str) -> ApiError {
    ApiError::InvalidArgument(what.to_string())
}

fn limit(what: &str) -> ApiError {
    ApiError::LimitExceeded(what.to_string())
}

fn text(what: &str, s: &str, max: usize) -> Result<(), ApiError> {
    if s.len() > max {
        return Err(limit(&format!("{what} longer than {max} bytes")));
    }
    Ok(())
}

/// A widget id: 1 to [`MAX_ID`] characters of `a-z0-9_.-`.
pub fn check_id(id: &str) -> Result<(), ApiError> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ID
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(invalid(&format!("widget id must be 1 to {MAX_ID} characters of a-z 0-9 _ . -")))
    }
}

/// Checks a widget's texts.
pub fn check_widget(w: &Widget) -> Result<(), ApiError> {
    text("widget text", &w.text, MAX_WIDGET_TEXT)?;
    text("widget title", w.title.as_deref().unwrap_or(""), MAX_TITLE)?;
    text("widget icon", w.icon.as_deref().unwrap_or(""), MAX_ID)
}

/// Checks a toast or notice text.
pub fn check_notice(s: &str) -> Result<(), ApiError> {
    text("notice", s, MAX_NOTICE)
}

/// Checks a dialog.
pub fn check_dialog(d: &Dialog) -> Result<(), ApiError> {
    text("dialog title", &d.title, MAX_DIALOG_TITLE)?;
    text("dialog text", &d.text, MAX_DIALOG_TEXT)?;
    if d.buttons.is_empty() || d.buttons.len() > MAX_BUTTONS {
        return Err(invalid(&format!("a dialog needs 1 to {MAX_BUTTONS} buttons")));
    }
    for b in &d.buttons {
        if b.label.is_empty() {
            return Err(invalid("a dialog button needs a label"));
        }
        text("button label", &b.label, MAX_BUTTON)?;
    }
    if let Some(f) = &d.field {
        if f.max_length == 0 || f.max_length > MAX_FIELD {
            return Err(invalid(&format!("field max-length must be 1 to {MAX_FIELD}")));
        }
        text("field placeholder", &f.placeholder, MAX_NOTICE)?;
        if f.initial.chars().count() > f.max_length as usize {
            return Err(limit("field initial text longer than max-length"));
        }
    }
    Ok(())
}

/// Checks an on-screen button (0.3.4).
pub fn check_button(b: &Button) -> Result<(), ApiError> {
    text("button icon", &b.icon, MAX_ID)?;
    text("button label", b.label.as_deref().unwrap_or(""), MAX_BUTTON)?;
    if !(0.0..=1.0).contains(&b.x) || !(0.0..=1.0).contains(&b.y) {
        return Err(invalid("button x and y must be 0.0 to 1.0"));
    }
    if !(BUTTON_SIZE.0..=BUTTON_SIZE.1).contains(&b.size) {
        return Err(invalid(&format!("button size must be {} to {}", BUTTON_SIZE.0, BUTTON_SIZE.1)));
    }
    if !(BUTTON_OPACITY.0..=BUTTON_OPACITY.1).contains(&b.opacity) {
        return Err(invalid(&format!("button opacity must be {} to {}", BUTTON_OPACITY.0, BUTTON_OPACITY.1)));
    }
    Ok(())
}

/// What each mod (by slot index) has on screen.
#[derive(Debug, Default)]
pub struct UiState {
    widgets: BTreeMap<usize, BTreeSet<String>>,
    buttons: BTreeMap<usize, BTreeSet<String>>,
    overheads: BTreeMap<usize, BTreeSet<String>>,
    dialogs: BTreeMap<u32, usize>,
    next_dialog: u32,
}

impl UiState {
    /// Checks that `owner` may show widget `id` (a new one must fit the per-mod count).
    pub fn check_set(&self, owner: usize, id: &str, w: &Widget) -> Result<(), ApiError> {
        check_id(id)?;
        check_widget(w)?;
        let mine = self.widgets.get(&owner);
        if mine.is_none_or(|m| !m.contains(id)) && mine.map_or(0, |m| m.len()) >= MAX_WIDGETS {
            return Err(limit("widgets per mod"));
        }
        Ok(())
    }

    /// Records a shown widget.
    pub fn added(&mut self, owner: usize, id: &str) {
        self.widgets.entry(owner).or_default().insert(id.to_string());
    }

    /// Forgets a widget; false when the mod had none by that id.
    pub fn removed(&mut self, owner: usize, id: &str) -> bool {
        self.widgets.get_mut(&owner).is_some_and(|m| m.remove(id))
    }

    /// Checks that `owner` may open a dialog and returns its id: one dialog at a time per mod.
    pub fn open_dialog(&mut self, owner: usize, d: &Dialog) -> Result<u32, ApiError> {
        check_dialog(d)?;
        if self.dialogs.values().any(|o| *o == owner) {
            return Err(limit("open dialogs per mod"));
        }
        self.next_dialog = self.next_dialog.wrapping_add(1).max(1);
        self.dialogs.insert(self.next_dialog, owner);
        Ok(self.next_dialog)
    }

    /// Forgets a dialog that failed to open or was closed by its mod; false when `owner` did not have it open.
    pub fn close_dialog(&mut self, owner: usize, id: u32) -> bool {
        if self.dialogs.get(&id) == Some(&owner) {
            self.dialogs.remove(&id);
            return true;
        }
        false
    }

    /// The loader answered a dialog: the owner, once.
    pub fn answered(&mut self, id: u32) -> Option<usize> {
        self.dialogs.remove(&id)
    }

    /// Checks that `owner` may show button `id` (a new one must fit the per-mod count).
    pub fn check_button(&self, owner: usize, id: &str, b: &Button) -> Result<(), ApiError> {
        check_id(id)?;
        check_button(b)?;
        let mine = self.buttons.get(&owner);
        if mine.is_none_or(|m| !m.contains(id)) && mine.map_or(0, |m| m.len()) >= MAX_HUD_BUTTONS {
            return Err(limit("buttons per mod"));
        }
        Ok(())
    }

    /// Records a shown button.
    pub fn button_added(&mut self, owner: usize, id: &str) {
        self.buttons.entry(owner).or_default().insert(id.to_string());
    }

    /// Forgets a button; false when the mod had none by that id.
    pub fn button_removed(&mut self, owner: usize, id: &str) -> bool {
        self.buttons.get_mut(&owner).is_some_and(|m| m.remove(id))
    }

    /// True when `owner` has button `id` (input from the loader for an unknown button is dropped).
    pub fn has_button(&self, owner: usize, id: &str) -> bool {
        self.buttons.get(&owner).is_some_and(|m| m.contains(id))
    }

    /// Records an overhead icon of `owner` on `player`.
    pub fn overhead_set(&mut self, owner: usize, player: &str) {
        self.overheads.entry(owner).or_default().insert(player.to_string());
    }

    /// Forgets an overhead icon; false when there was none.
    pub fn overhead_cleared(&mut self, owner: usize, player: &str) -> bool {
        self.overheads.get_mut(&owner).is_some_and(|m| m.remove(player))
    }

    /// The mod is gone. True when it had anything on screen.
    pub fn clear_owner(&mut self, owner: usize) -> bool {
        let mut had = false;
        for map in [&mut self.widgets, &mut self.buttons, &mut self.overheads] {
            had |= map.remove(&owner).is_some_and(|m| !m.is_empty());
        }
        let before = self.dialogs.len();
        self.dialogs.retain(|_, o| *o != owner);
        had || self.dialogs.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(text: &str) -> Widget {
        Widget { slot: Slot::TopLeft, kind: WidgetKind::Text, icon: None, title: None, text: text.into() }
    }

    fn dialog() -> Dialog {
        Dialog {
            title: "t".into(),
            text: "x".into(),
            field: None,
            buttons: vec![DialogButton { label: "OK".into(), kind: ButtonKind::Primary }],
        }
    }

    #[test]
    fn names_round_trip() {
        for s in ["top-left", "top-right"] {
            assert_eq!(Slot::parse(s).unwrap().name(), s);
        }
        for s in ["text", "counter", "panel"] {
            assert_eq!(WidgetKind::parse(s).unwrap().name(), s);
        }
        for s in ["info", "warning", "success", "error"] {
            assert_eq!(NoticeLevel::parse(s).unwrap().name(), s);
        }
        for s in ["primary", "secondary", "danger"] {
            assert_eq!(ButtonKind::parse(s).unwrap().name(), s);
        }
        for s in ["speaking", "no-voice", "self-mute", "deafen", "admin-mute"] {
            assert_eq!(OverheadIcon::parse(s).unwrap().name(), s);
        }
        for s in ["press", "long-press", "release"] {
            assert_eq!(ButtonAction::parse(s).unwrap().name(), s);
        }
        assert!(Slot::parse("middle").is_none());
    }

    #[test]
    fn ids_and_texts() {
        assert!(check_id("coins.1_a-b").is_ok());
        for bad in ["", "A", "a b", &"a".repeat(33)] {
            assert!(matches!(check_id(bad), Err(ApiError::InvalidArgument(_))), "{bad}");
        }
        assert!(check_widget(&widget("ok")).is_ok());
        assert!(matches!(check_widget(&widget(&"x".repeat(257))), Err(ApiError::LimitExceeded(_))));
        let mut w = widget("t");
        w.title = Some("t".repeat(49));
        assert!(check_widget(&w).is_err());
        w.title = None;
        w.icon = Some("i".repeat(33));
        assert!(check_widget(&w).is_err());
        assert!(check_notice("fine").is_ok());
        assert!(check_notice(&"n".repeat(201)).is_err());
    }

    #[test]
    fn widget_counts() {
        let mut s = UiState::default();
        for i in 0..MAX_WIDGETS {
            let id = format!("w{i}");
            s.check_set(1, &id, &widget("a")).unwrap();
            s.added(1, &id);
        }
        // replacing is fine, a new one is not, another mod is unaffected
        s.check_set(1, "w0", &widget("b")).unwrap();
        assert!(matches!(s.check_set(1, "new", &widget("b")), Err(ApiError::LimitExceeded(_))));
        s.check_set(2, "new", &widget("b")).unwrap();
        assert!(s.check_set(2, "BAD", &widget("b")).is_err());
        assert!(s.removed(1, "w0"));
        assert!(!s.removed(1, "w0"));
        assert!(!s.removed(9, "w0"));
        s.check_set(1, "new", &widget("b")).unwrap();
    }

    #[test]
    fn dialogs() {
        let mut s = UiState::default();
        let a = s.open_dialog(1, &dialog()).unwrap();
        assert!(matches!(s.open_dialog(1, &dialog()), Err(ApiError::LimitExceeded(_))));
        let b = s.open_dialog(2, &dialog()).unwrap();
        assert_ne!(a, b);
        assert!(!s.close_dialog(2, a));
        assert!(s.close_dialog(1, a));
        assert_eq!(s.answered(b), Some(2));
        assert_eq!(s.answered(b), None);
        s.next_dialog = u32::MAX;
        assert_eq!(s.open_dialog(1, &dialog()).unwrap(), 1);
        assert!(s.clear_owner(1));
        assert!(!s.clear_owner(1));
        s.added(3, "w");
        assert!(s.clear_owner(3));
        s.added(4, "w");
        s.removed(4, "w");
        assert!(!s.clear_owner(4));
    }

    #[test]
    fn dialog_checks() {
        let mut d = dialog();
        assert!(check_dialog(&d).is_ok());
        d.buttons.clear();
        assert!(matches!(check_dialog(&d), Err(ApiError::InvalidArgument(_))));
        d.buttons = vec![DialogButton { label: "a".into(), kind: ButtonKind::Danger }; 5];
        assert!(check_dialog(&d).is_err());
        d.buttons = vec![DialogButton { label: String::new(), kind: ButtonKind::Danger }];
        assert!(check_dialog(&d).is_err());
        d.buttons = vec![DialogButton { label: "x".repeat(25), kind: ButtonKind::Danger }];
        assert!(matches!(check_dialog(&d), Err(ApiError::LimitExceeded(_))));
        let mut d = dialog();
        d.title = "t".repeat(65);
        assert!(check_dialog(&d).is_err());
        let mut d = dialog();
        d.text = "t".repeat(513);
        assert!(check_dialog(&d).is_err());
        let mut d = dialog();
        d.field = Some(Field { placeholder: "p".into(), initial: "ab".into(), max_length: 5 });
        assert!(check_dialog(&d).is_ok());
        for max in [0, 201] {
            d.field = Some(Field { placeholder: String::new(), initial: String::new(), max_length: max });
            assert!(check_dialog(&d).is_err());
        }
        d.field = Some(Field { placeholder: "p".repeat(201), initial: String::new(), max_length: 5 });
        assert!(check_dialog(&d).is_err());
        d.field = Some(Field { placeholder: String::new(), initial: "abcdef".into(), max_length: 5 });
        assert!(matches!(check_dialog(&d), Err(ApiError::LimitExceeded(_))));
    }

    fn button() -> Button {
        Button { icon: "mic".into(), label: Some("Talk".into()), x: 0.9, y: 0.7, size: 64, opacity: 80 }
    }

    #[test]
    fn buttons_and_overheads() {
        assert!(check_button(&button()).is_ok());
        let bad = [
            Button { icon: "i".repeat(33), ..button() },
            Button { label: Some("l".repeat(25)), ..button() },
            Button { x: 1.5, ..button() },
            Button { y: -0.1, ..button() },
            Button { x: f32::NAN, ..button() },
            Button { size: 31, ..button() },
            Button { size: 161, ..button() },
            Button { opacity: 9, ..button() },
            Button { opacity: 101, ..button() },
        ];
        for b in bad {
            assert!(check_button(&b).is_err(), "{b:?}");
        }
        let mut s = UiState::default();
        assert!(s.check_button(1, "BAD", &button()).is_err());
        assert!(s.check_button(1, "b", &Button { size: 1, ..button() }).is_err());
        for i in 0..MAX_HUD_BUTTONS {
            s.check_button(1, &format!("b{i}"), &button()).unwrap();
            s.button_added(1, &format!("b{i}"));
        }
        s.check_button(1, "b0", &button()).unwrap();
        assert!(matches!(s.check_button(1, "new", &button()), Err(ApiError::LimitExceeded(_))));
        assert!(s.has_button(1, "b1"));
        assert!(!s.has_button(2, "b1"));
        assert!(s.button_removed(1, "b1"));
        assert!(!s.button_removed(1, "b1"));
        assert!(!s.button_removed(7, "b1"));
        s.overhead_set(5, "p");
        assert!(s.overhead_cleared(5, "p"));
        assert!(!s.overhead_cleared(5, "p"));
        assert!(!s.overhead_cleared(6, "p"));
        assert!(!s.clear_owner(5));
        s.overhead_set(5, "p");
        assert!(s.clear_owner(5));
        assert!(s.clear_owner(1));
    }
}
