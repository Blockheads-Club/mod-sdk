//! The lifecycle guard around our `RECORD_AUDIO` dialog.
//!
//! The system dialog is an activity: the game's activity pauses, Apportable calls `-[GameView willResignActive]`
//! (and `didEnterBackground` if it stops), and the game leaves an online world. The game itself keeps a world through
//! its own activities (the server welcome web view): `willResignActive` then calls `-[GameView startTimer]`, a 15 Hz
//! background timer that keeps updating the world and polling the network, and `didBecomeActive` cancels it. While the
//! guard is armed (only around our dialog) the hooks take that path: `willResignActive` → `startTimer`,
//! `didEnterBackground` / the matching `willEnterForeground` are skipped. `didBecomeActive` always runs the game's own
//! code and drops the guard. Unarmed, every hook just calls the original.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use bhmod_core::types::LogLevel;

use super::log;
use super::objc::{self, Id, Imp, Sel};

/// Whether in-world requests may rely on the guard. Turned off if a device shows that the guard does not keep the
/// connection (the dialog is then only shown outside a connection).
pub const IN_WORLD_REQUESTS: bool = true;

struct Orig {
    resign: Imp,
    background: Imp,
    foreground: Imp,
    active: Imp,
}
// SAFETY: method implementations, written once.
unsafe impl Send for Orig {}
unsafe impl Sync for Orig {}

static ORIG: OnceLock<Orig> = OnceLock::new();
static ARMED: AtomicBool = AtomicBool::new(false);
static SKIPPED_BACKGROUND: AtomicBool = AtomicBool::new(false);

const CLASS: &str = "GameView";

/// Hooks the four `GameView` lifecycle methods when the game has all of them (and `startTimer`). True when on.
pub fn install() -> bool {
    let names = ["willResignActive", "didEnterBackground", "willEnterForeground", "didBecomeActive", "startTimer"];
    if names.iter().any(|n| objc::method(CLASS, n, false).is_null()) {
        log(LogLevel::Warn, "voice: GameView lifecycle methods not found; the microphone is asked only outside worlds");
        return false;
    }
    let orig = Orig {
        resign: objc::swizzle(CLASS, "willResignActive", hook_resign as Imp),
        background: objc::swizzle(CLASS, "didEnterBackground", hook_background as Imp),
        foreground: objc::swizzle(CLASS, "willEnterForeground", hook_foreground as Imp),
        active: objc::swizzle(CLASS, "didBecomeActive", hook_active as Imp),
    };
    ORIG.set(orig).is_ok()
}

/// True when the guard is hooked and in-world requests are allowed.
pub fn available() -> bool {
    IN_WORLD_REQUESTS && ORIG.get().is_some()
}

/// Arms or drops the guard (Java, around the dialog).
pub fn arm(on: bool) {
    if ARMED.swap(on, Ordering::SeqCst) != on {
        log(LogLevel::Info, &format!("voice: lifecycle guard {}", if on { "armed" } else { "dropped" }));
    }
}

fn call(imp: Imp, this: Id, cmd: Sel) {
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel)>(imp)(this, cmd) };
}

extern "C" fn hook_resign(this: Id, cmd: Sel) {
    let o = ORIG.get().expect("hooks");
    if ARMED.load(Ordering::SeqCst) {
        log(LogLevel::Info, "voice: willResignActive during the microphone dialog: background timer, world kept");
        objc::send0(this, "startTimer");
        return;
    }
    call(o.resign, this, cmd);
}

extern "C" fn hook_background(this: Id, cmd: Sel) {
    let o = ORIG.get().expect("hooks");
    if ARMED.load(Ordering::SeqCst) {
        log(LogLevel::Info, "voice: didEnterBackground during the microphone dialog: skipped");
        SKIPPED_BACKGROUND.store(true, Ordering::SeqCst);
        return;
    }
    call(o.background, this, cmd);
}

extern "C" fn hook_foreground(this: Id, cmd: Sel) {
    let o = ORIG.get().expect("hooks");
    if SKIPPED_BACKGROUND.swap(false, Ordering::SeqCst) {
        log(LogLevel::Info, "voice: willEnterForeground after a skipped background: skipped");
        return;
    }
    call(o.foreground, this, cmd);
}

extern "C" fn hook_active(this: Id, cmd: Sel) {
    let o = ORIG.get().expect("hooks");
    call(o.active, this, cmd);
    arm(false);
}
