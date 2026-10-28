//! The lifecycle guard around our `RECORD_AUDIO` dialog.
//!
//! The system dialog is an activity: the game's activity pauses, Apportable calls `-[GameView willResignActive]`
//! (and `didEnterBackground` if it stops), and the game leaves an online world. The game itself keeps a world through
//! its own activities (the server welcome web view): `willResignActive` then calls `-[GameView startTimer]`, a 15 Hz
//! background timer that keeps updating the world and polling the network, and `didBecomeActive` cancels it. While the
//! guard is armed (only around our dialog) the hooks take that path: `willResignActive` → `startTimer`,
//! `didEnterBackground` / the matching `willEnterForeground` are skipped. `didBecomeActive` always runs the game's own
//! code and drops the guard. Unarmed, every hook just calls the original.
//!
//! Also here: the game's idle pause. In a world the game shows its PAUSE menu after about a minute without a touch on
//! the game (`World.pauseIdleTimer`). Touches on the loader's windows (the settings screen, mod dialogs, mod buttons)
//! never reach the game, so a player busy in the settings screen got the PAUSE menu behind it. Java reports those
//! touches ([`touched`]) and the next frame calls the game's own `-[World resetPauseIdleTimer]`, as a touch on the
//! game would. (Our windows taking the window focus does not pause the game: Apportable sends no resign-active for a
//! focus change inside the activity.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use bhmod_core::types::LogLevel;
use jni::objects::JClass;
use jni::JNIEnv;

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
static TOUCHED: AtomicBool = AtomicBool::new(false);
static IDLE_RESET: OnceLock<bool> = OnceLock::new();

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

/// The player touched a loader window (any thread).
pub fn touched() {
    TOUCHED.store(true, Ordering::SeqCst);
}

/// Each frame (loader thread): after a touch on a loader window, restarts the game's idle-pause timer of `world`.
pub fn on_frame(world: Id) {
    if world.is_null() || !TOUCHED.swap(false, Ordering::SeqCst) {
        return;
    }
    let has = *IDLE_RESET.get_or_init(|| {
        let ok = !objc::method("World", "resetPauseIdleTimer", false).is_null();
        if !ok {
            log(
                LogLevel::Warn,
                "-[World resetPauseIdleTimer] not found; the game may pause while a loader window is used",
            );
        }
        ok
    });
    if has {
        objc::send0(world, "resetPauseIdleTimer");
    }
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

/// `Native.touched()`: the player touched a loader window (counts as activity for the game's idle pause).
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_touched(_env: JNIEnv, _c: JClass) {
    touched();
}
