//! The voice pipeline of one connection, run once per `drawFrame` on the loader thread (the thread that also
//! services the game's ENet host, so unsequenced sends may leave from here):
//!
//! microphone frames ([`AudioIo::frames`]) → the [`Driver`] (activation, Opus, sealing: `bhmod_voice`'s
//! `VoiceClient`) → [`Net`] (reliable 0xBC, unsequenced 0xBC, or the server's voice UDP port); and the other way,
//! received frames went to the driver through the session, and here the driver mixes them into stereo that keeps
//! the [`super::audio::PlaybackRing`] [`TARGET`] samples ahead of the device.
//!
//! The microphone opens only while the driver wants it (a mod called `voice.set-mic(true)`), the first time after
//! the `RECORD_AUDIO` dialog; the answer and every change reach the driver as a [`MicState`] (`voice:mic`). The
//! mic-on indicator is the host's: [`Notice::Indicator`] whenever the driver starts or stops transmitting.

pub use bhmod_core::voice::MicState;
use bhmod_core::voice::Mode;

use super::audio::{AudioIo, Mic, FRAME};

/// One mixed 20 ms frame: interleaved stereo samples.
pub const BLOCK: usize = FRAME * 2;
/// Stereo samples kept queued for the device: three frames, 60 ms (the game thread runs at up to 60 Hz and sometimes
/// stalls).
pub const TARGET: usize = BLOCK * 3;

/// A packet the driver wants sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// A whole 0xBC packet, reliable (key exchange, control).
    Reliable(Vec<u8>),
    /// A whole 0xBC packet, unsequenced (a voice frame in-band).
    Unsequenced(Vec<u8>),
    /// A datagram for the server's voice UDP port.
    Udp(Vec<u8>),
}

/// The voice client the pipeline drives (`bhmod_voice::client::VoiceClient` on a phone, a fake in tests).
pub trait Driver {
    /// A captured 20 ms frame, mono, `-1.0..1.0`.
    fn on_mic_frame(&mut self, pcm: &[f32; FRAME]);
    /// What to send now.
    fn poll_send(&mut self, now: u64) -> Vec<Outgoing>;
    /// Mixes what was heard into interleaved stereo (`out` is zeroed first).
    fn mix(&mut self, out: &mut [f32]);
    /// True while a mod wants the microphone on.
    fn mic_wanted(&self) -> bool;
    /// The microphone changed.
    fn mic_state(&mut self, state: MicState);
    /// The mode being transmitted right now (`None`: not transmitting).
    fn transmitting(&self) -> Option<Mode>;
}

/// Where packets go.
pub trait Net {
    /// Sends a packet reliably (the game's own send path).
    fn reliable(&mut self, packet: &[u8]);
    /// Sends a packet unsequenced on channel 0; false when it could not leave.
    fn unsequenced(&mut self, packet: &[u8]) -> bool;
    /// Sends a datagram to the server's voice port; false without one.
    fn udp(&mut self, datagram: &[u8]) -> bool;
}

/// Something the glue shows or does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Show the system `RECORD_AUDIO` dialog.
    AskPermission,
    /// The microphone changed (also told to the driver).
    Mic(MicState),
    /// The host-owned mic-on indicator: `Some(mode)` while transmitting.
    Indicator(Option<Mode>),
    /// An audio stream failed (logged; voice keeps going without it).
    Failed(String),
}

/// i16 → `-1.0..1.0`.
pub fn to_f32(s: i16) -> f32 {
    f32::from(s) / 32768.0
}

/// `-1.0..1.0` → i16, clamped.
pub fn to_i16(f: f32) -> i16 {
    (f * 32767.0).clamp(-32768.0, 32767.0) as i16
}

/// The pipeline (owns the audio streams).
pub struct Pipeline {
    io: AudioIo,
    mic_on: bool,
    indicator: Option<Mode>,
    playback_failed: bool,
    mix: Vec<f32>,
    pcm: Vec<i16>,
}

impl Pipeline {
    /// A pipeline over these streams (nothing opens before the first [`Pipeline::frame`]).
    pub fn new(io: AudioIo) -> Pipeline {
        Pipeline { io, mic_on: false, indicator: None, playback_failed: false, mix: Vec::new(), pcm: Vec::new() }
    }

    /// The audio streams.
    pub fn io(&mut self) -> &mut AudioIo {
        &mut self.io
    }

    fn mic(&mut self, m: Mic, d: &mut dyn Driver, out: &mut Vec<Notice>) {
        let state = match m {
            Mic::Open => MicState::On,
            Mic::Closed => MicState::Off,
            Mic::Denied => MicState::Denied,
            Mic::AskPermission => {
                out.push(Notice::AskPermission);
                return;
            }
            Mic::Waiting => return,
            Mic::Failed(e) => {
                out.push(Notice::Failed(format!("microphone: {e}")));
                MicState::Off
            }
        };
        d.mic_state(state);
        out.push(Notice::Mic(state));
    }

    /// The answer of the `RECORD_AUDIO` dialog.
    pub fn on_permission(&mut self, granted: bool, d: &mut dyn Driver) -> Vec<Notice> {
        let mut out = Vec::new();
        let m = self.io.on_permission(granted);
        self.mic(m, d, &mut out);
        out
    }

    /// One frame: microphone wish, captured frames, sends, mixing, indicator.
    pub fn frame(&mut self, now: u64, d: &mut dyn Driver, net: &mut dyn Net) -> Vec<Notice> {
        let mut out = Vec::new();
        let want = d.mic_wanted();
        if want != self.mic_on {
            self.mic_on = want;
            let m = self.io.set_mic(want);
            self.mic(m, d, &mut out);
        }
        let frames = self.io.frames();
        let mut buf = [0.0f32; FRAME];
        while let Some(f) = frames.pop() {
            for (o, s) in buf.iter_mut().zip(&f) {
                *o = to_f32(*s);
            }
            d.on_mic_frame(&buf);
        }
        for o in d.poll_send(now) {
            match o {
                Outgoing::Reliable(p) => net.reliable(&p),
                Outgoing::Unsequenced(p) => {
                    net.unsequenced(&p);
                }
                Outgoing::Udp(p) => {
                    net.udp(&p);
                }
            }
        }
        self.fill(d, &mut out);
        let t = d.transmitting();
        if t != self.indicator {
            self.indicator = t;
            out.push(Notice::Indicator(t));
        }
        out
    }

    fn fill(&mut self, d: &mut dyn Driver, out: &mut Vec<Notice>) {
        if !self.io.playing() {
            if self.playback_failed {
                return;
            }
            if let Err(e) = self.io.set_playback(true) {
                self.playback_failed = true;
                out.push(Notice::Failed(format!("playback: {e}")));
                return;
            }
        }
        let ring = self.io.output();
        // The driver mixes one 20 ms frame per call (each call takes the next frame out of every jitter buffer),
        // so the ring is topped up in whole frames.
        while ring.len() < TARGET {
            self.mix.clear();
            self.mix.resize(BLOCK, 0.0);
            d.mix(&mut self.mix);
            self.pcm.clear();
            self.pcm.extend(self.mix.iter().map(|f| to_i16(*f)));
            ring.write(&self.pcm);
        }
    }

    /// The connection or the world ended: microphone and playback close, the indicator goes off.
    pub fn stop(&mut self, d: &mut dyn Driver) -> Vec<Notice> {
        let mut out = Vec::new();
        if self.mic_on {
            self.mic_on = false;
            let m = self.io.set_mic(false);
            self.mic(m, d, &mut out);
        }
        let _ = self.io.set_playback(false);
        self.playback_failed = false;
        if self.indicator.take().is_some() {
            out.push(Notice::Indicator(None));
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A voice driver that records what the pipeline did.
    use super::*;

    #[derive(Default)]
    pub struct FakeDriver {
        pub frames: Vec<f32>,
        pub mic: Vec<MicState>,
        pub want: bool,
        pub talking: Option<Mode>,
        pub send: Vec<Outgoing>,
        pub mixed: usize,
        pub level: f32,
    }

    impl Driver for FakeDriver {
        fn on_mic_frame(&mut self, pcm: &[f32; FRAME]) {
            self.frames.push(pcm[0]);
        }
        fn poll_send(&mut self, _now: u64) -> Vec<Outgoing> {
            std::mem::take(&mut self.send)
        }
        fn mix(&mut self, out: &mut [f32]) {
            self.mixed += out.len();
            out.fill(self.level);
        }
        fn mic_wanted(&self) -> bool {
            self.want
        }
        fn mic_state(&mut self, state: MicState) {
            self.mic.push(state);
        }
        fn transmitting(&self) -> Option<Mode> {
            self.talking
        }
    }

    #[derive(Default)]
    pub struct FakeNet {
        pub reliable: Vec<Vec<u8>>,
        pub unsequenced: Vec<Vec<u8>>,
        pub udp: Vec<Vec<u8>>,
    }

    impl Net for FakeNet {
        fn reliable(&mut self, packet: &[u8]) {
            self.reliable.push(packet.to_vec());
        }
        fn unsequenced(&mut self, packet: &[u8]) -> bool {
            self.unsequenced.push(packet.to_vec());
            true
        }
        fn udp(&mut self, datagram: &[u8]) -> bool {
            self.udp.push(datagram.to_vec());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::audio::fake::FakeBackend;
    use super::super::audio::Permission;
    use super::fake::{FakeDriver, FakeNet};
    use super::*;

    fn pipeline(permission: Permission) -> (Pipeline, FakeBackend) {
        let fake = FakeBackend::default();
        (Pipeline::new(AudioIo::new(Box::new(fake.clone()), permission)), fake)
    }

    #[test]
    fn samples_convert_both_ways() {
        assert_eq!(to_f32(-32768), -1.0);
        assert_eq!(to_i16(2.0), 32767);
        assert_eq!(to_i16(-2.0), -32768);
        assert_eq!(to_i16(0.5), 16383);
        assert_eq!((BLOCK, TARGET), (1920, 5760));
    }

    #[test]
    fn first_opt_in_asks_then_opens_and_frames_flow() {
        let (mut p, fake) = pipeline(Permission::Unknown);
        let (mut d, mut n) = (FakeDriver::default(), FakeNet::default());
        assert!(p.frame(0, &mut d, &mut n).is_empty());
        assert!(p.io().playing(), "playback runs from the first frame");
        assert_eq!(d.mixed, TARGET);
        d.want = true;
        assert_eq!(p.frame(1, &mut d, &mut n), vec![Notice::AskPermission]);
        assert!(p.frame(2, &mut d, &mut n).is_empty(), "waiting for the answer");
        d.want = false;
        assert_eq!(p.frame(2, &mut d, &mut n), vec![Notice::Mic(MicState::Off)]);
        d.want = true;
        assert!(p.frame(2, &mut d, &mut n).is_empty(), "the dialog is still up");
        d.mic.clear();
        assert_eq!(p.on_permission(true, &mut d), vec![Notice::Mic(MicState::On)]);
        assert_eq!(d.mic, vec![MicState::On]);
        let mut cb = fake.0.lock().unwrap().capture.take().unwrap();
        cb(&[16384; FRAME * 2]);
        d.send = vec![Outgoing::Reliable(vec![1]), Outgoing::Unsequenced(vec![2]), Outgoing::Udp(vec![3])];
        d.talking = Some(Mode::Proximity);
        let notes = p.frame(3, &mut d, &mut n);
        assert_eq!(notes, vec![Notice::Indicator(Some(Mode::Proximity))]);
        assert_eq!(d.frames, vec![0.5, 0.5]);
        assert_eq!((n.reliable.len(), n.unsequenced.len(), n.udp.len()), (1, 1, 1));
        d.want = false;
        d.talking = None;
        let notes = p.frame(4, &mut d, &mut n);
        assert_eq!(notes, vec![Notice::Mic(MicState::Off), Notice::Indicator(None)]);
    }

    #[test]
    fn the_ring_is_kept_at_the_target() {
        let (mut p, fake) = pipeline(Permission::Granted);
        let (mut d, mut n) = (FakeDriver { level: 0.25, ..Default::default() }, FakeNet::default());
        p.frame(0, &mut d, &mut n);
        assert_eq!(p.io().output().len(), TARGET);
        p.frame(1, &mut d, &mut n);
        assert_eq!(d.mixed, TARGET, "full: nothing mixed");
        let mut cb = fake.0.lock().unwrap().playback.take().unwrap();
        let mut dev = vec![0i16; 100];
        cb(&mut dev);
        assert_eq!(dev[0], 8191);
        p.frame(2, &mut d, &mut n);
        assert_eq!(d.mixed, TARGET + BLOCK, "topped up by one whole frame");
    }

    #[test]
    fn denial_failures_and_stop() {
        let (mut p, fake) = pipeline(Permission::Unknown);
        let (mut d, mut n) = (FakeDriver { want: true, ..Default::default() }, FakeNet::default());
        p.frame(0, &mut d, &mut n);
        assert_eq!(p.on_permission(false, &mut d), vec![Notice::Mic(MicState::Denied)]);
        assert_eq!(p.stop(&mut d), vec![Notice::Mic(MicState::Off)]);
        assert!(!p.io().playing());
        assert!(p.stop(&mut d).is_empty());
        // a backend that cannot open anything
        fake.0.lock().unwrap().fail = true;
        let (mut p, fake2) = pipeline(Permission::Granted);
        fake2.0.lock().unwrap().fail = true;
        let notes = p.frame(0, &mut d, &mut n);
        assert_eq!(
            notes,
            vec![
                Notice::Failed("microphone: no mic".into()),
                Notice::Mic(MicState::Off),
                Notice::Failed("playback: no output".into())
            ]
        );
        assert!(p.frame(1, &mut d, &mut n).is_empty(), "playback is not retried every frame");
        d.talking = Some(Mode::Global);
        p.frame(2, &mut d, &mut n);
        assert_eq!(p.stop(&mut d), vec![Notice::Mic(MicState::Off), Notice::Indicator(None)]);
    }
}
