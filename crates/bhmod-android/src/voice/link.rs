//! The voice client of one connection as the loader sees it ([`VoiceLink`], `bhmod_voice::client::SharedVoiceClient`
//! on a phone), the [`VoiceSlot`] the mods reach it through (`GameBridge::voice`), and the [`LinkDriver`] that lets
//! the [`super::pipeline::Pipeline`] drive it.

use std::cell::RefCell;
use std::rc::Rc;

use bhmod_core::types::ApiError;
use bhmod_core::voice::{Activation, Icons, MicState, Mode, VoiceEvent, VoiceHost, VoiceState};
use bhmod_proto::Message;

use super::audio::FRAME;
use super::pipeline::{Driver, Outgoing};

/// A shared voice client (every method takes `&self`: the mods reach it through [`VoiceHost`] while the loader
/// drives it).
pub trait VoiceLink: VoiceHost {
    /// Starts the key exchange: the `VOICE_KEY` packet (`None` when no key could be made).
    fn start(&self) -> Option<Vec<u8>>;
    /// A voice op from the server (`Output::Voice`).
    fn on_message(&self, msg: &Message);
    /// A datagram from the server's voice UDP port.
    fn on_udp(&self, datagram: &[u8]);
    /// The server's voice UDP port, once the key exchange announced one.
    fn udp_port(&self) -> Option<u16>;
    /// A 20 ms microphone frame.
    fn on_mic_frame(&self, pcm: &[f32]);
    /// What to send now.
    fn poll_send(&self, now: u64) -> Vec<Outgoing>;
    /// Mixes into interleaved stereo.
    fn mix(&self, out: &mut [f32]);
    /// True while a mod wants the microphone.
    fn mic_wanted(&self) -> bool;
    /// The microphone changed.
    fn mic_state(&self, state: MicState);
    /// The mode transmitted right now.
    fn transmitting(&self) -> Option<Mode>;
    /// Events for `ModHost::voice_event`.
    fn take_events(&self) -> Vec<VoiceEvent>;
    /// Master volume, per-player volumes and local mutes as text (stored on the device).
    fn prefs(&self) -> String;
    /// Restores [`VoiceLink::prefs`].
    fn load_prefs(&self, text: &str);
}

/// Drives a [`VoiceLink`] from the pipeline.
pub struct LinkDriver<'a>(pub &'a dyn VoiceLink);

impl Driver for LinkDriver<'_> {
    fn on_mic_frame(&mut self, pcm: &[f32; FRAME]) {
        self.0.on_mic_frame(pcm);
    }
    fn poll_send(&mut self, now: u64) -> Vec<Outgoing> {
        self.0.poll_send(now)
    }
    fn mix(&mut self, out: &mut [f32]) {
        self.0.mix(out);
    }
    fn mic_wanted(&self) -> bool {
        self.0.mic_wanted()
    }
    fn mic_state(&mut self, state: MicState) {
        self.0.mic_state(state);
    }
    fn transmitting(&self) -> Option<Mode> {
        self.0.transmitting()
    }
}

/// What `GameBridge::voice` hands the mods: the client functions of the current connection's [`VoiceLink`]
/// (`unsupported` while there is none; the server functions are always `unsupported` on a client).
#[derive(Default)]
pub struct VoiceSlot {
    link: RefCell<Option<Rc<dyn VoiceLink>>>,
}

impl VoiceSlot {
    /// Puts in (or takes out) the voice client.
    pub fn set(&self, link: Option<Rc<dyn VoiceLink>>) {
        *self.link.borrow_mut() = link;
    }

    /// The voice client, if any.
    pub fn get(&self) -> Option<Rc<dyn VoiceLink>> {
        self.link.borrow().clone()
    }

    /// True while a voice client is in.
    pub fn is_set(&self) -> bool {
        self.link.borrow().is_some()
    }

    fn with<R>(&self, f: impl FnOnce(&dyn VoiceLink) -> Result<R, ApiError>) -> Result<R, ApiError> {
        let l = self.get().ok_or(ApiError::Unsupported)?;
        f(&*l)
    }
}

impl VoiceHost for VoiceSlot {
    fn state(&self, player: &str) -> Result<VoiceState, ApiError> {
        self.with(|l| l.state(player))
    }
    fn icons(&self) -> Result<Icons, ApiError> {
        self.with(|l| l.icons())
    }
    fn set_default_mode(&self, mode: Mode) -> Result<(), ApiError> {
        self.with(|l| l.set_default_mode(mode))
    }
    fn set_activation(&self, activation: Activation) -> Result<(), ApiError> {
        self.with(|l| l.set_activation(activation))
    }
    fn talk(&self, mode: Option<Mode>, pressed: bool) -> Result<(), ApiError> {
        self.with(|l| l.talk(mode, pressed))
    }
    fn set_mic(&self, on: bool) -> Result<(), ApiError> {
        self.with(|l| l.set_mic(on))
    }
    fn set_self_mute(&self, on: bool) -> Result<(), ApiError> {
        self.with(|l| l.set_self_mute(on))
    }
    fn set_deafen(&self, on: bool) -> Result<(), ApiError> {
        self.with(|l| l.set_deafen(on))
    }
    fn set_vad_threshold(&self, db: f32) -> Result<(), ApiError> {
        self.with(|l| l.set_vad_threshold(db))
    }
    fn set_master(&self, percent: u16) -> Result<(), ApiError> {
        self.with(|l| l.set_master(percent))
    }
    fn master(&self) -> Result<u16, ApiError> {
        self.with(|l| l.master())
    }
    fn set_player_volume(&self, player: &str, percent: u16) -> Result<(), ApiError> {
        self.with(|l| l.set_player_volume(player, percent))
    }
    fn player_volume(&self, player: &str) -> Result<u16, ApiError> {
        self.with(|l| l.player_volume(player))
    }
    fn mute_player(&self, player: &str, muted: bool) -> Result<(), ApiError> {
        self.with(|l| l.mute_player(player, muted))
    }
    fn player_muted(&self, player: &str) -> Result<bool, ApiError> {
        self.with(|l| l.player_muted(player))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A voice client that records what the loader and the mods did.
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    pub struct FakeLink {
        pub messages: RefCell<Vec<Message>>,
        pub udp_in: RefCell<Vec<Vec<u8>>>,
        pub udp_port: Cell<Option<u16>>,
        pub frames: Cell<usize>,
        pub send: RefCell<Vec<Outgoing>>,
        pub want: Cell<bool>,
        pub mic: RefCell<Vec<MicState>>,
        pub talking: Cell<Option<Mode>>,
        pub events: RefCell<Vec<VoiceEvent>>,
        pub prefs: RefCell<String>,
        pub master: Cell<u16>,
        pub default_mode: Cell<Option<Mode>>,
        pub talks: RefCell<Vec<(Option<Mode>, bool)>>,
        pub icons: Cell<Option<Icons>>,
        pub no_key: Cell<bool>,
        pub volumes: RefCell<BTreeMap<String, u16>>,
    }

    impl VoiceHost for FakeLink {
        fn state(&self, _player: &str) -> Result<VoiceState, ApiError> {
            Ok(VoiceState::Ready)
        }
        fn icons(&self) -> Result<Icons, ApiError> {
            self.icons.get().ok_or(ApiError::Unsupported)
        }
        fn set_default_mode(&self, mode: Mode) -> Result<(), ApiError> {
            self.default_mode.set(Some(mode));
            Ok(())
        }
        fn set_activation(&self, _a: Activation) -> Result<(), ApiError> {
            Ok(())
        }
        fn talk(&self, mode: Option<Mode>, pressed: bool) -> Result<(), ApiError> {
            self.talks.borrow_mut().push((mode, pressed));
            Ok(())
        }
        fn set_mic(&self, on: bool) -> Result<(), ApiError> {
            self.want.set(on);
            Ok(())
        }
        fn set_self_mute(&self, _on: bool) -> Result<(), ApiError> {
            Ok(())
        }
        fn set_deafen(&self, _on: bool) -> Result<(), ApiError> {
            Ok(())
        }
        fn set_vad_threshold(&self, _db: f32) -> Result<(), ApiError> {
            Ok(())
        }
        fn set_master(&self, percent: u16) -> Result<(), ApiError> {
            self.master.set(percent);
            *self.prefs.borrow_mut() = format!("master={percent}");
            Ok(())
        }
        fn master(&self) -> Result<u16, ApiError> {
            Ok(self.master.get())
        }
        fn set_player_volume(&self, player: &str, percent: u16) -> Result<(), ApiError> {
            self.volumes.borrow_mut().insert(player.into(), percent);
            Ok(())
        }
        fn player_volume(&self, player: &str) -> Result<u16, ApiError> {
            Ok(self.volumes.borrow().get(player).copied().unwrap_or(100))
        }
        fn mute_player(&self, _player: &str, _muted: bool) -> Result<(), ApiError> {
            Ok(())
        }
        fn player_muted(&self, _player: &str) -> Result<bool, ApiError> {
            Ok(false)
        }
    }

    impl VoiceLink for FakeLink {
        fn start(&self) -> Option<Vec<u8>> {
            (!self.no_key.get()).then(|| vec![0xBC, 0x15])
        }
        fn on_message(&self, msg: &Message) {
            self.messages.borrow_mut().push(msg.clone());
        }
        fn on_udp(&self, datagram: &[u8]) {
            self.udp_in.borrow_mut().push(datagram.to_vec());
        }
        fn udp_port(&self) -> Option<u16> {
            self.udp_port.get()
        }
        fn on_mic_frame(&self, _pcm: &[f32]) {
            self.frames.set(self.frames.get() + 1);
        }
        fn poll_send(&self, _now: u64) -> Vec<Outgoing> {
            std::mem::take(&mut self.send.borrow_mut())
        }
        fn mix(&self, out: &mut [f32]) {
            out.fill(0.0);
        }
        fn mic_wanted(&self) -> bool {
            self.want.get()
        }
        fn mic_state(&self, state: MicState) {
            self.mic.borrow_mut().push(state);
            self.events.borrow_mut().push(VoiceEvent::Mic(state));
        }
        fn transmitting(&self) -> Option<Mode> {
            self.talking.get()
        }
        fn take_events(&self) -> Vec<VoiceEvent> {
            std::mem::take(&mut self.events.borrow_mut())
        }
        fn prefs(&self) -> String {
            self.prefs.borrow().clone()
        }
        fn load_prefs(&self, text: &str) {
            *self.prefs.borrow_mut() = text.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeLink;
    use super::*;

    #[test]
    fn the_slot_forwards_the_client_functions_and_is_unsupported_when_empty() {
        let slot = VoiceSlot::default();
        assert!(!slot.is_set());
        assert_eq!(slot.master(), Err(ApiError::Unsupported));
        let link = Rc::new(FakeLink::default());
        slot.set(Some(link.clone()));
        assert!(slot.is_set());
        assert_eq!(slot.state("p"), Ok(VoiceState::Ready));
        assert_eq!(slot.icons(), Err(ApiError::Unsupported));
        slot.set_default_mode(Mode::Global).unwrap();
        slot.set_activation(Activation::Vad).unwrap();
        slot.talk(None, true).unwrap();
        slot.set_mic(true).unwrap();
        slot.set_self_mute(true).unwrap();
        slot.set_deafen(false).unwrap();
        slot.set_vad_threshold(-40.0).unwrap();
        slot.set_master(150).unwrap();
        assert_eq!(slot.master(), Ok(150));
        slot.set_player_volume("p", 50).unwrap();
        assert_eq!(slot.player_volume("p"), Ok(50));
        slot.mute_player("p", true).unwrap();
        assert_eq!(slot.player_muted("p"), Ok(false));
        // server functions stay unsupported on a client
        assert_eq!(slot.set_routes("p", &[]), Err(ApiError::Unsupported));
        assert_eq!(link.default_mode.get(), Some(Mode::Global));
        assert!(link.want.get());
        slot.set(None);
        assert!(slot.get().is_none());
    }

    #[test]
    fn the_driver_forwards_to_the_link() {
        let link = FakeLink::default();
        link.want.set(true);
        link.talking.set(Some(Mode::Channel));
        link.send.borrow_mut().push(Outgoing::Udp(vec![1]));
        let mut d = LinkDriver(&link);
        d.on_mic_frame(&[0.0; FRAME]);
        assert_eq!(link.frames.get(), 1);
        assert_eq!(d.poll_send(0), vec![Outgoing::Udp(vec![1])]);
        let mut out = [1.0; 4];
        d.mix(&mut out);
        assert_eq!(out, [0.0; 4]);
        assert!(d.mic_wanted());
        d.mic_state(MicState::On);
        assert_eq!(d.transmitting(), Some(Mode::Channel));
        assert_eq!(link.take_events(), vec![VoiceEvent::Mic(MicState::On)]);
        assert_eq!(link.start(), Some(vec![0xBC, 0x15]));
        link.on_udp(&[9]);
        link.on_message(&Message::ListRequest);
        assert_eq!(link.udp_port(), None);
        link.load_prefs("x");
        assert_eq!(link.prefs(), "x");
    }
}
