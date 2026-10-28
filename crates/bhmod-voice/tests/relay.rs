//! End-to-end over the voice UDP port: clients authenticate and seal frames, the relay opens, applies policy and
//! re-seals per listener, clients open.

use bhmod_voice::crypto::{KeyPair, Role, Session};
use bhmod_voice::frame::{Direction, MAX_OPUS_BYTES};
use bhmod_voice::link::{ClientLink, LinkError};
use bhmod_voice::relay::*;
use bhmod_voice::udp::Datagram;
use bhmod_voice::{PeerId, VoiceMode};

#[derive(Default)]
struct Out(Vec<(PeerId, Vec<u8>)>);

impl Transport for Out {
    fn send(&mut self, to: PeerId, datagram: Vec<u8>) {
        self.0.push((to, datagram));
    }
}

struct Rig {
    relay: Relay,
    table: RelayTable,
    clients: Vec<ClientLink>,
}

fn keys(id: u8) -> (KeyPair, KeyPair) {
    (KeyPair::from_secret([id; 32]), KeyPair::from_secret([100 + id; 32]))
}

/// `n` clients with sessions; the first `authed` of them authenticated on the UDP port.
fn rig(n: u16, authed: u16) -> Rig {
    let mut relay = Relay::new(Some(15152));
    let clients = (1..=n)
        .map(|id| {
            let (ck, sk) = keys(id as u8);
            relay.add_peer(id, sk.session(Role::Server, &ck.public()).unwrap());
            ClientLink::new(ck.session(Role::Client, &sk.public()).unwrap(), id)
        })
        .collect();
    let mut r = Rig { relay, table: RelayTable::new(Caps::default()), clients };
    for id in 1..=authed {
        r.auth(id);
    }
    r
}

impl Rig {
    fn client(&mut self, id: PeerId) -> &mut ClientLink {
        &mut self.clients[usize::from(id) - 1]
    }

    fn auth(&mut self, id: PeerId) {
        let auth = self.client(id).auth_datagram();
        let mut out = Out::default();
        let UdpReceived::Auth { peer, reply } = self.relay.receive_udp(0, &auth, &self.table, &mut out).unwrap() else {
            panic!()
        };
        assert_eq!(peer, id);
        assert!(!self.client(id).authed());
        assert!(self.client(id).receive_udp(&reply).is_none());
        assert!(self.client(id).authed());
    }

    /// Client `from` talks; returns the relay's result and what it sent.
    fn talk(&mut self, now: u64, from: PeerId, mode: VoiceMode, opus: &[u8]) -> (Result<Forwarded, Drop>, Out) {
        let dgram = self.client(from).send(mode, opus).unwrap();
        let mut out = Out::default();
        let r = self.relay.receive_udp(now, &dgram, &self.table, &mut out).map(|r| match r {
            UdpReceived::Frame(f) => f,
            other => panic!("{other:?}"),
        });
        (r, out)
    }
}

#[test]
fn routes_gain_pan_muffle() {
    let mut r = rig(4, 4);
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
    assert_eq!(res, Ok(Forwarded { started: true, copies: 2, mode: VoiceMode::Proximity }));
    assert_eq!(out.0.len(), 2);
    let (to, dgram) = &out.0[0];
    assert_eq!(*to, 2);
    let f = r.client(2).receive_udp(dgram).unwrap().unwrap();
    assert_eq!((f.speaker, f.mode, f.seq, f.gain, f.pan, f.muffle), (1, VoiceMode::Proximity, 0, 75, -64, 51));
    assert_eq!(f.opus, vec![7; 60]);
    // Addressed to peer 2: client 3 ignores it; re-addressed, it cannot open it.
    assert!(r.client(3).receive_udp(dgram).is_none());
    let Some(Datagram::Frame { nonce, sealed, .. }) = Datagram::decode(dgram, 1200) else { panic!() };
    let forged = Datagram::Frame { peer: 3, nonce, sealed }.encode();
    assert!(matches!(r.client(3).receive_udp(&forged), Some(Err(LinkError::Crypto(_)))));
    let f3 = r.client(3).receive_udp(&out.0[1].1).unwrap().unwrap();
    assert_eq!(f3.gain, 150);

    // Global: the proximity-only route does not match; the second route to 2 does.
    let (res, out) = r.talk(10, 1, VoiceMode::Global, &[1; 10]);
    assert_eq!(res, Ok(Forwarded { started: false, copies: 2, mode: VoiceMode::Global }));
    let f = r.client(2).receive_udp(&out.0[0].1).unwrap().unwrap();
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
    let mut r = rig(3, 3);
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
fn auth_and_drops() {
    let mut r = rig(2, 0);
    assert_eq!(r.relay.udp_port(), Some(15152));
    let mut out = Out::default();

    // Bad tokens, unknown peers and non-auth junk.
    let bad = Datagram::Auth { peer: 1, token: [0; 32] }.encode();
    assert_eq!(r.relay.receive_udp(0, &bad, &r.table, &mut out), Err(Drop::Crypto));
    let unknown = Datagram::Auth { peer: 9, token: [0; 32] }.encode();
    assert_eq!(r.relay.receive_udp(0, &unknown, &r.table, &mut out), Err(Drop::UnknownPeer));
    assert_eq!(r.relay.receive_udp(0, &[1], &r.table, &mut out), Err(Drop::Malformed));
    let ok = Datagram::AuthOk { peer: 1 }.encode();
    assert_eq!(r.relay.receive_udp(0, &ok, &r.table, &mut out), Err(Drop::Malformed));
    let early = Datagram::Frame { peer: 1, nonce: 1, sealed: vec![0; 16] }.encode();
    assert_eq!(r.relay.receive_udp(0, &early, &r.table, &mut out), Err(Drop::NotOnUdp));
    let ghost = Datagram::Frame { peer: 9, nonce: 1, sealed: vec![0; 16] }.encode();
    assert_eq!(r.relay.receive_udp(0, &ghost, &r.table, &mut out), Err(Drop::UnknownPeer));

    // A client sends nothing before AUTH_OK; another peer's AUTH_OK or junk is ignored.
    assert!(r.client(1).send(VoiceMode::Global, &[1; 5]).is_none());
    assert!(r.client(1).receive_udp(&Datagram::AuthOk { peer: 2 }.encode()).is_none());
    assert!(r.client(1).receive_udp(&[0]).is_none());
    assert!(!r.client(1).authed());
    r.auth(1);

    // A listener that did not authenticate yet gets nothing; after AUTH it does.
    r.table.set_routes(1, vec![Route::new(2)]);
    let (res, out) = r.talk(0, 1, VoiceMode::Global, &[3; 40]);
    assert_eq!((res.unwrap().copies, out.0.len()), (0, 0));
    r.auth(2);
    let (res, mut out) = r.talk(20, 1, VoiceMode::Global, &[3; 40]);
    assert_eq!(res.unwrap().copies, 1);
    assert_eq!(r.client(2).receive_udp(&out.0[0].1).unwrap().unwrap().opus, vec![3; 40]);

    // Replay.
    let dgram = r.client(1).send(VoiceMode::Global, &[1; 5]).unwrap();
    assert!(r.relay.receive_udp(40, &dgram, &r.table, &mut out).is_ok());
    assert_eq!(r.relay.receive_udp(40, &dgram, &r.table, &mut out), Err(Drop::Crypto), "replay");
    assert!(r.relay.has_peer(1));
    r.relay.remove_peer(1);
    assert!(!r.relay.has_peer(1));

    // A sealed but malformed plaintext (the client link never sends one; seal by hand).
    let (ck, sk) = keys(50);
    let mut cs: Session = ck.session(Role::Client, &sk.public()).unwrap();
    r.relay.add_peer(5, sk.session(Role::Server, &ck.public()).unwrap());
    let auth = Datagram::Auth { peer: 5, token: cs.udp_token(5) }.encode();
    assert!(r.relay.receive_udp(0, &auth, &r.table, &mut out).is_ok());
    let (nonce, sealed) = cs.seal(&[9, 0, 0]).unwrap();
    let bad = Datagram::Frame { peer: 5, nonce, sealed }.encode();
    assert_eq!(r.relay.receive_udp(0, &bad, &r.table, &mut out), Err(Drop::Malformed));

    // The client rejects a malformed relayed plaintext.
    let (ck, sk) = keys(60);
    let mut ss = sk.session(Role::Server, &ck.public()).unwrap();
    let mut link = ClientLink::new(ck.session(Role::Client, &sk.public()).unwrap(), 3);
    let (nonce, sealed) = ss.seal(&[1, 2]).unwrap();
    let dgram = Datagram::Frame { peer: 3, nonce, sealed }.encode();
    assert!(matches!(link.receive_udp(&dgram), Some(Err(LinkError::Frame(_)))));
    assert_eq!(link.peer(), 3);
}

#[test]
fn no_port_refuses_auth() {
    let mut r = rig(1, 0);
    r.relay.set_udp_port(None);
    assert_eq!(r.relay.udp_port(), None);
    let auth = r.client(1).auth_datagram();
    assert_eq!(r.relay.receive_udp(0, &auth, &r.table, &mut Out::default()), Err(Drop::Crypto));
}

#[test]
fn oversized_opus_is_dropped() {
    let mut r = rig(2, 2);
    r.table.caps.max_bitrate = 1_000_000;
    r.table.set_routes(1, vec![Route::new(2)]);
    // The client seals it; the relay refuses the plaintext over the hard cap.
    let (res, out) = r.talk(0, 1, VoiceMode::Global, &[0; MAX_OPUS_BYTES + 1]);
    assert_eq!((res, out.0.len()), (Err(Drop::Malformed), 0));
    assert!(r.talk(0, 1, VoiceMode::Global, &[0; MAX_OPUS_BYTES]).0.is_ok());
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
    // Real datagrams for 20 ms Opus at 16/24/32 kbps (40/60/80 bytes), compared with the size helpers.
    use bhmod_voice::frame::{opus_bytes, udp_datagram_len, udp_ip_len, IP_UDP_OVERHEAD};
    let mut r = rig(2, 2);
    r.table.caps.max_bitrate = 32_000;
    r.table.set_routes(1, vec![Route::new(2)]);
    for (rate, up_ip, down_ip) in [(16_000, 98, 103), (24_000, 118, 123), (32_000, 138, 143)] {
        let opus = vec![0u8; opus_bytes(rate)];
        let up = r.client(1).send(VoiceMode::Global, &opus).unwrap();
        let mut out = Out::default();
        r.relay.receive_udp(0, &up, &r.table, &mut out).unwrap();
        let down = &out.0[0].1;
        assert_eq!(up.len() + IP_UDP_OVERHEAD, up_ip);
        assert_eq!(down.len() + IP_UDP_OVERHEAD, down_ip);
        assert_eq!(udp_ip_len(Direction::Up, opus.len()), up_ip);
        assert_eq!(udp_datagram_len(Direction::Down, opus.len()), down.len());
    }
}
