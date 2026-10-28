//! Microphone capture and voice playback, independent of the Android audio API.
//!
//! A [`Backend`] opens streams that call plain Rust callbacks on the audio thread: capture delivers 48 kHz mono i16
//! samples in whatever buffer size the device uses, playback asks for 48 kHz interleaved stereo i16. On a phone the
//! backend is AAudio (API 26+, loaded with `dlopen`, never linked) or OpenSL ES (API 24/25) (`android::aaudio`,
//! `android::opensles`); tests use a fake.
//!
//! [`capture_callback`] cuts the samples into 20 ms frames ([`FRAME`]) for a bounded [`FrameQueue`] the encoder
//! drains; [`playback_callback`] reads the mixed stereo stream from a bounded [`PlaybackRing`] the mixer fills.
//! [`AudioIo`] opens the microphone only on demand and only with the `RECORD_AUDIO` runtime permission.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Sample rate of both directions.
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples in one 20 ms capture frame (mono).
pub const FRAME: usize = SAMPLE_RATE as usize / 50;
/// Capture frames kept until the encoder takes them (200 ms).
pub const CAPTURE_FRAMES: usize = 10;
/// Playback samples kept ahead of the device (200 ms of stereo).
pub const PLAYBACK_SAMPLES: usize = SAMPLE_RATE as usize / 5 * 2;

/// Called on the audio thread with captured mono samples.
pub type CaptureCallback = Box<dyn FnMut(&[i16]) + Send>;
/// Called on the audio thread to fill interleaved stereo samples.
pub type PlaybackCallback = Box<dyn FnMut(&mut [i16]) + Send>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Collects samples into [`FRAME`]-long frames.
#[derive(Debug, Default)]
pub struct Chunker {
    buf: Vec<i16>,
}

impl Chunker {
    /// Adds `samples`; calls `out` with every frame that is complete.
    pub fn push(&mut self, mut samples: &[i16], mut out: impl FnMut(Vec<i16>)) {
        while !samples.is_empty() {
            let take = (FRAME - self.buf.len()).min(samples.len());
            self.buf.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.buf.len() == FRAME {
                out(std::mem::replace(&mut self.buf, Vec::with_capacity(FRAME)));
            }
        }
    }

    /// Samples of the unfinished frame.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

/// Captured frames for the encoder. Bounded: a full queue drops its oldest frame.
#[derive(Debug, Default)]
pub struct FrameQueue {
    q: Mutex<VecDeque<Vec<i16>>>,
    dropped: AtomicU64,
}

impl FrameQueue {
    /// Adds a frame (audio thread).
    pub fn push(&self, frame: Vec<i16>) {
        let mut q = lock(&self.q);
        if q.len() == CAPTURE_FRAMES {
            q.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        q.push_back(frame);
    }

    /// The oldest frame.
    pub fn pop(&self) -> Option<Vec<i16>> {
        lock(&self.q).pop_front()
    }

    /// Forgets every frame (the microphone closed).
    pub fn clear(&self) {
        lock(&self.q).clear();
    }

    /// Frames waiting.
    pub fn len(&self) -> usize {
        lock(&self.q).len()
    }

    /// True when no frame waits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Frames lost to overflow.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The capture callback for a backend: 20 ms frames into `queue`.
pub fn capture_callback(queue: Arc<FrameQueue>) -> CaptureCallback {
    let mut chunker = Chunker::default();
    Box::new(move |samples| chunker.push(samples, |f| queue.push(f)))
}

/// The mixed stereo stream waiting for the device. Bounded to [`PLAYBACK_SAMPLES`]: when the mixer runs ahead the
/// oldest samples go (latency stays low); when it falls behind the device gets silence.
#[derive(Debug, Default)]
pub struct PlaybackRing {
    q: Mutex<VecDeque<i16>>,
    underruns: AtomicU64,
}

impl PlaybackRing {
    /// Adds interleaved stereo samples (an odd trailing sample is ignored).
    pub fn write(&self, stereo: &[i16]) {
        let stereo = &stereo[..stereo.len() & !1];
        let mut q = lock(&self.q);
        q.extend(stereo.iter().copied());
        let over = q.len().saturating_sub(PLAYBACK_SAMPLES);
        q.drain(..over);
    }

    /// Fills `out` (audio thread); missing samples are silence.
    pub fn fill(&self, out: &mut [i16]) {
        let mut q = lock(&self.q);
        let n = q.len().min(out.len());
        for (o, s) in out.iter_mut().zip(q.drain(..n)) {
            *o = s;
        }
        if n < out.len() {
            out[n..].fill(0);
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Samples waiting.
    pub fn len(&self) -> usize {
        lock(&self.q).len()
    }

    /// True when nothing waits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Device callbacks that got less than they asked for.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}

/// The playback callback for a backend: reads `ring`.
pub fn playback_callback(ring: Arc<PlaybackRing>) -> PlaybackCallback {
    Box::new(move |out| ring.fill(out))
}

/// The audio API behind a [`Backend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// AAudio (`libaaudio.so`, Android 8.0 / API 26+).
    AAudio,
    /// OpenSL ES (`libOpenSLES.so`), for API 24 and 25.
    OpenSles,
}

/// AAudio from API 26 when `libaaudio.so` loaded, OpenSL ES otherwise.
pub fn choose(api_level: u32, aaudio_loaded: bool) -> BackendKind {
    if api_level >= 26 && aaudio_loaded {
        BackendKind::AAudio
    } else {
        BackendKind::OpenSles
    }
}

/// An open, running stream; dropping it stops and closes it (no callback runs afterwards).
pub trait Stream: Send {}

/// Opens 48 kHz streams.
pub trait Backend: Send {
    /// Which API this is.
    fn kind(&self) -> BackendKind;
    /// Starts mono capture from the microphone.
    fn open_capture(&self, cb: CaptureCallback) -> Result<Box<dyn Stream>, String>;
    /// Starts stereo playback.
    fn open_playback(&self, cb: PlaybackCallback) -> Result<Box<dyn Stream>, String>;
}

/// The `RECORD_AUDIO` runtime permission as the loader knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Not asked yet in this process.
    Unknown,
    /// The system dialog is up.
    Requested,
    /// Granted.
    Granted,
    /// Refused.
    Denied,
}

/// The answer of [`AudioIo::set_mic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mic {
    /// The microphone is capturing.
    Open,
    /// The microphone is off.
    Closed,
    /// The permission must be asked now (first opt-in): the glue shows the system dialog; the microphone opens
    /// when [`AudioIo::on_permission`] brings a grant.
    AskPermission,
    /// The system dialog is already up.
    Waiting,
    /// The player refused the permission.
    Denied,
    /// The backend could not open the microphone.
    Failed(String),
}

/// Capture and playback for the voice host.
pub struct AudioIo {
    backend: Box<dyn Backend>,
    permission: Permission,
    want_mic: bool,
    capture: Option<Box<dyn Stream>>,
    playback: Option<Box<dyn Stream>>,
    frames: Arc<FrameQueue>,
    output: Arc<PlaybackRing>,
}

impl AudioIo {
    /// Nothing open; `permission` is what Java reported at start (`Granted` when the player allowed it before).
    pub fn new(backend: Box<dyn Backend>, permission: Permission) -> AudioIo {
        AudioIo {
            backend,
            permission,
            want_mic: false,
            capture: None,
            playback: None,
            frames: Arc::default(),
            output: Arc::default(),
        }
    }

    /// The backend in use.
    pub fn kind(&self) -> BackendKind {
        self.backend.kind()
    }

    /// Captured 20 ms frames.
    pub fn frames(&self) -> Arc<FrameQueue> {
        Arc::clone(&self.frames)
    }

    /// Where the mixer writes the stereo output.
    pub fn output(&self) -> Arc<PlaybackRing> {
        Arc::clone(&self.output)
    }

    /// The permission state.
    pub fn permission(&self) -> Permission {
        self.permission
    }

    /// True while capturing.
    pub fn mic_open(&self) -> bool {
        self.capture.is_some()
    }

    /// True while playing.
    pub fn playing(&self) -> bool {
        self.playback.is_some()
    }

    /// Turns the microphone on or off. It never opens without the permission.
    pub fn set_mic(&mut self, on: bool) -> Mic {
        self.want_mic = on;
        if !on {
            self.capture = None;
            self.frames.clear();
            return Mic::Closed;
        }
        if self.capture.is_some() {
            return Mic::Open;
        }
        match self.permission {
            Permission::Unknown => {
                self.permission = Permission::Requested;
                Mic::AskPermission
            }
            Permission::Requested => Mic::Waiting,
            Permission::Denied => Mic::Denied,
            Permission::Granted => match self.backend.open_capture(capture_callback(self.frames())) {
                Ok(s) => {
                    self.capture = Some(s);
                    Mic::Open
                }
                Err(e) => Mic::Failed(e),
            },
        }
    }

    /// The answer of the system dialog. Opens the microphone when it is still wanted.
    pub fn on_permission(&mut self, granted: bool) -> Mic {
        self.permission = if granted { Permission::Granted } else { Permission::Denied };
        let want = self.want_mic;
        self.set_mic(want)
    }

    /// Starts or stops playback.
    pub fn set_playback(&mut self, on: bool) -> Result<(), String> {
        if !on {
            self.playback = None;
        } else if self.playback.is_none() {
            self.playback = Some(self.backend.open_playback(playback_callback(self.output()))?);
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A backend that records what was opened and lets tests drive the callbacks.
    use super::*;

    /// Shared state of a [`FakeBackend`].
    #[derive(Default)]
    pub struct State {
        pub capture: Option<CaptureCallback>,
        pub playback: Option<PlaybackCallback>,
        pub opened: usize,
        pub closed: usize,
        pub fail: bool,
    }

    /// The fake backend.
    #[derive(Clone, Default)]
    pub struct FakeBackend(pub Arc<Mutex<State>>);

    struct FakeStream(Arc<Mutex<State>>, bool);
    impl Stream for FakeStream {}
    impl Drop for FakeStream {
        fn drop(&mut self) {
            let mut s = lock(&self.0);
            s.closed += 1;
            if self.1 {
                s.capture = None;
            } else {
                s.playback = None;
            }
        }
    }

    impl Backend for FakeBackend {
        fn kind(&self) -> BackendKind {
            BackendKind::OpenSles
        }
        fn open_capture(&self, cb: CaptureCallback) -> Result<Box<dyn Stream>, String> {
            let mut s = lock(&self.0);
            if s.fail {
                return Err("no mic".into());
            }
            s.opened += 1;
            s.capture = Some(cb);
            Ok(Box::new(FakeStream(Arc::clone(&self.0), true)))
        }
        fn open_playback(&self, cb: PlaybackCallback) -> Result<Box<dyn Stream>, String> {
            let mut s = lock(&self.0);
            if s.fail {
                return Err("no output".into());
            }
            s.opened += 1;
            s.playback = Some(cb);
            Ok(Box::new(FakeStream(Arc::clone(&self.0), false)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeBackend;
    use super::*;

    #[test]
    fn chunker_cuts_20ms_frames_across_buffer_sizes() {
        let mut c = Chunker::default();
        let mut frames = Vec::new();
        let samples: Vec<i16> = (0..(FRAME as i32 * 2 + 100)).map(|i| i as i16).collect();
        for part in samples.chunks(333) {
            c.push(part, |f| frames.push(f));
        }
        assert_eq!(FRAME, 960);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| f.len() == FRAME));
        assert_eq!(frames[1][0], FRAME as i16);
        assert_eq!(c.pending(), 100);
    }

    #[test]
    fn frame_queue_is_bounded() {
        let q = FrameQueue::default();
        for i in 0..CAPTURE_FRAMES + 2 {
            q.push(vec![i as i16]);
        }
        assert_eq!(q.len(), CAPTURE_FRAMES);
        assert_eq!(q.dropped(), 2);
        assert_eq!(q.pop(), Some(vec![2]));
        q.clear();
        assert!(q.is_empty());
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn playback_ring_bounds_latency_and_pads_with_silence() {
        let r = PlaybackRing::default();
        r.write(&[1, 2, 3]);
        assert_eq!(r.len(), 2);
        let mut out = [9i16; 4];
        r.fill(&mut out);
        assert_eq!(out, [1, 2, 0, 0]);
        assert_eq!(r.underruns(), 1);
        r.write(&vec![5; PLAYBACK_SAMPLES + 10]);
        assert_eq!(r.len(), PLAYBACK_SAMPLES);
        let mut out = [0i16; 2];
        r.fill(&mut out);
        assert_eq!(out, [5, 5]);
        assert_eq!(r.underruns(), 1);
        assert!(!r.is_empty());
    }

    #[test]
    fn backend_choice_follows_the_api_level() {
        assert_eq!(choose(26, true), BackendKind::AAudio);
        assert_eq!(choose(34, false), BackendKind::OpenSles);
        assert_eq!(choose(24, true), BackendKind::OpenSles);
    }

    #[test]
    fn the_mic_asks_once_and_opens_on_grant() {
        let fake = FakeBackend::default();
        let mut io = AudioIo::new(Box::new(fake.clone()), Permission::Unknown);
        assert_eq!(io.kind(), BackendKind::OpenSles);
        assert_eq!(io.set_mic(true), Mic::AskPermission);
        assert_eq!(io.permission(), Permission::Requested);
        assert_eq!(io.set_mic(true), Mic::Waiting);
        assert!(!io.mic_open());
        assert_eq!(io.on_permission(true), Mic::Open);
        assert!(io.mic_open());
        assert_eq!(io.set_mic(true), Mic::Open);
        assert_eq!(fake.0.lock().unwrap().opened, 1);
        // the device delivers samples: 20 ms frames reach the queue
        let mut cb = fake.0.lock().unwrap().capture.take().unwrap();
        cb(&[7; FRAME + 5]);
        assert_eq!(io.frames().pop().unwrap().len(), FRAME);
        cb(&[7; FRAME]);
        assert_eq!(io.frames().len(), 1);
        assert_eq!(io.set_mic(false), Mic::Closed);
        assert!(io.frames().is_empty());
        assert_eq!(fake.0.lock().unwrap().closed, 1);
    }

    #[test]
    fn a_grant_without_a_wish_keeps_the_mic_closed_and_denial_sticks() {
        let fake = FakeBackend::default();
        let mut io = AudioIo::new(Box::new(fake.clone()), Permission::Unknown);
        assert_eq!(io.on_permission(true), Mic::Closed);
        assert!(!io.mic_open());
        let mut io = AudioIo::new(Box::new(fake), Permission::Unknown);
        io.set_mic(true);
        assert_eq!(io.on_permission(false), Mic::Denied);
        assert_eq!(io.set_mic(true), Mic::Denied);
    }

    #[test]
    fn backend_failures_are_reported() {
        let fake = FakeBackend::default();
        fake.0.lock().unwrap().fail = true;
        let mut io = AudioIo::new(Box::new(fake), Permission::Granted);
        assert_eq!(io.set_mic(true), Mic::Failed("no mic".into()));
        assert_eq!(io.set_playback(true), Err("no output".into()));
        assert!(!io.playing());
    }

    #[test]
    fn playback_reads_the_ring() {
        let fake = FakeBackend::default();
        let mut io = AudioIo::new(Box::new(fake.clone()), Permission::Denied);
        io.set_playback(true).unwrap();
        io.set_playback(true).unwrap();
        assert!(io.playing());
        assert_eq!(fake.0.lock().unwrap().opened, 1);
        io.output().write(&[3, 4]);
        let mut cb = fake.0.lock().unwrap().playback.take().unwrap();
        let mut out = [0i16; 2];
        cb(&mut out);
        assert_eq!(out, [3, 4]);
        io.set_playback(false).unwrap();
        assert!(!io.playing());
    }

    #[test]
    fn a_poisoned_lock_still_works() {
        let q = Arc::new(FrameQueue::default());
        let other = Arc::clone(&q);
        let _ = std::thread::spawn(move || {
            let _g = other.q.lock().unwrap();
            panic!("poison");
        })
        .join();
        q.push(vec![1]);
        assert_eq!(q.len(), 1);
    }
}
