//! Voice chat groundwork on Android (plan: `notes/voice-mod.md`, milestone V6): microphone capture and playback
//! ([`audio`]), the unsequenced 0xBC send path ([`net`]) and the events the Java side delivers ([`VoiceEvent`]).
//! Opus, encryption, jitter buffer and mixing live in `bhmod-voice`; the `bh.voice` host API wires them to this.

pub mod audio;
pub mod net;

use audio::{AudioIo, Mic};

/// Something the Java side reported (queued from the UI thread, handled on the loader thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceEvent {
    /// The answer of the `RECORD_AUDIO` dialog.
    Permission(bool),
    /// The push-to-talk control went down (`true`) or up. The HUD button is a later milestone; this is its seam.
    Ptt(bool),
}

/// The client voice state the loader thread owns.
pub struct Voice {
    io: AudioIo,
    ptt: bool,
}

impl Voice {
    /// Voice over these streams.
    pub fn new(io: AudioIo) -> Voice {
        Voice { io, ptt: false }
    }

    /// The audio streams.
    pub fn io(&mut self) -> &mut AudioIo {
        &mut self.io
    }

    /// Handles a Java event. A permission answer returns the new microphone state.
    pub fn on_event(&mut self, ev: VoiceEvent) -> Option<Mic> {
        match ev {
            VoiceEvent::Permission(granted) => Some(self.io.on_permission(granted)),
            VoiceEvent::Ptt(down) => {
                self.ptt = down;
                None
            }
        }
    }

    /// True while push-to-talk is held.
    pub fn ptt(&self) -> bool {
        self.ptt
    }

    /// True when captured frames should be sent now: the microphone is open and push-to-talk is held.
    pub fn transmitting(&self) -> bool {
        self.ptt && self.io.mic_open()
    }
}

#[cfg(test)]
mod tests {
    use super::audio::fake::FakeBackend;
    use super::audio::Permission;
    use super::*;

    #[test]
    fn ptt_and_permission_events() {
        let io = AudioIo::new(Box::new(FakeBackend::default()), Permission::Unknown);
        let mut v = Voice::new(io);
        assert_eq!(v.io().set_mic(true), Mic::AskPermission);
        assert_eq!(v.on_event(VoiceEvent::Ptt(true)), None);
        assert!(v.ptt());
        assert!(!v.transmitting());
        assert_eq!(v.on_event(VoiceEvent::Permission(true)), Some(Mic::Open));
        assert!(v.transmitting());
        v.on_event(VoiceEvent::Ptt(false));
        assert!(!v.transmitting());
    }
}
