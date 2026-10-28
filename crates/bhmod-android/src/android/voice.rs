//! Voice glue on Android: the audio backend, the unsequenced send path and the Java events (`RECORD_AUDIO` answer,
//! push-to-talk), all handled on the loader thread. Nothing here runs until something asks for voice: no stream is
//! open and no permission is asked before the voice host API (a later milestone) calls [`set_mic`].

use std::ffi::{c_char, c_int};
use std::sync::Mutex;

use bhmod_core::types::LogLevel;
use jni::objects::JClass;
use jni::sys::{jboolean, JNI_FALSE};
use jni::JNIEnv;

use super::{aaudio, enet, log, opensles, INBOX};
use crate::voice::audio::{choose, AudioIo, Backend, BackendKind, Mic, Permission};
use crate::voice::net::{Outbox, SendError, Sent};
use crate::voice::{Voice, VoiceEvent};

/// Unsequenced packets from other threads, sent at the next frame.
static OUTBOX: Outbox = Outbox::new();
/// Events from the Java UI thread.
static EVENTS: Mutex<Vec<VoiceEvent>> = Mutex::new(Vec::new());
/// The voice state (created on first use).
static VOICE: Mutex<Option<Voice>> = Mutex::new(None);

extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
}

/// `ro.build.version.sdk` (0 when unreadable).
pub fn api_level() -> u32 {
    let mut buf = [0 as c_char; 92];
    // SAFETY: PROP_VALUE_MAX is 92 bytes and the value is NUL-terminated.
    let v = unsafe {
        __system_property_get(c"ro.build.version.sdk".as_ptr(), buf.as_mut_ptr());
        std::ffi::CStr::from_ptr(buf.as_ptr())
    };
    v.to_str().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// AAudio on API 26+, OpenSL ES otherwise (or when AAudio is missing). None when neither loads.
fn backend() -> Option<Box<dyn Backend>> {
    match choose(api_level(), aaudio::loaded()) {
        BackendKind::AAudio => aaudio::AAudio::new().map(|b| Box::new(b) as Box<dyn Backend>),
        BackendKind::OpenSles => None,
    }
    .or_else(|| opensles::OpenSles::new().map(|b| Box::new(b) as Box<dyn Backend>))
}

/// Runs `f` on the voice state, creating it on first use (None without any audio backend).
pub fn with_voice<R>(f: impl FnOnce(&mut Voice) -> R) -> Option<R> {
    let mut v = VOICE.lock().unwrap_or_else(|e| e.into_inner());
    if v.is_none() {
        let b = backend()?;
        let granted = super::java_bool("micGranted");
        log(
            LogLevel::Info,
            &format!("voice: {:?} backend, RECORD_AUDIO {}", b.kind(), if granted { "granted" } else { "not granted" }),
        );
        *v = Some(Voice::new(AudioIo::new(b, if granted { Permission::Granted } else { Permission::Unknown })));
    }
    v.as_mut().map(f)
}

/// Turns the microphone on or off (voice host API). The first opt-in shows the system `RECORD_AUDIO` dialog
/// (`Bridge.requestMic`); the microphone opens when the grant arrives.
pub fn set_mic(on: bool) -> Option<Mic> {
    let m = with_voice(|v| v.io().set_mic(on))?;
    match &m {
        Mic::AskPermission => super::java_call("requestMic"),
        Mic::Failed(e) => log(LogLevel::Warn, &format!("voice: microphone failed: {e}")),
        _ => {}
    }
    Some(m)
}

/// True when the game's ENet can take unsequenced packets (`HELLO.features`).
pub fn unsequenced_available() -> bool {
    enet::available()
}

/// The ENet peer when unsequenced sends may go out now: loader thread, in a world, connected.
fn link(in_world: bool) -> Option<enet::Peer> {
    if !INBOX.is_owner() || !in_world {
        return None;
    }
    enet::Peer::of(super::conn()?.matc)
}

/// Sends a whole 0xBC packet (`bc 16 ...`, at most 1364 bytes) unsequenced on channel 0. Off the loader thread or
/// outside a world it waits for the next frame.
pub fn send_unsequenced(packet: &[u8]) -> Result<Sent, SendError> {
    let peer = link(super::in_world());
    OUTBOX.send(packet, peer.as_ref().map(|p| p as &dyn crate::voice::net::Enet))
}

/// Every frame (loader thread): Java events, then the queued unsequenced packets (dropped outside a world).
pub fn on_frame(in_world: bool) {
    let events: Vec<VoiceEvent> = std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()));
    for ev in events {
        if let Some(Some(m)) = with_voice(|v| v.on_event(ev)) {
            log(LogLevel::Info, &format!("voice: microphone {m:?} after the permission answer"));
        }
    }
    if !OUTBOX.is_empty() {
        let peer = link(in_world);
        OUTBOX.flush(peer.as_ref().map(|p| p as &dyn crate::voice::net::Enet));
    }
}

fn push(ev: VoiceEvent) {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
}

/// `Native.micPermission(granted)`: the answer of the `RECORD_AUDIO` dialog.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_micPermission(
    _env: JNIEnv,
    _c: JClass,
    granted: jboolean,
) {
    push(VoiceEvent::Permission(granted != JNI_FALSE));
}

/// `Native.ptt(down)`: the push-to-talk control (seam for the later HUD button).
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_ptt(_env: JNIEnv, _c: JClass, down: jboolean) {
    push(VoiceEvent::Ptt(down != JNI_FALSE));
}
