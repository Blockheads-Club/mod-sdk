use std::sync::Arc;

use bhmod_core::voice::{Caps as CoreCaps, Route as CoreRoute};
use bhmod_proto::{VoiceCtrl, VoiceFrame};

use super::*;
use crate::client::{Outgoing, VoiceClient};
use crate::codec::TestCodec;

/// Everything a sink got, as `(to, kind, packet)`; kind 0 reliable, 1 unsequenced, 2 udp (to = peer id).
#[derive(Default)]
struct Sink(Vec<(String, u8, Vec<u8>)>);

impl ServerSink for Sink {
    fn reliable(&mut self, player: &str, packet: Vec<u8>) {
        self.0.push((player.into(), 0, packet));
    }
    fn unsequenced(&mut self, player: &str, packet: Vec<u8>) {
        self.0.push((player.into(), 1, packet));
    }
    fn udp(&mut self, peer: PeerId, datagram: Vec<u8>) {
        self.0.push((peer.to_string(), 2, datagram));
    }
}

impl Sink {
    fn take(&mut self) -> Vec<(String, u8, Vec<u8>)> {
        std::mem::take(&mut self.0)
    }
    fn states(&self, to: &str) -> Vec<VoiceState> {
        self.0
            .iter()
            .filter(|(p, k, _)| p == to && *k == 0)
            .filter_map(|(_, _, b)| match Message::decode(b).unwrap() {
                Message::VoiceState(s) => Some(s),
                _ => None,
            })
            .collect()
    }
}

fn features(unsequenced: bool) -> Vec<String> {
    let mut f = vec!["voice".to_string(), "settings".to_string()];
    if unsequenced {
        f.push("voice-unsequenced".into());
    }
    f
}

fn server() -> VoiceServer {
    let mut n = 0u8;
    VoiceServer::with_keys(Box::new(move || {
        n += 1;
        Some(KeyPair::from_secret([200 - n; 32]))
    }))
}

fn client(seed: u8) -> (VoiceClient, Vec<u8>) {
    let mut c = VoiceClient::new(Arc::new(TestCodec), true);
    let key = c.start_with(KeyPair::from_secret([seed; 32]));
    (c, key)
}

/// Joins `name` with voice and runs the key exchange; returns the client.
fn connect(s: &mut VoiceServer, sink: &mut Sink, name: &str, seed: u8) -> VoiceClient {
    connect_with(s, sink, name, seed, true)
}

fn connect_with(s: &mut VoiceServer, sink: &mut Sink, name: &str, seed: u8, unsequenced: bool) -> VoiceClient {
    s.join(name, &features(unsequenced));
    let mut c = VoiceClient::new(Arc::new(TestCodec), unsequenced);
    let key = c.start_with(KeyPair::from_secret([seed; 32]));
    s.receive(0, name, &Message::decode(&key).unwrap(), 0, sink);
    for (to, _, b) in sink.take() {
        assert_eq!(to, name);
        c.on_packet(&b);
    }
    assert!(c.connected());
    c
}

fn route(to: &str) -> CoreRoute {
    CoreRoute { listener: to.into(), modes: 0, gain: 1.0, pan: 0.0, muffle: 0.0 }
}

#[test]
fn handshake_states_and_relay() {
    let mut s = server();
    let mut sink = Sink::default();
    // a vanilla player and a modded one without voice: ops are ignored
    s.join("vanilla", &[]);
    let (_, key) = client(9);
    s.receive(0, "vanilla", &Message::decode(&key).unwrap(), 0, &mut sink);
    s.receive(0, "nobody", &Message::decode(&key).unwrap(), 0, &mut sink);
    assert!(sink.take().is_empty());

    let mut a = connect(&mut s, &mut sink, "a", 1);
    let mut b = connect(&mut s, &mut sink, "b", 2);
    assert_eq!((s.peer_of("a"), s.peer_of("b"), s.peer_of("vanilla")), (Some(1), Some(2), None));
    assert_eq!(s.state("a"), State::Ready);
    assert_eq!(s.state("vanilla"), State::None);

    // first tick: every player's state is new; a and b get a delta (b's speaker id goes to a)
    s.tick(10, &mut sink);
    let ev = s.take_events();
    assert!(ev.contains(&VoiceEvent::State { player: "vanilla".into(), state: State::None }));
    assert!(ev.contains(&VoiceEvent::State { player: "a".into(), state: State::Ready }));
    for (to, _, pkt) in sink.take() {
        match to.as_str() {
            "a" => a.on_packet(&pkt),
            "b" => b.on_packet(&pkt),
            other => panic!("{other}"),
        }
    }
    assert_eq!(a.state("b"), State::Ready);
    assert_eq!(a.state("vanilla"), State::None);

    // a talks to b
    s.set_allowed("a", Mode::mask(&[Mode::Proximity]), Activation::mask(&[Activation::Ptt]));
    s.set_allowed("ghost", 1, 1);
    s.set_routes("a", &[route("b"), route("ghost")]);
    s.tick(20, &mut sink);
    for (_, _, pkt) in sink.take() {
        a.on_packet(&pkt);
    }
    a.set_mic(true);
    a.talk(None, true);
    a.on_mic_frame(&vec![0.25; crate::FRAME_SAMPLES]);
    let sent = a.poll_send(30);
    let [Outgoing::Unsequenced(pkt)] = &sent[..] else { panic!("{sent:?}") };
    s.receive(30, "a", &Message::decode(pkt).unwrap(), 0, &mut sink);
    assert_eq!(s.take_events(), vec![VoiceEvent::TalkStart { player: "a".into(), mode: Mode::Proximity }]);
    let out = sink.take();
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].0.as_str(), out[0].1), ("b", 1));
    b.on_packet(&out[0].2);
    assert_eq!(b.take_events().last(), Some(&VoiceEvent::TalkStart { player: "a".into(), mode: Mode::Proximity }));
    assert_eq!(s.state("a"), State::Talking);
    s.tick(40, &mut sink);
    assert!(s.take_events().contains(&VoiceEvent::State { player: "a".into(), state: State::Talking }));
    // silence: talk-stop with the spurt's mode
    s.tick(1_000, &mut sink);
    let ev = s.take_events();
    assert!(ev.contains(&VoiceEvent::TalkStop { player: "a".into(), mode: Mode::Proximity }));

    // VOICE_CTRL, unknown ops, frames before a session
    sink.take();
    s.receive(1_001, "a", &Message::VoiceCtrl(VoiceCtrl { self_mute: true, deafen: false }), 0, &mut sink);
    assert_eq!(s.state("a"), State::SelfMuted);
    s.receive(1_001, "a", &Message::ListRequest, 0, &mut sink);
    s.join("late", &features(false));
    s.receive(1_001, "late", &Message::VoiceCtrl(VoiceCtrl::default()), 0, &mut sink);
    s.receive(1_001, "late", &Message::VoiceFrame(VoiceFrame { nonce: 1, sealed: vec![0; 20] }), 0, &mut sink);
    s.receive(1_001, "a", &Message::VoiceFrame(VoiceFrame { nonce: 1, sealed: vec![0; 20] }), 0, &mut sink);
    assert!(sink.take().is_empty());

    // leaving: others get `gone`, routes naming the player are dropped
    s.leave("b");
    s.leave("b");
    s.tick(1_100, &mut sink);
    let st = sink.states("a");
    assert!(st.iter().any(|v| v.players.get("b").map(String::as_str) == Some("gone")));
}

#[test]
fn policy_mutes_gains_caps_icons() {
    let mut s = server();
    let mut sink = Sink::default();
    s.mute("a", Some(5_000));
    s.set_speaker_gain("a", 50);
    let _a = connect(&mut s, &mut sink, "a", 1);
    assert_eq!(s.state("a"), State::AdminMuted, "a mute before the session applies to it");
    assert_eq!(s.table.speaker_gain(1), 50);
    s.set_speaker_gain("a", 150);
    assert_eq!(s.table.speaker_gain(1), 150);
    assert!(s.unmute("a"));
    assert!(!s.unmute("a"));
    assert!(!s.unmute("nobody"));
    s.mute("a", None);
    s.mute("off", Some(10));
    s.tick(20, &mut sink);
    assert!(s.mutes.contains_key("a") && !s.mutes.contains_key("off"), "expired mutes of absent players go");
    // a mute that ends while the player is online
    s.unmute("a");
    s.mute("a", Some(100));
    s.tick(200, &mut sink);
    assert_eq!(s.state("a"), State::Ready);
    // reconnect keeps the mute (left while muted)
    s.mute("a", None);
    s.leave("a");
    let _a = connect(&mut s, &mut sink, "a", 3);
    assert_eq!(s.state("a"), State::AdminMuted);

    s.set_caps(CoreCaps { max_bitrate: 16_000, max_speakers: 2, max_frames_per_sec: 50 });
    assert_eq!(s.table.caps, Caps { max_bitrate: 16_000, max_speakers: 2, max_frames_per_sec: 50 });
    s.set_broadcast_overrides_deafen(true);
    assert!(s.table.broadcast_overrides_deafen);
    let hidden = Icons { deafen: false, ..Icons::default() };
    s.set_icons(hidden);
    s.set_icons(hidden);
    assert_eq!(s.icons(), hidden);
    s.tick(300, &mut sink);
    let st = sink.states("a");
    let last = st.last().unwrap();
    assert_eq!(last.icons.get("deafen"), Some(&false));
    assert_eq!(last.bitrate, Some(16_000), "caps change the client's bitrate");
    s.tick(400, &mut sink);
    sink.take();
    s.tick(500, &mut sink);
    assert!(sink.take().is_empty(), "nothing changed: no packet");
}

#[test]
fn udp_transport() {
    let mut s = server();
    let mut sink = Sink::default();
    s.set_transport(TransportConfig { mode: Transport::Udp, udp_port: Some(15151) });
    assert_eq!(s.transport().udp_port, Some(15151));
    assert_eq!(s.udp_port(), Some(15151));
    let mut a = connect_with(&mut s, &mut sink, "a", 1, false);
    let mut b = connect_with(&mut s, &mut sink, "b", 2, false);
    assert_eq!(a.udp_port(), Some(15151));
    s.set_routes("a", &[route("b")]);
    // AUTH from both
    for (c, peer) in [(&mut a, 1), (&mut b, 2)] {
        let sent = c.poll_send(0);
        let [Outgoing::Udp(auth)] = &sent[..] else { panic!("{sent:?}") };
        assert_eq!(s.receive_udp(0, auth, &|_| true, 0, &mut sink), Some(peer));
        let out = sink.take();
        assert_eq!(out[0].1, 2);
        c.on_udp(&out[0].2);
    }
    assert!(a.poll_send(1).is_empty(), "keepalive only every 15 s");
    s.tick(5, &mut sink);
    for (to, _, p) in sink.take() {
        if to == "a" {
            a.on_packet(&p)
        } else {
            b.on_packet(&p)
        }
    }
    s.set_allowed("a", 1, 1);
    s.tick(6, &mut sink);
    for (_, _, p) in sink.take() {
        a.on_packet(&p);
    }
    a.set_mic(true);
    a.talk(Some(Mode::Proximity), true);
    a.on_mic_frame(&vec![0.1; crate::FRAME_SAMPLES]);
    let sent = a.poll_send(10);
    let [Outgoing::Udp(frame)] = &sent[..] else { panic!("{sent:?}") };
    // from an address that is not a's: dropped
    assert_eq!(s.receive_udp(10, frame, &|_| false, 0, &mut sink), None);
    assert!(sink.take().is_empty());
    assert_eq!(s.receive_udp(10, frame, &|_| true, 0, &mut sink), None);
    assert_eq!(s.take_events()[2..], vec![VoiceEvent::TalkStart { player: "a".into(), mode: Mode::Proximity }]);
    let out = sink.take();
    assert_eq!((out[0].0.as_str(), out[0].1), ("2", 2));
    b.on_udp(&out[0].2);
    // garbage
    assert_eq!(s.receive_udp(10, &[0xff], &|_| true, 0, &mut sink), None);
}

#[test]
fn keys_and_helpers() {
    // no randomness: the exchange is refused; a low-order public key too
    let mut s = VoiceServer::with_keys(Box::new(|| None));
    let mut sink = Sink::default();
    s.join("a", &features(true));
    let (_, key) = client(1);
    s.receive(0, "a", &Message::decode(&key).unwrap(), 0, &mut sink);
    assert!(sink.take().is_empty());
    let mut s = server();
    s.join("a", &features(true));
    s.receive(0, "a", &Message::VoiceKey(VoiceKey { public: [0; 32], peer: None, udp_port: None }), 0, &mut sink);
    assert!(sink.take().is_empty() && s.peer_of("a").is_none());
    // a rekey keeps the speaker id; ids wrap
    let _ = connect(&mut s, &mut sink, "a", 1);
    let (_, key) = client(2);
    s.receive(0, "a", &Message::decode(&key).unwrap(), 0, &mut sink);
    sink.take();
    assert_eq!(s.peer_of("a"), Some(1));
    s.next_peer = u16::MAX;
    let _ = connect(&mut s, &mut sink, "b", 3);
    assert_eq!((s.peer_of("b"), s.next_peer), (Some(u16::MAX), 1));
    assert!(VoiceServer::new().udp_port().is_none());

    for (m, c) in VoiceMode::ALL.iter().zip(Mode::ALL) {
        assert_eq!(mode_of(*m), *c);
    }
    let all = [Status::None, Status::Ready, Status::Talking, Status::SelfMuted, Status::Deafened, Status::AdminMuted];
    for st in all {
        assert_eq!(state_of(st).wire(), st.as_str());
    }
    for t in Transport::ALL {
        assert_eq!(TransportMode::parse(t.name()), Some(transport_mode(*t)));
    }
}

#[test]
fn shared_server_is_a_voice_host() {
    let shared = SharedVoiceServer::new(server());
    let h: &dyn VoiceHost = &shared;
    shared.lock().join("a", &features(true));
    assert_eq!(h.state("a"), Ok(State::None));
    assert_eq!(h.icons(), Ok(Icons::default()));
    h.set_routes("a", &[route("b")]).unwrap();
    h.mute("a", None).unwrap();
    assert_eq!(h.unmute("a"), Ok(true));
    h.set_speaker_gain("a", 10).unwrap();
    h.set_caps(CoreCaps::default()).unwrap();
    h.set_transport(TransportConfig::default()).unwrap();
    h.set_allowed("a", 3, 1).unwrap();
    h.set_icons(Icons::default()).unwrap();
    h.set_broadcast_overrides_deafen(false).unwrap();
    assert_eq!(h.set_master(1), Err(ApiError::Unsupported), "client functions stay unsupported");
    let s = shared.lock();
    assert_eq!((s.players["a"].modes, s.gains["a"]), (3, 10));
}

#[test]
fn random_keys_and_a_poisoned_lock() {
    let mut s = VoiceServer::new();
    let mut sink = Sink::default();
    let _ = connect(&mut s, &mut sink, "a", 1);
    assert_eq!(s.peer_of("a"), Some(1));
    let shared = SharedVoiceServer::new(s);
    let s2 = shared.clone();
    let _ = std::thread::spawn(move || {
        let _g = s2.lock();
        panic!("poison");
    })
    .join();
    assert_eq!(shared.lock().peer_of("a"), Some(1));
}
