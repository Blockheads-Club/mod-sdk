//! Voice glue on Android: the audio backend, the `RECORD_AUDIO` prompt, the unsequenced send path and the Java
//! events (permission answer, push-to-talk seam). The loader ([`crate::loader`]) owns the pipeline; this module only
//! gives it what [`crate::loader::Platform`] asks for. Nothing opens before a mod turns the microphone on.

use std::ffi::{c_char, c_int};

use jni::objects::JClass;
use jni::sys::{jboolean, JNI_FALSE};
use jni::JNIEnv;

use super::{enet, push_ui, INBOX};
use crate::loader::UiEvent;
use crate::voice::audio::{choose, AudioIo, Backend, BackendKind, Permission};
use crate::voice::net::{Outbox, Sent};

use super::{aaudio, opensles};

/// Unsequenced packets from other threads, sent at the next frame.
static OUTBOX: Outbox = Outbox::new();

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

/// Fresh audio streams for a voice connection (`Platform::audio`): the backend and whether the player allowed the
/// microphone before (then no dialog is needed).
pub fn audio() -> Option<AudioIo> {
    let b = backend()?;
    let granted = super::java_bool("micGranted");
    super::log(
        bhmod_core::types::LogLevel::Info,
        &format!("voice: {:?} backend, RECORD_AUDIO {}", b.kind(), if granted { "granted" } else { "not granted" }),
    );
    Some(AudioIo::new(b, if granted { Permission::Granted } else { Permission::Unknown }))
}

/// Shows the system `RECORD_AUDIO` dialog (first opt-in); the answer comes back through `Native.micPermission`.
pub fn ask_mic() {
    super::java_call("requestMic");
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
/// outside a world it waits for the next frame. False when it was refused or can never be sent.
pub fn send_unsequenced(packet: &[u8]) -> bool {
    let peer = link(super::in_world());
    matches!(
        OUTBOX.send(packet, peer.as_ref().map(|p| p as &dyn crate::voice::net::Enet)),
        Ok(Sent::Now | Sent::Queued)
    )
}

/// Every frame (loader thread): the queued unsequenced packets go out (dropped outside a world).
pub fn on_frame(in_world: bool) {
    if !OUTBOX.is_empty() {
        let peer = link(in_world);
        OUTBOX.flush(peer.as_ref().map(|p| p as &dyn crate::voice::net::Enet));
    }
}

/// `Native.micPermission(granted)`: the answer of the `RECORD_AUDIO` dialog.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_micPermission(
    _env: JNIEnv,
    _c: JClass,
    granted: jboolean,
) {
    push_ui(UiEvent::MicPermission(granted != JNI_FALSE));
}

/// `Native.ptt(down)`: the host's push-to-talk seam (talks in the default mode).
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_ptt(_env: JNIEnv, _c: JClass, down: jboolean) {
    push_ui(UiEvent::Ptt(down != JNI_FALSE));
}
