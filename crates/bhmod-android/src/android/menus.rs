//! The "Mods" buttons inside the game's own menus (IG-Menus): the pause menu and the main menu are drawn by the
//! game in GL (`PauseUI`, `MainMenuUI`, built from `MJButton`s), not by view controllers, so each menu gets one more
//! `MJButton` that is tied to a button the game already has (its sibling):
//! - it is drawn right after the sibling, with the same matrix, so it shows exactly when the sibling does (the game
//!   decides which screen of the menu is up) and sits at the sibling's layout position, offset by [`crate::menus`];
//! - touches the game sends to the sibling are offered to the Mods button as well;
//! - it is created when the menu object is, released when the menu is released.
//!
//! The button's action opens the Mods screen of the Java UI.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use bhmod_core::types::LogLevel;

use super::objc::{self, Id, Imp, Sel};
use super::{java_post, log};
use crate::menus::{Frame, Menu};

/// The `GLKMatrix4` argument of `renderFrame:projectionMatrix:` as sixteen words. The type is 16-byte aligned, so
/// AAPCS skips `r3` and puts the whole matrix on the stack (`r3` is a padding word the callee ignores). Declared as
/// scalars, with the padding word, the call places every word exactly where the game's compiler did.
type W = u32;
type RenderFn = unsafe extern "C" fn(Id, Sel, f32, W, W, W, W, W, W, W, W, W, W, W, W, W, W, W, W, W);

/// Hooked methods' originals.
struct Orig {
    pause_init: Imp,
    main_init: Imp,
    pause_dealloc: Imp,
    main_dealloc: Imp,
    render: Imp,
    start: Imp,
    mv: Imp,
    end: Imp,
    in_ui: Imp,
}
// SAFETY: code addresses, written once.
unsafe impl Send for Orig {}
unsafe impl Sync for Orig {}
static ORIG: OnceLock<Orig> = OnceLock::new();

/// A Mods button and the menu it belongs to.
#[derive(Clone, Copy)]
struct Entry {
    menu: Menu,
    owner: Id,
    button: Id,
    anchor: Id,
    placed: Frame,
    /// Drawn in the last frame (so it takes touches).
    visible: bool,
    /// `setFrame:` ran on the GL thread: it builds the title's texture, which needs the GL context, and the menus are
    /// created off that thread, so the first frame does it.
    drawn: bool,
}
// SAFETY: plain pointers to game objects, only touched on the game's UI thread.
unsafe impl Send for Entry {}

/// Sibling button -> its Mods button.
static BUTTONS: Mutex<Option<HashMap<usize, Entry>>> = Mutex::new(None);
/// The label of the button (set from Java in the player's language).
static LABEL: Mutex<String> = Mutex::new(String::new());

fn with_buttons<R>(f: impl FnOnce(&mut HashMap<usize, Entry>) -> R) -> R {
    let mut g = BUTTONS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

/// Sets the button label (`MODS`, `MODLAR`, ...); buttons made later use it.
pub fn set_label(s: &str) {
    *LABEL.lock().unwrap_or_else(|e| e.into_inner()) = s.to_string();
}

fn label() -> String {
    let l = LABEL.lock().unwrap_or_else(|e| e.into_inner());
    if l.is_empty() {
        "MODS".to_string()
    } else {
        l.clone()
    }
}

fn frame_of(view: Id) -> Option<Frame> {
    let p = objc::ivar_ptr(view, "MJView", "frame") as *const f32;
    if p.is_null() {
        return None;
    }
    let f = unsafe { std::slice::from_raw_parts(p, 4) };
    Some(Frame { x: f[0], y: f[1], w: f[2], h: f[3] })
}

/// Width and height of the game's `WindowInfo` (its first two floats).
fn window_size(window: Id) -> (f32, f32) {
    if window.is_null() {
        return (0.0, 0.0);
    }
    let w = unsafe { std::slice::from_raw_parts(window as *const f32, 2) };
    (w[0], w[1])
}

fn set_frame(button: Id, f: Frame) {
    unsafe {
        objc::msg::<unsafe extern "C" fn(Id, Sel, f32, f32, f32, f32)>()(
            button,
            objc::sel("setFrame:"),
            f.x,
            f.y,
            f.w,
            f.h,
        );
    }
}

fn hidden(view: Id) -> bool {
    let p = objc::ivar_ptr(view, "MJView", "hidden") as *const i8;
    !p.is_null() && unsafe { *p != 0 }
}

/// The action of the Mods button: opens the Mods screen.
extern "C" fn mods_pressed(_this: Id, _cmd: Sel, _sender: Id) {
    java_post(&serde_json::json!({"t": "open-mods"}).to_string());
}

/// Creates the Mods button for a freshly initialised menu.
fn attach(menu: Menu, owner: Id) {
    if owner.is_null() {
        return;
    }
    let sibling = objc::ivar(owner, menu.owner_class(), menu.sibling_ivar());
    if sibling.is_null() {
        log(
            LogLevel::Warn,
            &format!("{}: no {} to put the Mods button next to", menu.owner_class(), menu.sibling_ivar()),
        );
        return;
    }
    let anchor = objc::ivar(owner, menu.owner_class(), menu.anchor_ivar());
    let (Some(sf), cache, window) = (
        frame_of(anchor),
        objc::ivar(owner, menu.owner_class(), "cache"),
        objc::ivar(owner, menu.owner_class(), "windowInfo"),
    ) else {
        return;
    };
    let f = menu.place(sf, window_size(window)).unwrap_or(sf);
    let title = objc::nsstring(&label());
    let alloc = objc::send0(objc::class("MJButton"), "alloc");
    let button = unsafe {
        objc::msg::<unsafe extern "C" fn(Id, Sel, f32, f32, f32, f32, Id, Id, Id) -> Id>()(
            alloc,
            objc::sel("initWithFrame:cache:windowInfo:title:"),
            f.x,
            f.y,
            f.w,
            f.h,
            cache,
            window,
            title,
        )
    };
    if button.is_null() {
        return;
    }
    // Same look as the game's plain buttons: copy their textures.
    let style = objc::ivar(owner, menu.owner_class(), menu.style_ivar());
    for (getter, setter) in
        [("backgroundTexture", "setBackgroundTexture:"), ("backgroundSelectedTexture", "setBackgroundSelectedTexture:")]
    {
        let tex = objc::ivar(style, "MJButton", getter);
        if !tex.is_null() {
            objc::send1(button, setter, tex);
        }
    }
    objc::send2(button, "setTarget:action:", owner, objc::sel("bhcModsButton:"));
    log(
        LogLevel::Info,
        &format!("{}: Mods button at {:?} next to {} {:?}", menu.owner_class(), f, menu.sibling_ivar(), sf),
    );
    with_buttons(|m| {
        m.insert(sibling as usize, Entry { menu, owner, button, anchor, placed: f, visible: false, drawn: false })
    });
}

/// Releases the Mods button of a menu that goes away.
fn detach(menu: Menu, owner: Id) {
    let sibling = objc::ivar(owner, menu.owner_class(), menu.sibling_ivar());
    if let Some(e) = with_buttons(|m| m.remove(&(sibling as usize))) {
        objc::send0(e.button, "release");
    }
}

/// The Mods button of a sibling button, when it is drawn (it takes touches only then).
fn visible_entry(view: Id) -> Option<Entry> {
    with_buttons(|m| m.get(&(view as usize)).copied()).filter(|e| e.visible)
}

extern "C" fn hook_pause_init(this: Id, cmd: Sel, world: Id, window: Id, cache: Id) -> Id {
    let orig = ORIG.get().expect("hooks").pause_init;
    let r = unsafe {
        std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel, Id, Id, Id) -> Id>(orig)(
            this, cmd, world, window, cache,
        )
    };
    super::guard("PauseUI init", || attach(Menu::Pause, r));
    r
}

extern "C" fn hook_main_init(this: Id, cmd: Sel, delegate: Id, window: Id, cache: Id, cloud: Id) -> Id {
    let orig = ORIG.get().expect("hooks").main_init;
    let r = unsafe {
        std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel, Id, Id, Id, Id) -> Id>(orig)(
            this, cmd, delegate, window, cache, cloud,
        )
    };
    super::guard("MainMenuUI init", || attach(Menu::Main, r));
    r
}

extern "C" fn hook_pause_dealloc(this: Id, cmd: Sel) {
    super::guard("PauseUI dealloc", || detach(Menu::Pause, this));
    let orig = ORIG.get().expect("hooks").pause_dealloc;
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel)>(orig)(this, cmd) };
}

extern "C" fn hook_main_dealloc(this: Id, cmd: Sel) {
    super::guard("MainMenuUI dealloc", || detach(Menu::Main, this));
    let orig = ORIG.get().expect("hooks").main_dealloc;
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel)>(orig)(this, cmd) };
}

#[allow(clippy::too_many_arguments)]
extern "C" fn hook_render(
    this: Id,
    cmd: Sel,
    dt: f32,
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
) {
    let orig = ORIG.get().expect("hooks").render;
    let render = unsafe { std::mem::transmute::<Imp, RenderFn>(orig) };
    unsafe { render(this, cmd, dt, pad, m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11, m12, m13, m14, m15) };
    let Some(mut e) = with_buttons(|m| m.get(&(this as usize)).copied()) else { return };
    super::guard("Mods button render", || {
        let key = this as usize;
        let want = frame_of(e.anchor)
            .and_then(|af| e.menu.place(af, window_size(objc::ivar(e.owner, e.menu.owner_class(), "windowInfo"))));
        let Some(want) = want.filter(|_| !hidden(this)) else {
            set_visible(key, false);
            return;
        };
        // `setFrame:` builds the title texture, so it runs once (on the GL thread, in the first frame) and then
        // only when the layout moved.
        if !e.drawn || want != e.placed {
            set_frame(e.button, want);
            e.placed = want;
            e.drawn = true;
        }
        e.visible = true;
        with_buttons(|b| b.insert(key, e));
        unsafe { render(e.button, cmd, dt, pad, m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11, m12, m13, m14, m15) };
    });
}

fn set_visible(key: usize, v: bool) {
    with_buttons(|b| {
        if let Some(e) = b.get_mut(&key) {
            e.visible = v;
        }
    });
}

type TouchFn = unsafe extern "C" fn(Id, Sel, f32, f32) -> i8;

extern "C" fn hook_start(this: Id, cmd: Sel, x: f32, y: f32) -> i8 {
    let orig = ORIG.get().expect("hooks").start;
    let game = unsafe { std::mem::transmute::<Imp, TouchFn>(orig) };
    let r = unsafe { game(this, cmd, x, y) };
    match visible_entry(this) {
        Some(e) if r == 0 => unsafe { game(e.button, cmd, x, y) },
        _ => r,
    }
}

extern "C" fn hook_move(this: Id, cmd: Sel, x: f32, y: f32) {
    let orig = ORIG.get().expect("hooks").mv;
    let game = unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel, f32, f32)>(orig) };
    unsafe { game(this, cmd, x, y) };
    if let Some(e) = visible_entry(this) {
        unsafe { game(e.button, cmd, x, y) };
    }
}

extern "C" fn hook_end(this: Id, cmd: Sel, x: f32, y: f32) {
    let orig = ORIG.get().expect("hooks").end;
    let game = unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel, f32, f32)>(orig) };
    unsafe { game(this, cmd, x, y) };
    if let Some(e) = visible_entry(this) {
        unsafe { game(e.button, cmd, x, y) };
    }
}

extern "C" fn hook_in_ui(this: Id, cmd: Sel, x: f32, y: f32) -> i8 {
    let orig = ORIG.get().expect("hooks").in_ui;
    let game = unsafe { std::mem::transmute::<Imp, TouchFn>(orig) };
    let r = unsafe { game(this, cmd, x, y) };
    match visible_entry(this) {
        Some(e) if r == 0 => unsafe { game(e.button, cmd, x, y) },
        _ => r,
    }
}

/// Installs the menu hooks; false when a game method is missing (the buttons are then simply absent).
pub fn install() -> bool {
    let types = "v12@0:4@8";
    let added =
        ["PauseUI", "MainMenuUI"].iter().all(|c| objc::add_method(c, "bhcModsButton:", mods_pressed as Imp, types));
    let pause_init = objc::swizzle("PauseUI", "initWithWorld:windowInfo:cache:", hook_pause_init as Imp);
    let main_init =
        objc::swizzle("MainMenuUI", "initWithDelegate:windowInfo:cache:cloudInterface:", hook_main_init as Imp);
    let pause_dealloc = objc::swizzle("PauseUI", "dealloc", hook_pause_dealloc as Imp);
    let main_dealloc = objc::swizzle("MainMenuUI", "dealloc", hook_main_dealloc as Imp);
    let render = objc::swizzle("MJButton", "renderFrame:projectionMatrix:", hook_render as Imp);
    let start = objc::swizzle("MJControl", "startTouch:", hook_start as Imp);
    let mv = objc::swizzle("MJControl", "moveTouch:", hook_move as Imp);
    let end = objc::swizzle("MJControl", "endTouch:", hook_end as Imp);
    let in_ui = objc::swizzle("MJControl", "touchIsInUI:", hook_in_ui as Imp);
    let all = [pause_init, main_init, pause_dealloc, main_dealloc, render, start, mv, end, in_ui];
    let ok = added && all.iter().all(|p| !p.is_null());
    if ok {
        let _ = ORIG.set(Orig { pause_init, main_init, pause_dealloc, main_dealloc, render, start, mv, end, in_ui });
    } else {
        log(LogLevel::Warn, "a menu method was not found; the Mods buttons are not added to the game's menus");
    }
    let _: *const c_void = std::ptr::null();
    ok
}
