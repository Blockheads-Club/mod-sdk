//! End-to-end: clients seal frames, the relay opens, applies policy and re-seals per listener, clients open.

use bhmod_proto::Message;
use bhmod_voice::crypto::{KeyPair, Role};
use bhmod_voice::frame::{Direction, MAX_OPUS_BYTES};
use bhmod_voice::link::{ClientLink, LinkError};
use bhmod_voice::path::{Path, TransportMode};
use bhmod_voice::relay::*;
use bhmod_voice::udp::Datagram;
use bhmod_voice::{PeerId, VoiceMode};

#[derive(Default)]
struct Out(Vec<(PeerId, Path, Vec<u8>)>);

impl Transport for Out {
    fn send(&mut self, to: PeerId, path: Path, packet: Vec<u8>) {
        self.0.push((to, path, packet));
    }
}

struct Rig {
    relay: Relay,
    table: RelayTable,
    clients: Vec<ClientLink>,
}

fn rig(n: u16, mode: TransportMode, port: Option<u16>) -> Rig {
    let mut relay = Relay::new(mode, port);
    let clients = (1..=n)
        .map(|id| {
            let ck = KeyPair::from_secret([id as u8; 32]);
            let sk = KeyPair::from_secret([100 + id as u8; 32]);
            relay.add_peer(id, sk.session(Role::Server, &ck.public()).unwrap(), true);
            ClientLink::new(ck.session(Role::Client, &sk.public()).unwrap(), id, mode, relay.udp_port())
        })
        .collect();
    Rig { relay, table: RelayTable::new(Caps::default()), clients }
}

/// The in-band packet's VOICE_FRAME fields.
fn fields(packet: &[u8]) -> (u64, Vec<u8>) {
    match Message::decode(packet).unwrap() {
        Message::VoiceFrame(f) => (f.nonce, f.sealed),
        other => panic!("{other:?}"),
    }
}

impl Rig {
    fn client(&mut self, id: PeerId) -> &mut ClientLink {
        &mut self.clients[usize::from(id) - 1]
    }

    /// Client `from` talks; returns the relay's result and what it sent.
    fn talk(&mut self, now: u64, from: PeerId, mode: VoiceMode, opus: &[u8]) -> (Result<Forwarded, Drop>, Out) {
        let (path, pkt) = self.client(from).send(mode, opus, 0.0).unwrap();
        assert_eq!(path, Path::InBand);
        let (n, sealed) = fields(&pkt);
        let mut out = Out::default();
        let r = self.relay.receive(now, from, n, &sealed, &self.table, 0, &mut out);
        (r, out)
    }
}

#[test]
fn routes_gain_pan_muffle() {
    let mut r = rig(4, TransportMode::Auto, None);
    r.table.set_speaker_gain(1, 150);
    r.table.set_routes(
        1,
        vec![
            Route { listener: 2, modes: VoiceMode::Proximity.bit(), gain: 0.5, pan: -0.5, muffle: 0.2 },
            Route { listener: 2, ..Route::new(2) },
            Route::new(3),
            Route::new(1),  // self: skipped
            Route::new(99), // unknown: skipped
        ],
    );
    let (res, out) = r.talk(0, 1, VoiceMode::Proximity, &[7; 60]);
    assert_eq!(res, Ok(Forwarded { started: true, copies: 2 }));
    assert_eq!(out.0.len(), 2);
    let (to, path, pkt) = &out.0[0];
    assert_eq!((*to, *path), (2, Path::InBand));
    let (n, sealed) = fields(pkt);
    let f = r.client(2).open(n, &sealed).unwrap();
    assert_eq!((f.speaker, f.mode, f.seq, f.gain, f.pan, f.muffle), (1, VoiceMode::Proximity, 0, 75, -64, 51));
    assert_eq!(f.opus, vec![7; 60]);
    // The wrong client cannot open it.
    assert!(matches!(r.client(3).open(n, &sealed), Err(LinkError::Crypto(_))));
    let (_, _, pkt3) = &out.0[1];
    let (n3, s3) = fields(pkt3);
    assert_eq!(r.client(3).open(n3, &s3).unwrap().gain, 150);

    // Global: the proximity-only route does not match; the second route to 2 does.
    let (res, out) = r.talk(10, 1, VoiceMode::Global, &[1; 10]);
    assert_eq!(res, Ok(Forwarded { started: false, copies: 2 }));
    let (n, sealed) = fields(&out.0[0].2);
    let f = r.client(2).open(n, &sealed).unwrap();
    assert_eq!((f.seq, f.gain, f.pan, f.muffle), (1, 150, 0, 0));
    assert_eq!(r.table.routes(1).len(), 5);
    r.table.set_routes(1, vec![]);
    assert!(r.table.routes(1).is_empty());
    r.table.set_routes(1, vec![Route::new(2)]);
    r.table.remove_peer(1);
    assert!(r.table.routes(1).is_empty());
}

#[test]
fn caps_and_mutes() {
    let mut r = rig(3, TransportMode::InBand, Some(9000));
    assert_eq!(r.relay.udp_port(), None, "inband never announces a port");
    r.table.caps = Caps { max_bitrate: 16_000, max_speakers: 1, max_frames_per_sec: 50 };
    r.table.set_routes(1, vec![Route::new(2)]);
    r.table.set_routes(2, vec![Route::new(1)]);

    assert_eq!(r.talk(0, 1, VoiceMode::Global, &[0; 81]).0, Err(Drop::TooLarge));
    assert_eq!(r.talk(0, 1, VoiceMode::Global, &[0; 80]).0.unwrap().copies, 1);
    assert_eq!(r.talk(0, 2, VoiceMode::Global, &[0; 10]).0, Err(Drop::TooManySpeakers));
    assert_eq!(r.relay.status(1, &r.table, 100), Status::Talking);
    assert_eq!(r.relay.status(2, &r.table, 100), Status::Ready);
    assert_eq!(r.relay.status(9, &r.table, 100), Status::None);

    // Burst of 10 frames (50 / 5), then the rate limit; time refills it.
    for _ in 1..10 {
        assert!(r.talk(1, 1, VoiceMode::Global, &[0; 10]).0.is_ok());
    }
    assert_eq!(r.talk(1, 1, VoiceMode::Global, &[0; 10]).0, Err(Drop::RateLimited));
    assert!(r.talk(21, 1, VoiceMode::Global, &[0; 10]).0.is_ok());

    // Talk stop after the timeout frees the speaker slot.
    let mut table = r.table.clone();
    assert!(r.relay.tick(100, &mut table).is_empty());
    assert_eq!(r.relay.tick(21 + TALK_TIMEOUT_MS, &mut table), vec![Event::TalkStop(1)]);
    assert!(r.talk(400, 2, VoiceMode::Global, &[0; 10]).0.unwrap().started);

    // Admin mute with expiry, forever mute, unmute.
    r.table.mute(1, Some(1000));
    r.table.mute(3, None);
    assert_eq!(r.talk(500, 1, VoiceMode::Global, &[0; 10]).0, Err(Drop::AdminMuted));
    assert_eq!(r.relay.status(1, &r.table, 500), Status::AdminMuted);
    assert_eq!(r.table.mutes(), vec![(1, Some(1000)), (3, None)]);
    assert!(r.table.is_muted(3, u64::MAX));
    assert_eq!(r.relay.tick(1000, &mut r.table), vec![Event::TalkStop(2), Event::MuteExpired(1)]);
    assert!(!r.table.is_muted(1, 1000));
    r.table.unmute(3);
    assert!(r.table.mutes().is_empty());

    // Self mute and deafen.
    r.relay.set_ctrl(1, true, false);
    assert_eq!(r.relay.status(1, &r.table, 2000), Status::SelfMuted);
    assert_eq!(r.talk(2000, 1, VoiceMode::Global, &[0; 10]).0, Err(Drop::SelfMuted));
    r.relay.set_ctrl(1, false, false);
    r.relay.set_ctrl(2, false, true);
    r.relay.set_ctrl(42, true, true); // unknown: ignored
    assert_eq!(r.relay.status(2, &r.table, 2000), Status::Deafened);
    let (res, out) = r.talk(2000, 1, VoiceMode::Global, &[0; 10]);
    assert_eq!((res.unwrap().copies, out.0.len()), (0, 0), "deafened listener gets nothing");
    let (_, out) = r.talk(2010, 1, VoiceMode::Broadcast, &[0; 10]);
    assert!(out.0.is_empty(), "broadcast respects deafen by default");
    r.table.broadcast_overrides_deafen = true;
    let (_, out) = r.talk(2020, 1, VoiceMode::Broadcast, &[0; 10]);
    assert_eq!(out.0.len(), 1);
}

#[test]
fn drops() {
    let mut r = rig(2, TransportMode::Auto, None);
    let mut out = Out::default();
    let (_, pkt) = r.client(1).send(VoiceMode::Global, &[1; 5], 0.0).unwrap();
    let (n, sealed) = fields(&pkt);
    assert_eq!(r.relay.receive(0, 7, n, &sealed, &r.table, 0, &mut out), Err(Drop::UnknownPeer));
    assert!(r.relay.receive(0, 1, n, &sealed, &r.table, 0, &mut out).is_ok());
    assert_eq!(r.relay.receive(0, 1, n, &sealed, &r.table, 0, &mut out), Err(Drop::Crypto), "replay");
    assert!(r.relay.has_peer(1));
    r.relay.remove_peer(1);
    assert!(!r.relay.has_peer(1));

    // A sealed but malformed plaintext (the client link never sends one; seal by hand).
    let ck = KeyPair::from_secret([50; 32]);
    let sk = KeyPair::from_secret([51; 32]);
    let mut cs = ck.session(Role::Client, &sk.public()).unwrap();
    r.relay.add_peer(5, sk.session(Role::Server, &ck.public()).unwrap(), true);
    let (n, sealed) = cs.seal(&[9, 0, 0]).unwrap();
    assert_eq!(r.relay.receive(0, 5, n, &sealed, &r.table, 0, &mut out), Err(Drop::Malformed));
    assert!(out.0.is_empty());

    // Opus over the hard cap cannot be sent at all.
    assert!(r.client(2).send(VoiceMode::Global, &[0; MAX_OPUS_BYTES + 1], 0.0).is_none());

    // The client rejects a malformed relayed plaintext.
    let ck = KeyPair::from_secret([60; 32]);
    let sk = KeyPair::from_secret([61; 32]);
    let mut ss = sk.session(Role::Server, &ck.public()).unwrap();
    let mut link = ClientLink::new(ck.session(Role::Client, &sk.public()).unwrap(), 3, TransportMode::Auto, None);
    let (n, sealed) = ss.seal(&[1, 2]).unwrap();
    assert!(matches!(link.open(n, &sealed), Err(LinkError::Frame(_))));
    assert_eq!(link.peer(), 3);
}

#[test]
fn udp_fallback() {
    let mut r = rig(2, TransportMode::Auto, Some(15152));
    assert_eq!(r.relay.udp_port(), Some(15152));
    r.table.set_routes(1, vec![Route::new(2)]);
    let mut out = Out::default();

    // Bad tokens, unknown peers and non-auth junk.
    let bad = Datagram::Auth { peer: 1, token: [0; 32] }.encode();
    assert_eq!(r.relay.receive_udp(0, &bad, &r.table, 0, &mut out), Err(Drop::Crypto));
    let unknown = Datagram::Auth { peer: 9, token: [0; 32] }.encode();
    assert_eq!(r.relay.receive_udp(0, &unknown, &r.table, 0, &mut out), Err(Drop::UnknownPeer));
    assert_eq!(r.relay.receive_udp(0, &[1], &r.table, 0, &mut out), Err(Drop::Malformed));
    let ok = Datagram::AuthOk { peer: 1 }.encode();
    assert_eq!(r.relay.receive_udp(0, &ok, &r.table, 0, &mut out), Err(Drop::Malformed));
    let early = Datagram::Frame { peer: 1, nonce: 1, sealed: vec![0; 16] }.encode();
    assert_eq!(r.relay.receive_udp(0, &early, &r.table, 0, &mut out), Err(Drop::NotOnUdp));
    let ghost = Datagram::Frame { peer: 9, nonce: 1, sealed: vec![0; 16] }.encode();
    assert_eq!(r.relay.receive_udp(0, &ghost, &r.table, 0, &mut out), Err(Drop::UnknownPeer));

    // Both clients authenticate.
    for id in [1, 2] {
        let auth = r.client(id).auth_datagram();
        let UdpReceived::Auth { peer, reply } = r.relay.receive_udp(0, &auth, &r.table, 0, &mut out).unwrap() else {
            panic!()
        };
        assert_eq!(peer, id);
        assert!(r.client(id).receive_udp(&reply).is_none());
    }
    // Another peer's AUTH_OK or junk is ignored by a client.
    assert!(r.client(1).receive_udp(&Datagram::AuthOk { peer: 2 }.encode()).is_none());
    assert!(r.client(1).receive_udp(&[0]).is_none());

    // auto still prefers in-band while it is healthy...
    let (path, pkt) = r.client(1).send(VoiceMode::Global, &[3; 40], 0.0).unwrap();
    assert_eq!(path, Path::InBand);
    let (n, sealed) = fields(&pkt);
    r.relay.receive(0, 1, n, &sealed, &r.table, 0, &mut out).unwrap();
    assert_eq!(out.0.pop().unwrap().1, Path::InBand);
    // ...and switches when the server lags (event once), or the client measures loss.
    let (_, pkt) = r.client(1).send(VoiceMode::Global, &[3; 40], 0.0).unwrap();
    let (n, sealed) = fields(&pkt);
    r.relay.receive(20, 1, n, &sealed, &r.table, 50, &mut out).unwrap();
    let (to, path, dgram) = out.0.pop().unwrap();
    assert_eq!((to, path), (2, Path::Udp));
    let mut table = r.table.clone();
    assert_eq!(r.relay.tick(20, &mut table), vec![Event::PathChanged(2, Path::Udp)]);
    let f = r.client(2).receive_udp(&dgram).unwrap().unwrap();
    assert_eq!(f.opus, vec![3; 40]);
    assert!(r.client(1).receive_udp(&dgram).is_none(), "addressed to peer 2");

    let (path, dgram) = r.client(1).send(VoiceMode::Global, &[4; 40], 0.5).unwrap();
    assert_eq!(path, Path::Udp);
    let UdpReceived::Frame(fw) = r.relay.receive_udp(40, &dgram, &r.table, 0, &mut out).unwrap() else { panic!() };
    assert_eq!(fw.copies, 1);

    // Big frames go UDP directly; client 2 cannot send in-band at all.
    r.client(2).selector_mut().inband_capable = false;
    r.table.set_routes(2, vec![Route::new(1)]);
    let (path, _) = r.client(2).send(VoiceMode::Global, &[0; 10], 0.0).unwrap();
    assert_eq!(path, Path::Udp);
}

#[test]
fn lost_listener_path_and_exhaustion() {
    // A listener whose path cannot take the frame (in-band only, frame too big for in-band) gets nothing.
    let mut r = rig(2, TransportMode::InBand, None);
    r.table.caps.max_bitrate = 128_000;
    r.table.set_routes(1, vec![Route::new(2)]);
    let ck = KeyPair::from_secret([1; 32]);
    let sk = KeyPair::from_secret([101; 32]);
    let mut cs = ck.session(Role::Client, &sk.public()).unwrap();
    let plain = bhmod_voice::frame::ClientFrame { mode: VoiceMode::Global, seq: 0, opus: vec![0; 300] }.encode();
    let (n, sealed) = cs.seal(&plain).unwrap();
    let mut out = Out::default();
    let fw = r.relay.receive(0, 1, n, &sealed, &r.table, 0, &mut out).unwrap();
    assert_eq!((fw.copies, out.0.len()), (0, 0));
    assert_eq!(bhmod_voice::frame::inband_packet_len(Direction::Down, 300), 334);
}

#[test]
fn table_helpers() {
    let mut t = RelayTable::default();
    t.set_speaker_gain(1, 500);
    assert_eq!(t.speaker_gain(1), MAX_SPEAKER_GAIN);
    t.set_speaker_gain(1, 100);
    assert_eq!(t.speaker_gain(1), 100);
    assert_eq!(gain_byte(200, 1.0), 200);
    assert_eq!(gain_byte(200, 2.0), 200);
    assert_eq!(gain_byte(100, -1.0), 0);
    assert_eq!(pan_byte(-3.0), -127);
    assert_eq!(muffle_byte(0.5), 128);
    for (s, w) in [
        (Status::None, "none"),
        (Status::Ready, "ready"),
        (Status::Talking, "talking"),
        (Status::SelfMuted, "self_muted"),
        (Status::Deafened, "deafened"),
        (Status::AdminMuted, "admin_muted"),
    ] {
        assert_eq!(s.as_str(), w);
    }
    fn send_sync<T: Send + Sync>() {}
    send_sync::<RelayTable>();
    send_sync::<Relay>();
    send_sync::<ClientLink>();
}

#[test]
fn measured_wire_sizes() {
    // Real sealed packets for 20 ms Opus at 16/24/32 kbps (40/60/80 bytes), compared with the size helpers.
    use bhmod_voice::frame::{inband_ip_len, opus_bytes, udp_datagram_len, ENET_OVERHEAD, IP_UDP_OVERHEAD};
    let mut r = rig(2, TransportMode::Auto, None);
    r.table.caps.max_bitrate = 32_000;
    r.table.set_routes(1, vec![Route::new(2)]);
    for (rate, up_ip, down_ip) in [(16_000, 109, 114), (24_000, 129, 134), (32_000, 149, 154)] {
        let opus = vec![0u8; opus_bytes(rate)];
        let (_, up) = r.client(1).send(VoiceMode::Global, &opus, 0.0).unwrap();
        let (n, sealed) = fields(&up);
        let mut out = Out::default();
        r.relay.receive(0, 1, n, &sealed, &r.table, 0, &mut out).unwrap();
        let down = &out.0[0].2;
        assert_eq!(up.len() + ENET_OVERHEAD + IP_UDP_OVERHEAD, up_ip);
        assert_eq!(down.len() + ENET_OVERHEAD + IP_UDP_OVERHEAD, down_ip);
        assert_eq!(inband_ip_len(Direction::Up, opus.len()), up_ip);
        assert_eq!(inband_ip_len(Direction::Down, opus.len()), down_ip);
        assert_eq!(udp_datagram_len(Direction::Up, opus.len()), up.len() + 1);
    }
}
