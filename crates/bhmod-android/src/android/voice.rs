//! Voice glue on Android: the audio backend, the `RECORD_AUDIO` prompt and the Java
//! events (permission answer, push-to-talk seam). The loader ([`crate::loader`]) owns the pipeline; this module only
//! gives it what [`crate::loader::Platform`] asks for. Nothing opens before a mod turns the microphone on.

use std::ffi::{c_char, c_int};

use jni::objects::JClass;
use jni::sys::{jboolean, JNI_FALSE};
use jni::JNIEnv;

use super::push_ui;
use crate::loader::UiEvent;
use crate::voice::audio::{choose, AudioIo, Backend, BackendKind, Permission};

use super::{aaudio, opensles};

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
    // Without the lifecycle guard the system dialog would end the connection: the player allows the microphone
    // from the Mods screen in the main menu instead.
    let io = AudioIo::new(b, if granted { Permission::Granted } else { Permission::Unknown });
    Some(io.asking(super::lifecycle::available()))
}

/// Shows the system `RECORD_AUDIO` dialog (first opt-in); the answer comes back through `Native.micPermission`.
pub fn ask_mic() {
    super::java_call("requestMic");
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

/// `Native.micGuardAvailable()`: the lifecycle guard is hooked (in-world requests allowed).
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_micGuardAvailable(
    _env: JNIEnv,
    _c: JClass,
) -> jboolean {
    jboolean::from(super::lifecycle::available())
}

/// `Native.micGuard(on)`: arms or drops the lifecycle guard around our dialog.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_micGuard(_env: JNIEnv, _c: JClass, on: jboolean) {
    super::lifecycle::arm(on != JNI_FALSE);
}

/// `Native.ptt(down)`: the host's push-to-talk seam (talks in the default mode).
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_ptt(_env: JNIEnv, _c: JClass, down: jboolean) {
    push_ui(UiEvent::Ptt(down != JNI_FALSE));
}
