//! Weapon skins (the rules, shared by the Android glue in `android/weapons.rs` and tested on the desktop).
//!
//! The game draws a held tool as a sprite inside one big blockhead drawing function (`libApplication.so` +
//! [`DRAW`]): it binds the shared `Items` atlas, asks the exported `texCoordsForItemType` for the item's rectangle
//! in that atlas and draws a quad. There is no per-blockhead item texture, and the atlas is shared with every UI
//! icon, so a skin needs which blockhead is drawing and a hook inside the renderer:
//!
//! 1. the three entry points the game draws a blockhead with (the body, the item in its hand while it is used, the
//!    HUD portrait) tell [`Window::enter`] which blockhead it is, and [`Window::leave`] when they return;
//! 2. an inline hook on `texCoordsForItemType` answers a call that comes from inside [`DRAW`] (the return address
//!    says so) for a blockhead whose player wears a weapon cosmetic for that item with the mod texture's rectangle,
//!    after binding the mod texture;
//! 3. the swap is scoped to the held item's draw: the mod texture stays bound after the quad, and the game draws
//!    other sprites (the dig-target indicator, the HUD icons) through the same `texCoordsForItemType` without
//!    binding the atlas again, so the first call that is not a skinned held item (and the end of the blockhead's
//!    draw, and of the frame) binds the atlas back that was bound before the swap.
//!
//! Only the game build whose function prologue matches [`PROLOGUE`] is known: the address range is specific to it.

use std::ops::Range;

/// Offset of the blockhead drawing function in `libApplication.so` of the 1.7.6 build, and where it ends.
pub const DRAW: Range<u32> = 0xc0_0780..0xc0_1a98;
/// Its first four instructions (`push {r4-r8, r10, r11, lr}; add r11, sp, #24; sub sp, sp, #1152; bic sp, sp, #15`).
pub const PROLOGUE: [u32; 4] = [0xe92d_4df0, 0xe28d_b018, 0xe24d_dd12, 0xe3cd_d00f];
/// The exported function that answers the rectangle of an item in the atlas (`texCoordsForItemType(ItemType)`).
pub const TEX_COORDS: &str = "_Z20texCoordsForItemType8ItemType";
/// `GL_TEXTURE_2D`.
pub const TEXTURE_2D: u32 = 0xde1;
/// `GL_TEXTURE_BINDING_2D`.
pub const TEXTURE_BINDING_2D: u32 = 0x8069;

/// The three methods the game draws a blockhead with (class `Blockhead`).
pub const ENTRY_POINTS: [&str; 3] = [
    "draw:projectionMatrix:modelViewMatrix:cameraMinXWorld:cameraMaxXWorld:cameraMinYWorld:cameraMaxYWorld:",
    "drawTransparentInventoryItem:projectionMatrix:modelViewMatrix:cameraMinXWorld:cameraMaxXWorld:cameraMinYWorld:cameraMaxYWorld:",
    "drawForButtonProjectionMatrix:modelViewMatrix:",
];

/// True when the first instructions of the drawing function are the known build's.
pub fn is_known_build(prologue: &[u32]) -> bool {
    prologue == PROLOGUE
}

/// True when a return address (`lr` at the hook) lies inside the blockhead drawing function of the module at `base`.
pub fn in_draw(lr: u32, base: u32) -> bool {
    lr >= base.wrapping_add(DRAW.start) && lr < base.wrapping_add(DRAW.end)
}

/// What one thread knows while it draws blockheads.
#[derive(Debug, Default)]
pub struct Window {
    /// The blockhead being drawn (0: none, or a draw that is not the world's).
    bh: u32,
    /// The texture that was bound before the mod's texture replaced it; `Some` while a mod texture is bound.
    atlas: Option<u32>,
}

impl Window {
    /// A draw entry point runs for `bh`.
    pub fn enter(&mut self, bh: u32) {
        self.bh = bh;
    }

    /// A draw entry point returned: the window of its held item is over. The atlas to bind back, if a mod texture
    /// is bound.
    pub fn leave(&mut self) -> Option<u32> {
        self.bh = 0;
        self.atlas.take()
    }

    /// The blockhead whose held item a `texCoordsForItemType` call may skin: only calls from inside the blockhead
    /// drawing function (`in_draw`) belong to a blockhead. Any other call (UI, target indicator) ends the window.
    pub fn candidate(&mut self, in_draw: bool) -> Option<u32> {
        if in_draw && self.bh != 0 {
            Some(self.bh)
        } else {
            self.bh = 0;
            None
        }
    }

    /// The call was not skinned: the atlas to bind back before it goes on, if a mod texture is bound.
    pub fn unskinned(&mut self) -> Option<u32> {
        self.atlas.take()
    }

    /// The call is skinned and a mod texture is about to be bound. `bound` reads the texture bound now; it only
    /// runs when no mod texture is bound yet (the held item of the next blockhead keeps the one that is).
    pub fn skinned(&mut self, bound: impl FnOnce() -> u32) {
        if self.atlas.is_none() {
            self.atlas = Some(bound());
        }
    }

    /// True while a mod texture is bound.
    pub fn swapped(&self) -> bool {
        self.atlas.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_known_build_is_hooked() {
        assert!(is_known_build(&PROLOGUE));
        assert!(!is_known_build(&[0; 4]));
        assert!(!is_known_build(&PROLOGUE[..3]));
    }

    #[test]
    fn calls_from_the_drawing_function_are_recognised() {
        let base = 0x4000_0000;
        assert!(in_draw(base + DRAW.start, base));
        assert!(in_draw(base + DRAW.end - 4, base));
        assert!(!in_draw(base + DRAW.end, base));
        assert!(!in_draw(base + DRAW.start - 4, base));
        // A module near the top of the address space wraps instead of panicking.
        assert!(!in_draw(0x10, 0xffff_f000));
    }

    #[test]
    fn window_is_the_blockhead_drawing_function_only() {
        let mut w = Window::default();
        w.enter(0x1000);
        assert_eq!(w.candidate(true), Some(0x1000));
        // A call from the UI (the dig-target indicator) is not a blockhead's: no skin, and the window is closed.
        assert_eq!(w.candidate(false), None);
        // Even a call from inside the function has no blockhead after that.
        assert_eq!(w.candidate(true), None);
        w.enter(0x2000);
        assert_eq!(w.candidate(true), Some(0x2000));
        assert_eq!(w.leave(), None);
        assert_eq!(w.candidate(true), None);
    }

    #[test]
    fn atlas_is_bound_back_for_the_first_call_that_is_not_skinned() {
        let mut w = Window::default();
        assert!(!w.swapped());
        assert_eq!(w.unskinned(), None);
        w.skinned(|| 7);
        assert!(w.swapped());
        // The held item of the next blockhead keeps the mod texture bound (and the first atlas is remembered).
        // (it would read 0 if it asked again: the assert below checks the first one is kept).
        w.skinned(u32::default);
        // The target indicator's call: the atlas comes back before its quad.
        assert_eq!(w.unskinned(), Some(7));
        assert!(!w.swapped());
        assert_eq!(w.unskinned(), None);
    }

    #[test]
    fn the_end_of_a_blockhead_draw_binds_the_atlas_back() {
        let mut w = Window::default();
        w.enter(1);
        w.skinned(|| 9);
        assert_eq!(w.leave(), Some(9));
        assert!(!w.swapped());
        assert_eq!(w.leave(), None);
    }

    #[test]
    fn entry_points_are_the_three_draw_methods() {
        assert_eq!(ENTRY_POINTS.len(), 3);
        assert!(ENTRY_POINTS.iter().all(|s| s.starts_with("draw")));
    }
}
