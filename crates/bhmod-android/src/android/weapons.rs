//! Weapon skins on the game's blockheads (Android): the held tool of a blockhead shows the texture of the weapon
//! cosmetic its player wears. The rules and the reasons are in [`crate::weapons`]; this is the glue with the game:
//!
//! - the three `Blockhead` draw methods are replaced (through the engine's Objective-C hooks, so they come off with
//!   the rest) by functions that note which blockhead draws, run the original with the very same arguments (the
//!   matrices are 16-byte aligned `GLKMatrix4` unions: AAPCS skips `r3` for them, so they are declared as the words
//!   the game's compiler placed, with a padding word), and close the window afterwards;
//! - `texCoordsForItemType` is inline-hooked (the engine's primitive, the same as for native mods). The hook needs
//!   the caller's return address to know the call comes from the blockhead drawing function, which Rust cannot
//!   read, so the hook is a two-instruction ARM shim ([`bhmod_weapon_shim`]) that passes `lr` as the third argument.
//!
//! Everything is installed while a loaded mod declares a weapon cosmetic and taken out again when none does.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use bhmod_content::Part;
use bhmod_core::types::LogLevel;

use super::cosmetics::{player_of, texture, with_content};
use super::log;
use super::native::with_engine;
use super::objc::{self, Id, Sel};
use crate::weapons::{self, Window};

type W = u32;

/// Owner id of the hooks in the engine (native mods count up from 0).
const OWNER: u32 = u32::MAX - 1;

const OFF: u32 = 0;
const ON: u32 = 1;
/// The build is not the known one, or hooking failed: do not try again this run.
const GAVE_UP: u32 = 2;
static STATE: AtomicU32 = AtomicU32::new(OFF);

/// Original implementations of the three entry points (draw body, held item, portrait) and of `texCoordsForItemType`.
static ORIG: [AtomicU32; 4] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static BASE: AtomicU32 = AtomicU32::new(0);
static BIND: AtomicU32 = AtomicU32::new(0);
static GET_INT: AtomicU32 = AtomicU32::new(0);
/// Handles of the hooks (to take them out again).
static HANDLES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

thread_local! {
    static WINDOW: RefCell<Window> = RefCell::new(Window::default());
}

fn with_window<R>(f: impl FnOnce(&mut Window) -> R) -> R {
    WINDOW.with(|w| f(&mut w.borrow_mut()))
}

fn active() -> bool {
    STATE.load(Ordering::Relaxed) == ON
}

/// `glBindTexture(GL_TEXTURE_2D, name)` through the game's own entry.
fn bind(name: u32) {
    let f = BIND.load(Ordering::Relaxed);
    if f != 0 {
        // SAFETY: the game's own GL wrapper, called on the render thread with plain integers.
        unsafe { std::mem::transmute::<usize, unsafe extern "C" fn(u32, u32)>(f as usize)(weapons::TEXTURE_2D, name) };
    }
}

/// The texture bound now (`glGetIntegerv(GL_TEXTURE_BINDING_2D)`).
fn bound_texture() -> u32 {
    let f = GET_INT.load(Ordering::Relaxed);
    let mut v: i32 = 0;
    if f != 0 {
        // SAFETY: as [`bind`]; `v` is a valid out parameter.
        unsafe {
            std::mem::transmute::<usize, unsafe extern "C" fn(u32, *mut i32)>(f as usize)(
                weapons::TEXTURE_BINDING_2D,
                &mut v,
            )
        };
    }
    v as u32
}

/// Binds the game's own texture back if a mod texture is bound (the end of a draw).
fn leave() {
    if let Some(atlas) = with_window(Window::leave) {
        bind(atlas);
    }
}

fn enter(bh: Id) {
    with_window(|w| w.enter(bh as usize as u32));
}

/// The skin of the held item `item` of blockhead `bh`: the GL name of the mod's texture and its rectangle.
fn skin(bh: u32, item: u16) -> Option<(u32, [f32; 4])> {
    let bh = bh as usize as Id;
    let player = player_of(bh);
    let id = with_content(|c| c.weapon(&player, item)).flatten()?;
    let tex = texture(id, Part::Main);
    if tex.is_null() {
        return None;
    }
    let uv = with_content(|c| c.texture(id, Part::Main).ok().flatten().map(|g| g.uv())).flatten()?;
    let name = objc::send0(tex, "name") as usize as u32;
    (name != 0).then_some((name, uv))
}

/// The replacement of `texCoordsForItemType(ItemType) -> Vector4` (the result through `sret` in `r0`, the item in
/// `r1`); `lr` is the caller's return address, passed by [`bhmod_weapon_shim`].
extern "C" fn tex_coords(sret: *mut u32, item: u32, lr: u32) -> *mut u32 {
    let in_draw = weapons::in_draw(lr, BASE.load(Ordering::Relaxed));
    let skinned = with_window(|w| w.candidate(in_draw))
        .and_then(|bh| std::panic::catch_unwind(|| skin(bh, item as u16)).ok().flatten());
    let Some((name, uv)) = skinned else {
        if let Some(atlas) = with_window(Window::unskinned) {
            bind(atlas);
        }
        return original(sret, item);
    };
    // SAFETY: `sret` is the caller's result slot for a four-float vector.
    for (i, f) in uv.iter().enumerate() {
        unsafe { *sret.add(i) = f.to_bits() };
    }
    with_window(|w| w.skinned(bound_texture));
    bind(name);
    sret
}

fn original(sret: *mut u32, item: u32) -> *mut u32 {
    let f = ORIG[3].load(Ordering::Relaxed);
    // SAFETY: the engine's trampoline for the hooked function, same signature.
    unsafe { std::mem::transmute::<usize, unsafe extern "C" fn(*mut u32, u32) -> *mut u32>(f as usize)(sret, item) }
}

// The shim the inline hook jumps to: `tex_coords(r0, r1, lr)`. The hook is a branch, so `lr` is still the game's
// return address; `tex_coords` returns straight to the game. ARM mode (the entry patch jumps in ARM state), and the
// Rust function is Thumb: the jump goes through `bx` with the address kept in [`TARGET`] (position independent).
std::arch::global_asm!(
    ".arm",
    ".section .text.bhmod_weapon_shim,\"ax\",%progbits",
    ".global bhmod_weapon_shim",
    ".hidden bhmod_weapon_shim",
    ".type bhmod_weapon_shim,%function",
    "bhmod_weapon_shim:",
    "mov r2, lr",
    "ldr r3, 2f",
    "1: add r3, pc, r3",
    "ldr r3, [r3]",
    "bx r3",
    "2: .word {t} - (1b + 8)",
    ".thumb",
    t = sym TARGET,
);

/// Address of [`tex_coords`] (with the Thumb bit), read by the shim.
static TARGET: AtomicU32 = AtomicU32::new(0);

extern "C" {
    fn bhmod_weapon_shim();
}

type DrawFn = unsafe extern "C" fn(
    Id,
    Sel,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
);
type ButtonFn = unsafe extern "C" fn(
    Id,
    Sel,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
    W,
);

/// A draw method with the camera arguments (`word` is the float or the item the method starts with).
#[allow(clippy::too_many_arguments)]
fn forward_draw(slot: usize, this: Id, cmd: Sel, word: W, pad: W, m: [W; 16], n: [W; 16], c: [W; 4]) {
    enter(this);
    let f = ORIG[slot].load(Ordering::Relaxed);
    // SAFETY: the original implementation with the arguments it was called with.
    unsafe {
        std::mem::transmute::<usize, DrawFn>(f as usize)(
            this, cmd, word, pad, m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11], m[12],
            m[13], m[14], m[15], n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], n[8], n[9], n[10], n[11], n[12],
            n[13], n[14], n[15], c[0], c[1], c[2], c[3],
        )
    };
    leave();
}

macro_rules! draw_entry {
    ($name:ident, $slot:expr) => {
        #[allow(clippy::too_many_arguments)]
        extern "C" fn $name(
            this: Id,
            cmd: Sel,
            word: W,
            pad: W,
            m0: W,
            m1: W,
            m2: W,
            m3: W,
            m4: W,
            m5: W,
            m6: W,
            m7: W,
            m8: W,
            m9: W,
            m10: W,
            m11: W,
            m12: W,
            m13: W,
            m14: W,
            m15: W,
            n0: W,
            n1: W,
            n2: W,
            n3: W,
            n4: W,
            n5: W,
            n6: W,
            n7: W,
            n8: W,
            n9: W,
            n10: W,
            n11: W,
            n12: W,
            n13: W,
            n14: W,
            n15: W,
            c0: W,
            c1: W,
            c2: W,
            c3: W,
        ) {
            forward_draw(
                $slot,
                this,
                cmd,
                word,
                pad,
                [m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11, m12, m13, m14, m15],
                [n0, n1, n2, n3, n4, n5, n6, n7, n8, n9, n10, n11, n12, n13, n14, n15],
                [c0, c1, c2, c3],
            );
        }
    };
}
draw_entry!(hook_draw, 0);
draw_entry!(hook_ghost, 1);

#[allow(clippy::too_many_arguments)]
extern "C" fn hook_button(
    this: Id,
    cmd: Sel,
    m0: W,
    m1: W,
    m2: W,
    m3: W,
    m4: W,
    m5: W,
    m6: W,
    m7: W,
    m8: W,
    m9: W,
    m10: W,
    m11: W,
    m12: W,
    m13: W,
    m14: W,
    m15: W,
    n0: W,
    n1: W,
    n2: W,
    n3: W,
    n4: W,
    n5: W,
    n6: W,
    n7: W,
    n8: W,
    n9: W,
    n10: W,
    n11: W,
    n12: W,
    n13: W,
    n14: W,
    n15: W,
) {
    enter(this);
    let f = ORIG[2].load(Ordering::Relaxed);
    // SAFETY: as [`forward_draw`]; the two matrices are 32 words from `r2` on.
    unsafe {
        std::mem::transmute::<usize, ButtonFn>(f as usize)(
            this, cmd, m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11, m12, m13, m14, m15, n0, n1, n2, n3, n4, n5,
            n6, n7, n8, n9, n10, n11, n12, n13, n14, n15,
        )
    };
    leave();
}

fn fail(why: &str) {
    log(LogLevel::Warn, &format!("weapon skins are off: {why}"));
    STATE.store(GAVE_UP, Ordering::Relaxed);
}

/// Installs the hooks.
fn install() {
    with_engine(|e, host| {
        let Some(base) = host.modules().iter().find(|m| m.name.ends_with("libApplication.so")).map(|m| m.base) else {
            return fail("libApplication.so is not loaded");
        };
        let prologue: Option<Vec<u32>> = host
            .read(base + weapons::DRAW.start, 16)
            .map(|b| b.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect());
        if !prologue.is_some_and(|p| weapons::is_known_build(&p)) {
            return fail("this is not the game build they know");
        }
        let find = |names: &[&str]| names.iter().find_map(|n| host.symbol(None, n));
        let (Some(bind), Some(get_int)) =
            (find(&["__wrap_glBindTexture", "glBindTexture"]), find(&["__wrap_glGetIntegerv", "glGetIntegerv"]))
        else {
            return fail("the GL entry points were not found");
        };
        BASE.store(base, Ordering::Relaxed);
        TARGET.store(tex_coords as *const () as usize as u32, Ordering::Relaxed);
        BIND.store(bind, Ordering::Relaxed);
        GET_INT.store(get_int, Ordering::Relaxed);
        let address = |f: *const ()| f as usize as u32;
        let replacements =
            [address(hook_draw as *const ()), address(hook_ghost as *const ()), address(hook_button as *const ())];
        let mut handles = Vec::new();
        let mut result: Result<(), String> = Ok(());
        for (i, (sel, imp)) in weapons::ENTRY_POINTS.iter().zip(replacements).enumerate() {
            match e.hook_objc(host, OWNER, "Blockhead", sel, false, imp) {
                Ok(h) => {
                    ORIG[i].store(h.original, Ordering::Relaxed);
                    handles.push(h.handle);
                }
                Err(err) => {
                    result = Err(err.message);
                    break;
                }
            }
        }
        if result.is_ok() {
            match e.hook_symbol(
                host,
                OWNER,
                Some("libApplication.so"),
                weapons::TEX_COORDS,
                address(bhmod_weapon_shim as *const ()),
            ) {
                Ok(h) => {
                    ORIG[3].store(h.original, Ordering::Relaxed);
                    handles.push(h.handle);
                }
                Err(err) => result = Err(err.message),
            }
        }
        match result {
            Ok(()) => {
                *HANDLES.lock().unwrap_or_else(|p| p.into_inner()) = handles;
                STATE.store(ON, Ordering::Relaxed);
                log(LogLevel::Info, "weapon skins: hooks installed");
            }
            Err(why) => {
                e.unhook_owner(host, OWNER);
                fail(&why);
            }
        }
    });
}

/// Takes the hooks out (no mod declares a weapon cosmetic any more).
fn remove() {
    if active() {
        // The inline hook goes first: nothing then calls the entry points' windows with a stale original.
        with_engine(|e, host| e.unhook_owner(host, OWNER));
        HANDLES.lock().unwrap_or_else(|p| p.into_inner()).clear();
        STATE.store(OFF, Ordering::Relaxed);
        log(LogLevel::Info, "weapon skins: hooks removed");
    }
}

/// Installs or removes the hooks as mods declare weapon cosmetics (once per frame on the loader's thread), and binds
/// the game's own texture back if a frame ended inside a window.
pub fn sync() {
    leave();
    let want = with_content(|c| c.has_weapons()).unwrap_or(false);
    match (want, STATE.load(Ordering::Relaxed)) {
        (true, OFF) => install(),
        (false, ON) => remove(),
        _ => {}
    }
}
