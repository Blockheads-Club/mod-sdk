//! The real voice client: `bhmod_voice`'s [`SharedVoiceClient`] with libopus ([`OpusCodec`], armv7 NEON build on a
//! phone) behind the loader's [`VoiceLink`].

use std::rc::Rc;
use std::sync::Arc;

use bhmod_core::voice::{MicState, Mode, VoiceEvent};
use bhmod_opus_sys::OpusCodec;
use bhmod_proto::Message;
use bhmod_voice::client::{Outgoing as Out, SharedVoiceClient, VoiceClient};

use super::link::VoiceLink;
use super::pipeline::Outgoing;
use crate::loader::VoiceFactory;

fn out(o: Out) -> Outgoing {
    match o {
        Out::Reliable(p) => Outgoing::Reliable(p),
        Out::Udp(p) => Outgoing::Udp(p),
    }
}

impl VoiceLink for SharedVoiceClient {
    fn start(&self) -> Option<Vec<u8>> {
        self.lock().start()
    }
    fn on_message(&self, msg: &Message) {
        self.lock().on_message(msg);
    }
    fn on_udp(&self, datagram: &[u8]) {
        self.lock().on_udp(datagram);
    }
    fn udp_port(&self) -> Option<u16> {
        self.lock().udp_port()
    }
    fn on_mic_frame(&self, pcm: &[f32]) {
        self.lock().on_mic_frame(pcm);
    }
    fn poll_send(&self, now: u64) -> Vec<Outgoing> {
        self.lock().poll_send(now).into_iter().map(out).collect()
    }
    fn mix(&self, out: &mut [f32]) {
        self.lock().mix(out);
    }
    fn mic_wanted(&self) -> bool {
        self.lock().mic_wanted()
    }
    fn mic_state(&self, state: MicState) {
        self.lock().mic_state(state);
    }
    fn transmitting(&self) -> Option<Mode> {
        self.lock().transmitting()
    }
    fn take_events(&self) -> Vec<VoiceEvent> {
        self.lock().take_events()
    }
    fn prefs(&self) -> String {
        self.lock().prefs()
    }
    fn load_prefs(&self, text: &str) {
        self.lock().load_prefs(text);
    }
    fn players(&self) -> Vec<(String, String)> {
        self.lock().players()
    }
    fn alias(&self, player: &str) -> Option<String> {
        self.lock().alias(player)
    }
    fn speakers(&self) -> Vec<String> {
        self.lock().speakers()
    }
}

/// The loader's voice factory: a fresh libopus voice client per voice-enabled connection.
pub fn factory() -> VoiceFactory {
    Box::new(move || {
        let c = VoiceClient::new(Arc::new(OpusCodec));
        Some(Rc::new(SharedVoiceClient::new(c)) as Rc<dyn VoiceLink>)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::audio::FRAME;

    #[test]
    fn packets_map_one_to_one() {
        assert_eq!(out(Out::Reliable(vec![1])), Outgoing::Reliable(vec![1]));
        assert_eq!(out(Out::Udp(vec![3])), Outgoing::Udp(vec![3]));
    }

    #[test]
    fn the_real_client_behind_the_link() {
        let link = factory()().unwrap();
        let key = link.start().unwrap();
        assert!(matches!(Message::decode(&key), Ok(Message::VoiceKey(_))));
        link.on_message(&Message::ListRequest);
        link.on_udp(&[1, 2, 3]);
        assert_eq!(link.udp_port(), None);
        assert!(!link.mic_wanted());
        link.set_mic(true).unwrap();
        assert!(link.mic_wanted());
        link.mic_state(MicState::On);
        assert!(link.take_events().contains(&VoiceEvent::Mic(MicState::On)));
        link.on_mic_frame(&[0.0; FRAME]);
        assert!(link.poll_send(0).is_empty(), "no session key yet: nothing to send");
        assert_eq!(link.transmitting(), None);
        let mut mixed = [1.0f32; FRAME * 2];
        link.mix(&mut mixed);
        assert!(mixed.iter().all(|x| *x == 0.0));
        link.set_master(140).unwrap();
        let prefs = link.prefs();
        let other = factory()().unwrap();
        other.load_prefs(&prefs);
        assert_eq!(other.master(), Ok(140));
        assert!(link.players().is_empty() && link.speakers().is_empty());
        assert_eq!(link.alias("p"), None);
    }
}
