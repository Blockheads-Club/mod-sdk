//! OpenSL ES backend: the fallback for API 24 and 25 (and for devices whose AAudio will not give 48 kHz). Opened with
//! `dlopen` like AAudio, so the loader links neither. Android simple buffer queues with two 20 ms buffers: the queue
//! callback hands a filled capture buffer to Rust (or asks Rust to fill a playback buffer) and enqueues it again.

use std::ffi::{c_char, c_void, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use crate::voice::audio::{Backend, BackendKind, CaptureCallback, PlaybackCallback, Stream, FRAME, SAMPLE_RATE};

extern "C" {
    fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: i32 = 2;

type SlResult = u32;
type SlBool = u32;
/// `SLInterfaceID`: a pointer to a 16-byte GUID.
type Iid = *const c_void;
/// `SLObjectItf`, `SLEngineItf`... : a pointer to a pointer to the interface's function table.
type Itf<T> = *const *const T;

const SUCCESS: SlResult = 0;
const FALSE: SlBool = 0;
const TRUE: SlBool = 1;
const DATALOCATOR_IODEVICE: u32 = 3;
const DATALOCATOR_OUTPUTMIX: u32 = 4;
const DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE: u32 = 0x800007BD;
const IODEVICE_AUDIOINPUT: u32 = 1;
const DEFAULTDEVICEID_AUDIOINPUT: u32 = 0xFFFF_FFFF;
const DATAFORMAT_PCM: u32 = 2;
const PCMSAMPLEFORMAT_16: u32 = 16;
const SPEAKER_FRONT_LEFT: u32 = 1;
const SPEAKER_FRONT_RIGHT: u32 = 2;
const SPEAKER_FRONT_CENTER: u32 = 4;
const BYTEORDER_LITTLEENDIAN: u32 = 2;
const PLAYSTATE_STOPPED: u32 = 1;
const PLAYSTATE_PLAYING: u32 = 3;
const RECORDSTATE_STOPPED: u32 = 1;
const RECORDSTATE_RECORDING: u32 = 3;
const BUFFERS: usize = 2;

#[repr(C)]
struct ObjectItf {
    realize: unsafe extern "C" fn(Itf<ObjectItf>, SlBool) -> SlResult,
    resume: *const c_void,
    get_state: *const c_void,
    get_interface: unsafe extern "C" fn(Itf<ObjectItf>, Iid, *mut c_void) -> SlResult,
    register_callback: *const c_void,
    abort_async_operation: *const c_void,
    destroy: unsafe extern "C" fn(Itf<ObjectItf>),
    // SetPriority, GetPriority, SetLossOfControlInterfaces follow (unused).
}

#[repr(C)]
struct EngineItf {
    create_led_device: *const c_void,
    create_vibra_device: *const c_void,
    create_audio_player: unsafe extern "C" fn(
        Itf<EngineItf>,
        *mut Itf<ObjectItf>,
        *const DataPair,
        *const DataPair,
        u32,
        *const Iid,
        *const SlBool,
    ) -> SlResult,
    create_audio_recorder: unsafe extern "C" fn(
        Itf<EngineItf>,
        *mut Itf<ObjectItf>,
        *const DataPair,
        *const DataPair,
        u32,
        *const Iid,
        *const SlBool,
    ) -> SlResult,
    create_midi_player: *const c_void,
    create_listener: *const c_void,
    create_3d_group: *const c_void,
    create_output_mix:
        unsafe extern "C" fn(Itf<EngineItf>, *mut Itf<ObjectItf>, u32, *const Iid, *const SlBool) -> SlResult,
    // the rest is unused
}

#[repr(C)]
struct PlayItf {
    set_play_state: unsafe extern "C" fn(Itf<PlayItf>, u32) -> SlResult,
}

#[repr(C)]
struct RecordItf {
    set_record_state: unsafe extern "C" fn(Itf<RecordItf>, u32) -> SlResult,
}

type QueueCallback = unsafe extern "C" fn(Itf<QueueItf>, *mut c_void);

#[repr(C)]
struct QueueItf {
    enqueue: unsafe extern "C" fn(Itf<QueueItf>, *const c_void, u32) -> SlResult,
    clear: unsafe extern "C" fn(Itf<QueueItf>) -> SlResult,
    get_state: *const c_void,
    register_callback: unsafe extern "C" fn(Itf<QueueItf>, QueueCallback, *mut c_void) -> SlResult,
}

/// `SLDataSource` / `SLDataSink`.
#[repr(C)]
struct DataPair {
    locator: *const c_void,
    format: *const c_void,
}

#[repr(C)]
struct LocBufferQueue {
    kind: u32,
    buffers: u32,
}

#[repr(C)]
struct LocIoDevice {
    kind: u32,
    device_type: u32,
    device_id: u32,
    device: *const c_void,
}

#[repr(C)]
struct LocOutputMix {
    kind: u32,
    mix: Itf<ObjectItf>,
}

#[repr(C)]
struct FormatPcm {
    kind: u32,
    channels: u32,
    /// milliHertz
    rate: u32,
    bits: u32,
    container: u32,
    mask: u32,
    endian: u32,
}

struct Lib {
    create_engine:
        unsafe extern "C" fn(*mut Itf<ObjectItf>, u32, *const c_void, u32, *const Iid, *const SlBool) -> SlResult,
    iid_engine: Iid,
    iid_play: Iid,
    iid_record: Iid,
    iid_queue: Iid,
}
// SAFETY: a function pointer and constant interface ids of a library that stays loaded.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

static LIB: OnceLock<Option<Lib>> = OnceLock::new();

fn lib() -> Option<&'static Lib> {
    LIB.get_or_init(|| unsafe {
        let h = dlopen(c"libOpenSLES.so".as_ptr(), RTLD_NOW);
        if h.is_null() {
            return None;
        }
        let f = |n: &str| {
            let c = CString::new(n).expect("no NUL");
            dlsym(h, c.as_ptr())
        };
        // `SL_IID_*` are exported variables holding the id pointers.
        let iid = |n: &str| {
            let p = f(n) as *const Iid;
            if p.is_null() {
                std::ptr::null()
            } else {
                *p
            }
        };
        let create = f("slCreateEngine");
        let l = Lib {
            create_engine: if create.is_null() { return None } else { std::mem::transmute_copy(&create) },
            iid_engine: iid("SL_IID_ENGINE"),
            iid_play: iid("SL_IID_PLAY"),
            iid_record: iid("SL_IID_RECORD"),
            iid_queue: iid("SL_IID_ANDROIDSIMPLEBUFFERQUEUE"),
        };
        if [l.iid_engine, l.iid_play, l.iid_record, l.iid_queue].iter().any(|p| p.is_null()) {
            return None;
        }
        Some(l)
    })
    .as_ref()
}

fn check(r: SlResult, what: &str) -> Result<(), String> {
    if r == SUCCESS {
        Ok(())
    } else {
        Err(format!("OpenSL ES: {what} failed ({r:#x})"))
    }
}

/// Destroys an object (null-safe).
unsafe fn destroy(o: Itf<ObjectItf>) {
    if !o.is_null() {
        ((**o).destroy)(o);
    }
}

unsafe fn interface<T>(o: Itf<ObjectItf>, iid: Iid, what: &str) -> Result<Itf<T>, String> {
    let mut i: Itf<T> = std::ptr::null();
    check(((**o).get_interface)(o, iid, &mut i as *mut Itf<T> as *mut c_void), what)?;
    Ok(i)
}

enum Callback {
    Capture(CaptureCallback),
    Playback(PlaybackCallback),
}

/// What the buffer queue callback works on (boxed, address stable).
struct Ctx {
    cb: Callback,
    bufs: [Vec<i16>; BUFFERS],
    next: usize,
}

impl Ctx {
    /// Hands the buffer that just completed to Rust (capture) or refills it (playback), then enqueues it.
    unsafe fn turn(&mut self, q: Itf<QueueItf>) {
        let i = self.next;
        self.next = (i + 1) % BUFFERS;
        let buf = &mut self.bufs[i];
        let _ = catch_unwind(AssertUnwindSafe(|| match &mut self.cb {
            Callback::Capture(f) => f(buf),
            Callback::Playback(f) => f(buf),
        }));
        ((**q).enqueue)(q, buf.as_ptr() as *const c_void, (buf.len() * 2) as u32);
    }
}

unsafe extern "C" fn on_buffer(q: Itf<QueueItf>, ctx: *mut c_void) {
    (*(ctx as *mut Ctx)).turn(q);
}

struct SlStream {
    engine: Itf<ObjectItf>,
    mix: Itf<ObjectItf>,
    obj: Itf<ObjectItf>,
    play: Itf<PlayItf>,
    record: Itf<RecordItf>,
    queue: Itf<QueueItf>,
    ctx: *mut Ctx,
}
// SAFETY: OpenSL ES objects may be stopped and destroyed from any thread.
unsafe impl Send for SlStream {}
impl Stream for SlStream {}

impl Drop for SlStream {
    fn drop(&mut self) {
        // SAFETY: stop, then destroy the player/recorder (Android waits for a running callback), then free its context.
        unsafe {
            if !self.play.is_null() {
                ((**self.play).set_play_state)(self.play, PLAYSTATE_STOPPED);
            }
            if !self.record.is_null() {
                ((**self.record).set_record_state)(self.record, RECORDSTATE_STOPPED);
            }
            if !self.queue.is_null() {
                ((**self.queue).clear)(self.queue);
            }
            destroy(self.obj);
            destroy(self.mix);
            destroy(self.engine);
            drop(Box::from_raw(self.ctx));
        }
    }
}

fn pcm(channels: u32) -> FormatPcm {
    FormatPcm {
        kind: DATAFORMAT_PCM,
        channels,
        rate: SAMPLE_RATE * 1000,
        bits: PCMSAMPLEFORMAT_16,
        container: PCMSAMPLEFORMAT_16,
        mask: if channels == 1 { SPEAKER_FRONT_CENTER } else { SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT },
        endian: BYTEORDER_LITTLEENDIAN,
    }
}

unsafe fn open(l: &Lib, cb: Callback) -> Result<Box<dyn Stream>, String> {
    let input = matches!(cb, Callback::Capture(_));
    let channels: u32 = if input { 1 } else { 2 };
    let len = FRAME * channels as usize;
    let ctx = Box::into_raw(Box::new(Ctx { cb, bufs: [vec![0; len], vec![0; len]], next: 0 }));
    // From here on `s` owns everything created; an early return drops it, which cleans up.
    let mut s = SlStream {
        engine: std::ptr::null(),
        mix: std::ptr::null(),
        obj: std::ptr::null(),
        play: std::ptr::null(),
        record: std::ptr::null(),
        queue: std::ptr::null(),
        ctx,
    };
    check(
        (l.create_engine)(&mut s.engine, 0, std::ptr::null(), 0, std::ptr::null(), std::ptr::null()),
        "slCreateEngine",
    )?;
    check(((**s.engine).realize)(s.engine, FALSE), "engine Realize")?;
    let engine: Itf<EngineItf> = interface(s.engine, l.iid_engine, "engine interface")?;
    let mut queue_loc = LocBufferQueue { kind: DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE, buffers: BUFFERS as u32 };
    let format = pcm(channels);
    let ids = [l.iid_queue];
    let req = [TRUE];
    if input {
        let mic = LocIoDevice {
            kind: DATALOCATOR_IODEVICE,
            device_type: IODEVICE_AUDIOINPUT,
            device_id: DEFAULTDEVICEID_AUDIOINPUT,
            device: std::ptr::null(),
        };
        let src = DataPair { locator: &mic as *const _ as *const c_void, format: std::ptr::null() };
        let sink = DataPair {
            locator: &mut queue_loc as *mut _ as *const c_void,
            format: &format as *const _ as *const c_void,
        };
        check(
            ((**engine).create_audio_recorder)(engine, &mut s.obj, &src, &sink, 1, ids.as_ptr(), req.as_ptr()),
            "CreateAudioRecorder (RECORD_AUDIO granted?)",
        )?;
        check(((**s.obj).realize)(s.obj, FALSE), "recorder Realize")?;
        s.record = interface(s.obj, l.iid_record, "record interface")?;
    } else {
        check(
            ((**engine).create_output_mix)(engine, &mut s.mix, 0, std::ptr::null(), std::ptr::null()),
            "CreateOutputMix",
        )?;
        check(((**s.mix).realize)(s.mix, FALSE), "output mix Realize")?;
        let out = LocOutputMix { kind: DATALOCATOR_OUTPUTMIX, mix: s.mix };
        let src = DataPair {
            locator: &mut queue_loc as *mut _ as *const c_void,
            format: &format as *const _ as *const c_void,
        };
        let sink = DataPair { locator: &out as *const _ as *const c_void, format: std::ptr::null() };
        check(
            ((**engine).create_audio_player)(engine, &mut s.obj, &src, &sink, 1, ids.as_ptr(), req.as_ptr()),
            "CreateAudioPlayer",
        )?;
        check(((**s.obj).realize)(s.obj, FALSE), "player Realize")?;
        s.play = interface(s.obj, l.iid_play, "play interface")?;
    }
    s.queue = interface(s.obj, l.iid_queue, "buffer queue interface")?;
    check(((**s.queue).register_callback)(s.queue, on_buffer, ctx as *mut c_void), "RegisterCallback")?;
    if input {
        // Empty buffers for the recorder to fill.
        for b in &(*ctx).bufs {
            check(((**s.queue).enqueue)(s.queue, b.as_ptr() as *const c_void, (b.len() * 2) as u32), "Enqueue")?;
        }
        check(((**s.record).set_record_state)(s.record, RECORDSTATE_RECORDING), "SetRecordState")?;
    } else {
        // Prime the queue with silence; each completed buffer is refilled from the ring.
        for b in &(*ctx).bufs {
            check(((**s.queue).enqueue)(s.queue, b.as_ptr() as *const c_void, (b.len() * 2) as u32), "Enqueue")?;
        }
        check(((**s.play).set_play_state)(s.play, PLAYSTATE_PLAYING), "SetPlayState")?;
    }
    Ok(Box::new(s))
}

/// The OpenSL ES backend (None when `libOpenSLES.so` is missing).
pub struct OpenSles(&'static Lib);

impl OpenSles {
    /// The backend, when the library loaded.
    pub fn new() -> Option<OpenSles> {
        lib().map(OpenSles)
    }
}

impl Backend for OpenSles {
    fn kind(&self) -> BackendKind {
        BackendKind::OpenSles
    }
    fn open_capture(&self, cb: CaptureCallback) -> Result<Box<dyn Stream>, String> {
        // SAFETY: OpenSL ES used as documented for Android buffer queues.
        unsafe { open(self.0, Callback::Capture(cb)) }
    }
    fn open_playback(&self, cb: PlaybackCallback) -> Result<Box<dyn Stream>, String> {
        // SAFETY: as above.
        unsafe { open(self.0, Callback::Playback(cb)) }
    }
}
