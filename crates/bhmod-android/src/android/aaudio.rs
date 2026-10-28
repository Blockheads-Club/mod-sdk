//! AAudio backend (Android 8.0 / API 26+). `libaaudio.so` is opened with `dlopen` at run time and never linked: the
//! loader's minimum is API 24, where the library does not exist and a hard link would stop the loader from loading.
//!
//! Streams are 48 kHz i16, shared mode, low latency, callback driven. A device that will not give exactly 48 kHz with
//! the asked channel count is refused, and the caller falls back to OpenSL ES.

use std::ffi::{c_char, c_void, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use crate::voice::audio::{Backend, BackendKind, CaptureCallback, PlaybackCallback, Stream, SAMPLE_RATE};

extern "C" {
    fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: i32 = 2;

type Builder = *mut c_void;
type AStream = *mut c_void;
type DataCallback = unsafe extern "C" fn(AStream, *mut c_void, *mut c_void, i32) -> i32;

const DIRECTION_OUTPUT: i32 = 0;
const DIRECTION_INPUT: i32 = 1;
const FORMAT_PCM_I16: i32 = 1;
const SHARING_SHARED: i32 = 1;
const PERFORMANCE_LOW_LATENCY: i32 = 12;
const CALLBACK_CONTINUE: i32 = 0;
const INPUT_PRESET_VOICE_COMMUNICATION: i32 = 7;
const OK: i32 = 0;

struct Api {
    create_builder: unsafe extern "C" fn(*mut Builder) -> i32,
    set_direction: unsafe extern "C" fn(Builder, i32),
    set_sample_rate: unsafe extern "C" fn(Builder, i32),
    set_channel_count: unsafe extern "C" fn(Builder, i32),
    set_format: unsafe extern "C" fn(Builder, i32),
    set_sharing_mode: unsafe extern "C" fn(Builder, i32),
    set_performance_mode: unsafe extern "C" fn(Builder, i32),
    set_data_callback: unsafe extern "C" fn(Builder, DataCallback, *mut c_void),
    /// `AAudioStreamBuilder_setInputPreset` (API 28), absent before.
    set_input_preset: Option<unsafe extern "C" fn(Builder, i32)>,
    open: unsafe extern "C" fn(Builder, *mut AStream) -> i32,
    delete: unsafe extern "C" fn(Builder) -> i32,
    start: unsafe extern "C" fn(AStream) -> i32,
    stop: unsafe extern "C" fn(AStream) -> i32,
    close: unsafe extern "C" fn(AStream) -> i32,
    sample_rate: unsafe extern "C" fn(AStream) -> i32,
    channel_count: unsafe extern "C" fn(AStream) -> i32,
    format: unsafe extern "C" fn(AStream) -> i32,
}
// SAFETY: function pointers into a library that stays loaded.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

static API: OnceLock<Option<Api>> = OnceLock::new();

fn api() -> Option<&'static Api> {
    API.get_or_init(|| unsafe {
        let h = dlopen(c"libaaudio.so".as_ptr(), RTLD_NOW);
        if h.is_null() {
            return None;
        }
        let f = |n: &str| {
            let c = CString::new(n).expect("no NUL");
            dlsym(h, c.as_ptr())
        };
        macro_rules! req {
            ($name:literal) => {{
                let p = f($name);
                if p.is_null() {
                    return None;
                }
                // SAFETY: the AAudio function of that name has the field's C signature.
                std::mem::transmute_copy(&p)
            }};
        }
        let preset = f("AAudioStreamBuilder_setInputPreset");
        Some(Api {
            create_builder: req!("AAudio_createStreamBuilder"),
            set_direction: req!("AAudioStreamBuilder_setDirection"),
            set_sample_rate: req!("AAudioStreamBuilder_setSampleRate"),
            set_channel_count: req!("AAudioStreamBuilder_setChannelCount"),
            set_format: req!("AAudioStreamBuilder_setFormat"),
            set_sharing_mode: req!("AAudioStreamBuilder_setSharingMode"),
            set_performance_mode: req!("AAudioStreamBuilder_setPerformanceMode"),
            set_data_callback: req!("AAudioStreamBuilder_setDataCallback"),
            set_input_preset: (!preset.is_null())
                .then(|| std::mem::transmute::<*mut c_void, unsafe extern "C" fn(Builder, i32)>(preset)),
            open: req!("AAudioStreamBuilder_openStream"),
            delete: req!("AAudioStreamBuilder_delete"),
            start: req!("AAudioStream_requestStart"),
            stop: req!("AAudioStream_requestStop"),
            close: req!("AAudioStream_close"),
            sample_rate: req!("AAudioStream_getSampleRate"),
            channel_count: req!("AAudioStream_getChannelCount"),
            format: req!("AAudioStream_getFormat"),
        })
    })
    .as_ref()
}

/// True when `libaaudio.so` loaded with every function the backend needs.
pub fn loaded() -> bool {
    api().is_some()
}

enum Callback {
    Capture(CaptureCallback),
    Playback(PlaybackCallback),
}

unsafe extern "C" fn on_data(_s: AStream, user: *mut c_void, data: *mut c_void, frames: i32) -> i32 {
    let cb = &mut *(user as *mut Callback);
    let _ = catch_unwind(AssertUnwindSafe(|| match cb {
        Callback::Capture(f) => f(std::slice::from_raw_parts(data as *const i16, frames.max(0) as usize)),
        Callback::Playback(f) => f(std::slice::from_raw_parts_mut(data as *mut i16, frames.max(0) as usize * 2)),
    }));
    CALLBACK_CONTINUE
}

struct AaStream {
    api: &'static Api,
    stream: AStream,
    user: *mut Callback,
}
// SAFETY: AAudio stream handles may be stopped and closed from any thread.
unsafe impl Send for AaStream {}
impl Stream for AaStream {}

impl Drop for AaStream {
    fn drop(&mut self) {
        // SAFETY: close() returns after the last data callback, so the callback box is free to go afterwards.
        unsafe {
            (self.api.stop)(self.stream);
            (self.api.close)(self.stream);
            drop(Box::from_raw(self.user));
        }
    }
}

fn open(api: &'static Api, input: bool, cb: Callback) -> Result<Box<dyn Stream>, String> {
    let channels = if input { 1 } else { 2 };
    let user = Box::into_raw(Box::new(cb));
    // SAFETY: AAudio C API used as documented; `user` lives until the stream is closed.
    unsafe {
        let mut b: Builder = std::ptr::null_mut();
        if (api.create_builder)(&mut b) != OK {
            drop(Box::from_raw(user));
            return Err("AAudio: no stream builder".into());
        }
        (api.set_direction)(b, if input { DIRECTION_INPUT } else { DIRECTION_OUTPUT });
        (api.set_sample_rate)(b, SAMPLE_RATE as i32);
        (api.set_channel_count)(b, channels);
        (api.set_format)(b, FORMAT_PCM_I16);
        (api.set_sharing_mode)(b, SHARING_SHARED);
        (api.set_performance_mode)(b, PERFORMANCE_LOW_LATENCY);
        if let (true, Some(preset)) = (input, api.set_input_preset) {
            preset(b, INPUT_PRESET_VOICE_COMMUNICATION);
        }
        (api.set_data_callback)(b, on_data, user as *mut c_void);
        let mut s: AStream = std::ptr::null_mut();
        let r = (api.open)(b, &mut s);
        (api.delete)(b);
        if r != OK || s.is_null() {
            drop(Box::from_raw(user));
            return Err(format!("AAudio: open failed ({r})"));
        }
        let stream = AaStream { api, stream: s, user };
        let (rate, ch, fmt) = ((api.sample_rate)(s), (api.channel_count)(s), (api.format)(s));
        if rate != SAMPLE_RATE as i32 || ch != channels || fmt != FORMAT_PCM_I16 {
            return Err(format!("AAudio: device gave {rate} Hz x{ch} format {fmt}"));
        }
        let r = (api.start)(s);
        if r != OK {
            return Err(format!("AAudio: start failed ({r})"));
        }
        Ok(Box::new(stream))
    }
}

/// The AAudio backend (None when the library is missing).
pub struct AAudio(&'static Api);

impl AAudio {
    /// The backend, when `libaaudio.so` loaded.
    pub fn new() -> Option<AAudio> {
        api().map(AAudio)
    }
}

impl Backend for AAudio {
    fn kind(&self) -> BackendKind {
        BackendKind::AAudio
    }
    fn open_capture(&self, cb: CaptureCallback) -> Result<Box<dyn Stream>, String> {
        open(self.0, true, Callback::Capture(cb))
    }
    fn open_playback(&self, cb: PlaybackCallback) -> Result<Box<dyn Stream>, String> {
        open(self.0, false, Callback::Playback(cb))
    }
}
