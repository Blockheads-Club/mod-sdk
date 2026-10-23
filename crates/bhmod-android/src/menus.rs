//! Where the "Mods" buttons go in the game's own menus. The game lays its menus out in GL units from a window
//! description; the Mods button follows a sibling button the game already has and takes its size, so it always
//! matches the game's buttons (IG-Menus: "same size as the game's own buttons, placed last").

/// A view frame in the game's units: origin and size (`CGRect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Left.
    pub x: f32,
    /// Bottom.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

/// The menus that get a Mods button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// The pause menu (`PauseUI`), drawn after "RESUME".
    Pause,
    /// The main menu (`MainMenuUI`), "MORE GAMES!".
    Main,
}

impl Menu {
    /// The game class of the menu.
    pub fn owner_class(self) -> &'static str {
        match self {
            Menu::Pause => "PauseUI",
            Menu::Main => "MainMenuUI",
        }
    }

    /// The sibling button the Mods button is drawn after (the last button the game draws, so everything else of the
    /// menu is known by then) and whose touches it shares.
    pub fn sibling_ivar(self) -> &'static str {
        match self {
            Menu::Pause => "resumeButton",
            Menu::Main => "moreGamesButton",
        }
    }

    /// The button whose frame the Mods button is placed by.
    pub fn anchor_ivar(self) -> &'static str {
        match self {
            Menu::Pause => "resumeButton",
            Menu::Main => "moreGamesButton",
        }
    }

    /// The button whose textures the Mods button copies (the game's plain blue button).
    pub fn style_ivar(self) -> &'static str {
        match self {
            Menu::Pause => "exitButton",
            Menu::Main => "moreGamesButton",
        }
    }

    /// The Mods button's frame, `None` when there is no room for it. `anchor` is the anchor button's frame, `window`
    /// the game's window size in the same units (the game's menus are centred on the origin, y up).
    /// - Pause menu: the panel is 264 x 320 units and full, so the button goes beside it (landscape) or under it
    ///   (portrait), at the size of the game's buttons (the anchor, "RESUME", is a full-width one).
    /// - Main menu: the game animates its buttons, so the button sits in the bottom-right corner, 120 x 40.
    pub fn place(self, anchor: Frame, window: (f32, f32)) -> Option<Frame> {
        match self {
            Menu::Pause => {
                let beside = window.0 / 2.0 - PANEL_HALF_W - GAP;
                if beside >= MIN_BESIDE {
                    Some(Frame { x: PANEL_HALF_W + GAP, y: anchor.y, w: beside.min(anchor.w / 2.0), h: anchor.h })
                } else if window.1 / 2.0 - PANEL_HALF_H - GAP >= anchor.h {
                    Some(Frame { x: anchor.x, y: -PANEL_HALF_H - GAP - anchor.h, w: anchor.w, h: anchor.h })
                } else {
                    None
                }
            }
            Menu::Main => Some(Frame { x: window.0 / 2.0 - 128.0, y: -window.1 / 2.0 + 8.0, w: 120.0, h: 40.0 }),
        }
    }
}

/// Half the width and height of the pause panel the game draws, and the gap to it.
const PANEL_HALF_W: f32 = 132.0;
const PANEL_HALF_H: f32 = 160.0;
const GAP: f32 = 8.0;
/// Narrowest button worth placing beside the panel.
const MIN_BESIDE: f32 = 100.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement() {
        let resume = Frame { x: -122.0, y: -150.0, w: 244.0, h: 40.0 };
        // landscape phone: beside the panel, half the width of a full button
        let f = Menu::Pause.place(resume, (812.0, 375.0)).unwrap();
        assert_eq!(f, Frame { x: 140.0, y: -150.0, w: 122.0, h: 40.0 });
        // a window just wide enough keeps what fits
        let f = Menu::Pause.place(resume, (2.0 * (132.0 + 8.0 + 100.0), 375.0)).unwrap();
        assert_eq!((f.x, f.w), (140.0, 100.0));
        // portrait: under the panel at full width
        let f = Menu::Pause.place(resume, (375.0, 812.0)).unwrap();
        assert_eq!(f, Frame { x: -122.0, y: -208.0, w: 244.0, h: 40.0 });
        // no room anywhere
        assert_eq!(Menu::Pause.place(resume, (375.0, 375.0)), None);
        let f = Menu::Main.place(resume, (812.0, 375.0)).unwrap();
        assert_eq!(f, Frame { x: 278.0, y: -179.5, w: 120.0, h: 40.0 });
        assert_eq!(
            (Menu::Pause.owner_class(), Menu::Pause.sibling_ivar(), Menu::Pause.anchor_ivar()),
            ("PauseUI", "resumeButton", "resumeButton")
        );
        assert_eq!((Menu::Pause.style_ivar(), Menu::Main.style_ivar()), ("exitButton", "moreGamesButton"));
        assert_eq!(
            (Menu::Main.owner_class(), Menu::Main.sibling_ivar(), Menu::Main.anchor_ivar()),
            ("MainMenuUI", "moreGamesButton", "moreGamesButton")
        );
    }
}
