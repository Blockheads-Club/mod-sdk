//! The fonts of the Java UI (IG-Kit): Tiny5 for titles, buttons, tabs and HUD numbers, Atkinson Hyperlegible Next
//! for reading text and Golos Text for Russian and Kazakh reading text. All three are SIL Open Font License 1.1
//! (licenses next to the files in `fonts/`). They are embedded here and handed to the Java side on demand
//! (`Native.font`), which writes them to its cache directory and loads them as typefaces: the loader zip stays
//! two files (library and dex) and the Patcher needs no asset support.

/// Font ids as the Java side asks for them.
pub const TINY5: i32 = 0;
/// Atkinson Hyperlegible Next, Regular (Latin and Latin Extended, so Turkish too).
pub const ATKINSON: i32 = 1;
/// Golos Text, Regular (Latin, Cyrillic and Cyrillic Extended, so Kazakh too).
pub const GOLOS: i32 = 2;

static TINY5_TTF: &[u8] = include_bytes!("../fonts/Tiny5-Regular.ttf");
static ATKINSON_TTF: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf");
static GOLOS_TTF: &[u8] = include_bytes!("../fonts/GolosText-Regular.ttf");

/// The TrueType bytes of a font id.
pub fn get(id: i32) -> Option<&'static [u8]> {
    match id {
        TINY5 => Some(TINY5_TTF),
        ATKINSON => Some(ATKINSON_TTF),
        GOLOS => Some(GOLOS_TTF),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_are_truetype() {
        for id in [TINY5, ATKINSON, GOLOS] {
            let f = get(id).expect("embedded");
            assert_eq!(&f[..4], &[0, 1, 0, 0], "font {id} is a TrueType file");
            assert!(f.len() > 10_000);
        }
        assert!(get(3).is_none());
        assert!(get(-1).is_none());
    }
}
