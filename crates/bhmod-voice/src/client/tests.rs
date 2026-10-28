use std::collections::BTreeMap;

use bhmod_proto::VoiceFrame;

use super::*;
use crate::codec::{CodecError, TestCodec};
use crate::server::{ServerSink, VoiceServer};

#[derive(Default)]
struct Sink(Vec<(String, Vec<u8>)>);

impl ServerSink for Sink {
    fn reliable(&mut self, player: &str, packet: Vec<u8>) {
        self.0.push((player.into(), packet));
    }
    fn unsequenced(&mut self, player: &str, packet: Vec<u8>) {
        self.0.push((player.into(), packet));
    }
    fn udp(&mut self, _peer: PeerId, _datagram: Vec<u8>) {}
}

fn feats() -> Vec<String> {
    vec!["voice".into(), "voice-unsequenced".into()]
}

/// A server with talker `a` (routes to `b`) and listener `b`, both connected; returns (server, a, b).
fn pair() -> (VoiceServer, VoiceClient, VoiceClient, Sink) {
    let mut n = 0u8;
    let mut s = VoiceServer::with_keys(Box::new(move || {
        n += 1;
        Some(KeyPair::from_secret([90 + n; 32]))
    }));
    let mut sink = Sink::default();
    let mut cs = Vec::new();
    for (name, seed) in [("a", 1u8), ("b", 2)] {
        s.join(name, &uc_alias(name), &feats());
        let mut c = VoiceClient::new(Arc::new(TestCodec), true);
        let key = c.start_with(KeyPair::from_secret([seed; 32]));
        s.receive(0, name, &Message::decode(&key).unwrap(), 0, &mut sink);
        for (_, p) in std::mem::take(&mut sink.0) {
            c.on_packet(&p);
        }
        cs.push(c);
    }
    let route = bhmod_core::voice::Route { listener: "b".into(), modes: 0, gain: 1.0, pan: 0.5, muffle: 0.0 };
    s.set_routes("a", &[route]);
    s.set_allowed("a", 0x0f, 0x07);
    s.tick(1, &mut sink);
    let mut b = cs.pop().unwrap();
    let mut a = cs.pop().unwrap();
    for (to, p) in std::mem::take(&mut sink.0) {
        if to == "a" {
            a.on_packet(&p)
        } else {
            b.on_packet(&p)
        }
    }
    a.take_events();
    b.take_events();
    (s, a, b, sink)
}

fn tone() -> Vec<f32> {
    (0..FRAME_SAMPLES).map(|i| if i % 40 < 20 { 0.3 } else { -0.3 }).collect()
}

/// `a` sends one frame; the server relays it to `b`.
fn say(s: &mut VoiceServer, a: &mut VoiceClient, b: &mut VoiceClient, sink: &mut Sink, now: u64) -> bool {
    a.on_mic_frame(&tone());
    let sent = a.poll_send(now);
    let Some(Outgoing::Unsequenced(p)) = sent.first() else { return false };
    s.receive(now, "a", &Message::decode(p).unwrap(), 0, sink);
    for (_, p) in std::mem::take(&mut sink.0) {
        b.on_packet(&p);
    }
    true
}

#[test]
fn talk_hear_and_mix() {
    let (mut s, mut a, mut b, mut sink) = pair();
    assert_eq!(b.state("a"), State::Ready);
    assert_eq!(a.transmitting(), None);
    // mic off: nothing
    a.talk(None, true);
    assert!(!say(&mut s, &mut a, &mut b, &mut sink, 10));
    a.set_mic(true);
    assert!(a.mic_wanted());
    // wrong frame length: ignored
    a.on_mic_frame(&[0.0; 10]);
    for t in 0..6 {
        assert!(say(&mut s, &mut a, &mut b, &mut sink, 20 + t * 20));
    }
    assert_eq!(a.transmitting(), Some(Mode::Proximity));
    assert_eq!(b.take_events(), vec![VoiceEvent::TalkStart { player: "a".into(), mode: Mode::Proximity }]);
    let mut out = vec![0.0; 2 * FRAME_SAMPLES];
    b.mix(&mut out);
    assert!(out.iter().any(|x| *x != 0.0), "heard");
    assert!(out[1].abs() >= out[0].abs(), "panned right");
    // drain: lost frames conceal, then the spurt ends
    for _ in 0..20 {
        b.mix(&mut out);
    }
    assert_eq!(b.take_events(), vec![VoiceEvent::TalkStop { player: "a".into(), mode: Mode::Proximity }]);
    assert!(out.iter().all(|x| *x == 0.0));
    a.talk(None, false);
    assert_eq!(a.transmitting(), None);

    // local mute / volume of `a` on b's side
    b.mute_player("a", true);
    assert!(b.player_muted("a"));
    for t in 0..6 {
        say(&mut s, &mut a, &mut b, &mut sink, 1_000 + t * 20);
    }
    a.talk(Some(Mode::Global), true);
    for t in 0..6 {
        say(&mut s, &mut a, &mut b, &mut sink, 2_000 + t * 20);
    }
    b.mix(&mut out);
    assert!(out.iter().all(|x| *x == 0.0), "muted locally");
    b.mute_player("a", false);
    b.set_player_volume("a", 300);
    assert_eq!(b.player_volume("a"), 200);
    b.set_master(250);
    assert_eq!(b.master(), 200);

    // deafen drops what arrives and tells the server
    b.set_deafen(true);
    let sent = b.poll_send(3_000);
    assert!(matches!(&sent[..], [Outgoing::Reliable(_)]));
    say(&mut s, &mut a, &mut b, &mut sink, 3_000);
    b.mix(&mut out);
    assert!(out.iter().all(|x| *x == 0.0));
    b.set_deafen(false);
    // self mute stops sending
    a.set_self_mute(true);
    a.on_mic_frame(&tone());
    assert!(matches!(&a.poll_send(3_100)[..], [Outgoing::Reliable(_)]));
}

#[test]
fn activation_and_allowed() {
    let (_s, mut a, _b, _) = pair();
    a.set_mic(true);
    assert!(a.set_activation(Act::Toggle));
    a.take_events();
    a.set_default_mode(Mode::Channel);
    a.set_default_mode(Mode::Channel);
    assert_eq!(a.take_events(), vec![VoiceEvent::Mode(Mode::Channel)]);
    assert_eq!(a.default_mode(), Mode::Channel);
    a.talk(None, true);
    assert_eq!(a.transmitting(), Some(Mode::Channel));
    a.talk(None, false);
    assert_eq!(a.transmitting(), Some(Mode::Channel), "toggle ignores the release");
    a.talk(None, true);
    assert_eq!(a.transmitting(), None);
    assert!(a.set_activation(Act::Vad));
    a.set_vad_threshold(-20.0);
    a.on_mic_frame(&tone());
    a.on_mic_frame(&tone());
    assert_eq!(a.transmitting(), Some(Mode::Channel));
    a.on_mic_frame(&[0.0; FRAME_SAMPLES]);
    assert!(a.set_activation(Act::Ptt));
    // the server allows PTT only
    a.take_events();
    a.on_message(&Message::VoiceState(VoiceState { activations: Some(1), modes: Some(1), ..Default::default() }));
    let allowed = Allowance { modes: 1, activations: 1, max_bitrate: 24_000 };
    assert_eq!(a.take_events(), vec![VoiceEvent::Allowed(allowed)]);
    a.set_self_mute(true);
    a.set_deafen(true);
    assert!(a.self_muted() && a.deafened());
    assert!(!a.set_activation(Act::Vad));
    assert!(!a.set_activation(Act::Toggle));
    // denied microphone
    a.mic_state(MicState::Denied);
    assert!(!a.mic_wanted());
    a.mic_state(MicState::On);
    assert_eq!(a.take_events()[..], [VoiceEvent::Mic(MicState::Denied), VoiceEvent::Mic(MicState::On)]);
}

#[test]
fn state_messages_and_prefs() {
    let mut c = VoiceClient::new(Arc::new(TestCodec), false);
    assert!(c.start().is_some());
    assert!(!c.connected());
    // frames before a session, garbage, other ops: ignored
    c.on_message(&Message::VoiceFrame(VoiceFrame { nonce: 1, sealed: vec![0; 16] }));
    c.on_udp(&[1, 2]);
    c.on_packet(&[0xff]);
    c.on_message(&Message::ListRequest);
    // a key reply without a peer id or with a low-order key does nothing
    c.on_message(&Message::VoiceKey(VoiceKey { public: [5; 32], peer: None, udp_port: None }));
    c.on_message(&Message::VoiceKey(VoiceKey { public: [0; 32], peer: Some(1), udp_port: None }));
    assert!(!c.connected());

    let full = VoiceState {
        full: true,
        players: BTreeMap::from([("x".into(), "talking".into()), ("y".into(), "bogus".into())]),
        speakers: BTreeMap::from([("x".into(), 4)]),
        icons: BTreeMap::from([("admin_mute".into(), false)]),
        bitrate: Some(16_000),
        ..Default::default()
    };
    c.on_message(&Message::VoiceState(full.clone()));
    assert_eq!((c.state("x"), c.state("y"), c.state("z")), (State::Talking, State::None, State::None));
    assert!(!c.icons().admin_mute);
    assert_eq!(c.bitrate, 16_000);
    c.on_message(&Message::VoiceState(full));
    assert_eq!(c.alias("x"), None, "no names sent");
    let named = VoiceState { names: BTreeMap::from([("x".into(), "EX".into())]), ..Default::default() };
    c.on_message(&Message::VoiceState(named));
    assert_eq!(c.alias("x").as_deref(), Some("EX"));
    let delta = VoiceState { players: BTreeMap::from([("x".into(), "gone".into())]), ..Default::default() };
    c.on_message(&Message::VoiceState(delta));
    assert_eq!((c.state("x"), c.alias("x")), (State::None, None));
    let ev = c.take_events();
    assert_eq!(ev.len(), 3, "unchanged states are not repeated: {ev:?}");
    let only = VoiceState { full: true, players: BTreeMap::from([("q".into(), "ready".into())]), ..Default::default() };
    c.on_message(&Message::VoiceState(only));
    assert_eq!(c.states.len(), 1);

    c.set_master(80);
    c.set_player_volume("p1", 50);
    c.mute_player("p2", true);
    let text = c.prefs();
    assert_eq!(text, "master 80\nplayer p1 50 0\nplayer p2 100 1\n");
    let mut d = VoiceClient::new(Arc::new(TestCodec), false);
    d.load_prefs(&format!("{text}junk\nmaster x\nplayer p3 x 1\nmaster 900\n"));
    assert_eq!((d.master(), d.player_volume("p1"), d.player_muted("p2"), d.player_muted("p3")), (200, 50, true, false));
    // self mute and deafen before a session send nothing
    d.set_self_mute(true);
    d.set_deafen(true);
    assert!(d.poll_send(0).is_empty());
}

#[test]
fn udp_path_and_keepalive() {
    let mut s = VoiceServer::with_keys(Box::new(|| Some(KeyPair::from_secret([77; 32]))));
    s.set_transport(bhmod_core::voice::TransportConfig {
        mode: bhmod_core::voice::Transport::Udp,
        udp_port: Some(9000),
    });
    let mut sink = Sink::default();
    s.join("a", &uc_alias("a"), &["voice".to_string()]);
    let mut c = VoiceClient::new(Arc::new(TestCodec), false);
    let key = c.start_with(KeyPair::from_secret([3; 32]));
    s.receive(0, "a", &Message::decode(&key).unwrap(), 0, &mut sink);
    for (_, p) in std::mem::take(&mut sink.0) {
        c.on_packet(&p);
    }
    assert_eq!(c.udp_port(), Some(9000));
    assert_eq!(c.poll_send(0).len(), 1);
    assert!(c.poll_send(KEEPALIVE_MS - 1).is_empty());
    assert_eq!(c.poll_send(KEEPALIVE_MS).len(), 1);
    // not authenticated on UDP yet and no in-band: frames cannot go anywhere
    s.set_allowed("a", 1, 1);
    s.tick(1, &mut sink);
    for (_, p) in std::mem::take(&mut sink.0) {
        c.on_packet(&p);
    }
    c.set_mic(true);
    c.talk(None, true);
    c.on_mic_frame(&tone());
    assert!(c.poll_send(KEEPALIVE_MS + 1).is_empty());
}

/// A codec whose encoder or decoder fails.
struct Broken {
    encoder: bool,
}

struct BadEnc;
impl Encoder for BadEnc {
    fn encode(&mut self, _: &[f32], _: &mut Vec<u8>) -> Result<(), CodecError> {
        Err(CodecError::Corrupt)
    }
    fn set_bitrate(&mut self, _: u32) {}
}

struct BadDec;
impl Decoder for BadDec {
    fn decode(&mut self, _: Option<&[u8]>, _: &mut [f32]) -> Result<(), CodecError> {
        Err(CodecError::Corrupt)
    }
}

impl Codec for Broken {
    fn encoder(&self, _: u32) -> Result<Box<dyn Encoder>, CodecError> {
        if self.encoder {
            Ok(Box::new(BadEnc))
        } else {
            TestCodec.encoder(24_000)
        }
    }
    fn decoder(&self) -> Result<Box<dyn Decoder>, CodecError> {
        if self.encoder {
            Err(CodecError::Corrupt)
        } else {
            Ok(Box::new(BadDec))
        }
    }
}

#[test]
fn codec_failures() {
    let (mut s, mut a, mut b, mut sink) = pair();
    // a's encoder fails: nothing is sent
    a.encoder = Broken { encoder: true }.encoder(1).ok();
    a.set_mic(true);
    a.talk(None, true);
    assert!(!say(&mut s, &mut a, &mut b, &mut sink, 10));
    // b cannot make a decoder: frames are dropped
    a.encoder = TestCodec.encoder(24_000).ok();
    b.codec = Arc::new(Broken { encoder: true });
    assert!(say(&mut s, &mut a, &mut b, &mut sink, 30));
    assert!(b.speakers.is_empty());
    // b's decoder fails: silence
    b.codec = Arc::new(Broken { encoder: false });
    for t in 0..6 {
        say(&mut s, &mut a, &mut b, &mut sink, 50 + t * 20);
    }
    let mut out = vec![1.0; 2 * FRAME_SAMPLES];
    b.mix(&mut out);
    b.mix(&mut out);
    assert!(out.iter().all(|x| *x == 0.0));
    // a frame from an unknown speaker id
    b.peer_of.clear();
    say(&mut s, &mut a, &mut b, &mut sink, 500);
}

#[test]
fn shared_client_is_a_voice_host() {
    let shared = SharedVoiceClient::new(VoiceClient::new(Arc::new(TestCodec), true));
    let h: &dyn VoiceHost = &shared;
    assert_eq!(h.state("p"), Ok(State::None));
    assert_eq!(h.icons(), Ok(Icons::default()));
    h.set_default_mode(Mode::Global).unwrap();
    h.set_activation(Act::Vad).unwrap();
    shared.lock().activation.allowed.vad = false;
    assert!(matches!(h.set_activation(Act::Vad), Err(ApiError::InvalidArgument(_))));
    h.talk(None, true).unwrap();
    h.set_mic(true).unwrap();
    h.set_self_mute(false).unwrap();
    h.set_deafen(false).unwrap();
    h.set_vad_threshold(-30.0).unwrap();
    h.set_master(90).unwrap();
    assert_eq!(h.master(), Ok(90));
    h.set_player_volume("p", 70).unwrap();
    assert_eq!(h.player_volume("p"), Ok(70));
    h.mute_player("p", true).unwrap();
    assert_eq!(h.player_muted("p"), Ok(true));
    assert_eq!(h.mute("p", None), Err(ApiError::Unsupported), "server functions stay unsupported");
    assert!(shared.lock().mic_wanted());
    assert_eq!(h.allowed().unwrap(), Allowance { modes: 0, activations: 0b101, max_bitrate: 24_000 });
    assert_eq!((h.self_muted(), h.deafened(), h.mic()), (Ok(false), Ok(false), Ok(MicState::Off)));
    shared.lock().mic_state(MicState::On);
    assert_eq!(h.mic(), Ok(MicState::On));
}

#[test]
fn poisoned_lock_is_taken_over() {
    let shared = SharedVoiceClient::new(VoiceClient::new(Arc::new(TestCodec), true));
    let s2 = shared.clone();
    let _ = std::thread::spawn(move || {
        let _g = s2.lock();
        panic!("poison");
    })
    .join();
    assert_eq!(shared.lock().master(), 100);
}

#[test]
fn heard_speakers_feed_the_loss_of_auto() {
    let (mut s, mut a, mut b, mut sink) = pair();
    let route = bhmod_core::voice::Route { listener: "a".into(), modes: 0, gain: 1.0, pan: 0.0, muffle: 0.0 };
    s.set_routes("b", &[route]);
    s.set_allowed("b", 0x0f, 0x07);
    s.tick(2, &mut sink);
    for (to, p) in std::mem::take(&mut sink.0) {
        if to == "b" {
            b.on_packet(&p);
        }
    }
    b.set_mic(true);
    b.talk(None, true);
    b.on_mic_frame(&tone());
    let sent = b.poll_send(10);
    let [Outgoing::Unsequenced(p)] = &sent[..] else { panic!("{sent:?}") };
    s.receive(10, "b", &Message::decode(p).unwrap(), 0, &mut sink);
    for (_, p) in std::mem::take(&mut sink.0) {
        a.on_packet(&p);
    }
    assert_eq!(a.speakers.len(), 1);
    a.set_mic(true);
    a.talk(None, true);
    a.on_mic_frame(&tone());
    assert_eq!(a.poll_send(20).len(), 1);
}

/// The alias the tests give a player id.
fn uc_alias(p: &str) -> String {
    p.to_uppercase()
}
