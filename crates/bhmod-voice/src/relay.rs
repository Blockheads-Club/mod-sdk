//! The server voice relay. It opens a client's frame, enforces caps and mutes, and seals one copy per listener of
//! the mod's route table. It never decodes Opus, so no decoded audio exists on the server.
//!
//! Two parts, both `Send + Sync` so a host can share them across its game thread and a UDP thread
//! (`Arc<RwLock<RelayTable>>`, `Arc<Mutex<Relay>>`):
//!
//! - [`RelayTable`]: policy written by the voice mod (routes at ~5 Hz, admin mutes, speaker gain, caps).
//! - [`Relay`]: per-peer state (session keys, replay window, rate limit, path, self mute / deafen).
//!
//! Frames arrive and leave only as datagrams of the voice UDP port ([`Relay::receive_udp`]); output goes to a
//! [`Transport`].

use std::collections::HashMap;

use crate::crypto::Session;
use crate::frame::{max_opus_bytes, ClientFrame, ServerFrame};
use crate::udp::Datagram;
use crate::{PeerId, VoiceMode};

/// A speaker counts as talking this long after its last frame.
pub const TALK_TIMEOUT_MS: u64 = 300;
/// Most server speaker gain, percent.
pub const MAX_SPEAKER_GAIN: u16 = 200;

/// One listener of a speaker, as published by the voice mod.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Route {
    /// Who hears.
    pub listener: PeerId,
    /// Modes this route applies to ([`VoiceMode::bit`] mask).
    pub modes: u8,
    /// Proximity / channel gain 0..=1.
    pub gain: f32,
    /// Stereo position -1 (left) ..= 1 (right).
    pub pan: f32,
    /// Muffling through blocks 0 (clear) ..= 1.
    pub muffle: f32,
}

impl Route {
    /// A route in every mode at full gain, centred, clear.
    pub fn new(listener: PeerId) -> Route {
        Route { listener, modes: 0x0f, gain: 1.0, pan: 0.0, muffle: 0.0 }
    }
}

/// Server caps (`quality` settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Bitrate cap (bits per second); frames over [`max_opus_bytes`] of it are dropped.
    pub max_bitrate: u32,
    /// Most speakers talking at once.
    pub max_speakers: usize,
    /// Per-peer frame rate limit.
    pub max_frames_per_sec: u32,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps { max_bitrate: 24_000, max_speakers: 8, max_frames_per_sec: 60 }
    }
}

/// The mod-written policy of the relay.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelayTable {
    /// Caps.
    pub caps: Caps,
    /// Admin broadcast reaches deafened players.
    pub broadcast_overrides_deafen: bool,
    routes: HashMap<PeerId, Vec<Route>>,
    mutes: HashMap<PeerId, Option<u64>>,
    gains: HashMap<PeerId, u16>,
}

impl RelayTable {
    /// An empty table with these caps.
    pub fn new(caps: Caps) -> RelayTable {
        RelayTable { caps, ..RelayTable::default() }
    }

    /// Replaces who hears `speaker` (`bh.voice.set_routes`). For a listener listed twice, the first route whose
    /// modes match the frame wins.
    pub fn set_routes(&mut self, speaker: PeerId, routes: Vec<Route>) {
        if routes.is_empty() {
            self.routes.remove(&speaker);
        } else {
            self.routes.insert(speaker, routes);
        }
    }

    /// Who hears `speaker`.
    pub fn routes(&self, speaker: PeerId) -> &[Route] {
        self.routes.get(&speaker).map_or(&[], Vec::as_slice)
    }

    /// Admin mute until `until` (ms, same clock as `now`), or forever with `None` (`bh.voice.mute`).
    pub fn mute(&mut self, peer: PeerId, until: Option<u64>) {
        self.mutes.insert(peer, until);
    }

    /// Lifts an admin mute.
    pub fn unmute(&mut self, peer: PeerId) {
        self.mutes.remove(&peer);
    }

    /// True while an admin mute holds.
    pub fn is_muted(&self, peer: PeerId, now: u64) -> bool {
        self.mutes.get(&peer).is_some_and(|u| u.is_none_or(|t| now < t))
    }

    /// Removes expired mutes and returns their peers, sorted.
    pub fn prune_mutes(&mut self, now: u64) -> Vec<PeerId> {
        let mut gone: Vec<PeerId> =
            self.mutes.iter().filter(|(_, u)| u.is_some_and(|t| now >= t)).map(|(p, _)| *p).collect();
        gone.sort_unstable();
        gone.iter().for_each(|p| {
            self.mutes.remove(p);
        });
        gone
    }

    /// All admin mutes (for persistence), sorted by peer.
    pub fn mutes(&self) -> Vec<(PeerId, Option<u64>)> {
        let mut v: Vec<_> = self.mutes.iter().map(|(p, u)| (*p, *u)).collect();
        v.sort_unstable();
        v
    }

    /// Server gain for everything `peer` says, percent 0..=200 (clamped; 100 is the default).
    pub fn set_speaker_gain(&mut self, peer: PeerId, percent: u16) {
        let p = percent.min(MAX_SPEAKER_GAIN);
        if p == 100 {
            self.gains.remove(&peer);
        } else {
            self.gains.insert(peer, p);
        }
    }

    /// The server gain of `peer`, percent.
    pub fn speaker_gain(&self, peer: PeerId) -> u16 {
        self.gains.get(&peer).copied().unwrap_or(100)
    }

    /// Drops the routes of a speaker that left. Admin mutes and speaker gains stay (they are persisted per player);
    /// routes naming it as listener are skipped by the relay until the mod rewrites them.
    pub fn remove_peer(&mut self, peer: PeerId) {
        self.routes.remove(&peer);
    }
}

/// The `gain` byte: speaker gain (percent) × route gain (0..=1), clamped to 0..=200.
pub fn gain_byte(speaker_percent: u16, route_gain: f32) -> u8 {
    (f32::from(speaker_percent) * route_gain.clamp(0.0, 1.0)).round().min(200.0) as u8
}

/// The `pan` byte of -1..=1.
pub fn pan_byte(pan: f32) -> i8 {
    (pan.clamp(-1.0, 1.0) * 127.0).round() as i8
}

/// The `muffle` byte of 0..=1.
pub fn muffle_byte(muffle: f32) -> u8 {
    (muffle.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Where relayed frames go: the host's voice UDP socket.
pub trait Transport {
    /// Sends `datagram` to the address `to` authenticated from.
    fn send(&mut self, to: PeerId, datagram: Vec<u8>);
}

/// Why a frame was not relayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    /// No session for that peer.
    UnknownPeer,
    /// Failed to authenticate or replayed.
    Crypto,
    /// Plaintext malformed.
    Malformed,
    /// A UDP frame from a peer that did not authenticate on UDP.
    NotOnUdp,
    /// Admin mute.
    AdminMuted,
    /// The player muted themselves.
    SelfMuted,
    /// Over the bitrate cap.
    TooLarge,
    /// Over the frame rate limit.
    RateLimited,
    /// Too many speakers at once.
    TooManySpeakers,
}

/// A relayed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forwarded {
    /// The speaker was silent before this frame (`voice:talk-start`).
    pub started: bool,
    /// Copies sent (deafened listeners and listeners not yet authenticated on the UDP port get none).
    pub copies: usize,
    /// The frame's talk mode.
    pub mode: VoiceMode,
}

/// Something the host reports to the voice mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A speaker stopped talking (`voice:talk-stop`).
    TalkStop(PeerId),
    /// An admin mute expired.
    MuteExpired(PeerId),
}

/// A peer's voice state (`VOICE_STATE` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No voice (vanilla, no mod, no session).
    None,
    /// Can talk and hear.
    Ready,
    /// Talking now.
    Talking,
    /// Muted themselves.
    SelfMuted,
    /// Hears nobody.
    Deafened,
    /// Admin mute.
    AdminMuted,
}

impl Status {
    /// The wire text.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::None => "none",
            Status::Ready => "ready",
            Status::Talking => "talking",
            Status::SelfMuted => "self_muted",
            Status::Deafened => "deafened",
            Status::AdminMuted => "admin_muted",
        }
    }
}

/// What a UDP datagram did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UdpReceived {
    /// The peer authenticated: bind its address, then send `reply` (`AUTH_OK`) to it.
    Auth {
        /// The peer.
        peer: PeerId,
        /// The `AUTH_OK` datagram.
        reply: Vec<u8>,
    },
    /// A frame was relayed.
    Frame(Forwarded),
}

struct Peer {
    session: Session,
    /// Authenticated on the UDP port.
    udp: bool,
    tokens: u64,
    refilled: Option<u64>,
    last_frame: Option<u64>,
    self_mute: bool,
    deafen: bool,
}

/// Per-peer relay state.
pub struct Relay {
    udp_port: Option<u16>,
    peers: HashMap<PeerId, Peer>,
}

impl Relay {
    /// A relay with the voice UDP port the host has open (`None`: no port, voice is off).
    pub fn new(udp_port: Option<u16>) -> Relay {
        Relay { udp_port, peers: HashMap::new() }
    }

    /// Changes the voice UDP port (announced to sessions that start afterwards).
    pub fn set_udp_port(&mut self, udp_port: Option<u16>) {
        self.udp_port = udp_port;
    }

    /// The UDP port to announce (`SERVER_HELLO.voiceUdpPort`, `VOICE_KEY.udpPort`).
    pub fn udp_port(&self) -> Option<u16> {
        self.udp_port
    }

    /// Adds a peer once its `VOICE_KEY` arrived; it gets frames once it authenticated on the UDP port.
    pub fn add_peer(&mut self, peer: PeerId, session: Session) {
        let p =
            Peer { session, udp: false, tokens: 0, refilled: None, last_frame: None, self_mute: false, deafen: false };
        self.peers.insert(peer, p);
    }

    /// Forgets a peer (left the game).
    pub fn remove_peer(&mut self, peer: PeerId) {
        self.peers.remove(&peer);
    }

    /// True if the peer has a session.
    pub fn has_peer(&self, peer: PeerId) -> bool {
        self.peers.contains_key(&peer)
    }

    /// Applies `VOICE_CTRL`.
    pub fn set_ctrl(&mut self, peer: PeerId, self_mute: bool, deafen: bool) {
        if let Some(p) = self.peers.get_mut(&peer) {
            p.self_mute = self_mute;
            p.deafen = deafen;
        }
    }

    /// The peer's state for `VOICE_STATE`.
    pub fn status(&self, peer: PeerId, table: &RelayTable, now: u64) -> Status {
        let Some(p) = self.peers.get(&peer) else { return Status::None };
        if table.is_muted(peer, now) {
            Status::AdminMuted
        } else if p.deafen {
            Status::Deafened
        } else if p.self_mute {
            Status::SelfMuted
        } else if talking(p, now) {
            Status::Talking
        } else {
            Status::Ready
        }
    }

    /// A UDP datagram arrived. The host must check that a `Frame`'s source address is the one bound by `Auth`.
    pub fn receive_udp(
        &mut self,
        now: u64,
        datagram: &[u8],
        table: &RelayTable,
        out: &mut impl Transport,
    ) -> Result<UdpReceived, Drop> {
        match Datagram::decode(datagram, crate::frame::DEFAULT_UDP_MAX).ok_or(Drop::Malformed)? {
            Datagram::Auth { peer, token } => {
                let p = self.peers.get_mut(&peer).ok_or(Drop::UnknownPeer)?;
                if self.udp_port.is_none() || !p.session.verify_udp_token(peer, &token) {
                    return Err(Drop::Crypto);
                }
                p.udp = true;
                Ok(UdpReceived::Auth { peer, reply: Datagram::AuthOk { peer }.encode() })
            }
            Datagram::Frame { peer, nonce, sealed } => {
                if !self.peers.get(&peer).ok_or(Drop::UnknownPeer)?.udp {
                    return Err(Drop::NotOnUdp);
                }
                self.relay(now, peer, nonce, &sealed, table, out).map(UdpReceived::Frame)
            }
            Datagram::AuthOk { .. } => Err(Drop::Malformed),
        }
    }

    /// An authenticated peer's frame: relays it to its routes.
    fn relay(
        &mut self,
        now: u64,
        from: PeerId,
        nonce: u64,
        sealed: &[u8],
        table: &RelayTable,
        out: &mut impl Transport,
    ) -> Result<Forwarded, Drop> {
        let speakers = self.peers.iter().filter(|(id, p)| **id != from && talking(p, now)).count();
        let p = self.peers.get_mut(&from).expect("checked by receive_udp");
        let plain = p.session.open(nonce, sealed).map_err(|_| Drop::Crypto)?;
        let frame = ClientFrame::decode(&plain).map_err(|_| Drop::Malformed)?;
        if table.is_muted(from, now) {
            return Err(Drop::AdminMuted);
        }
        if p.self_mute {
            return Err(Drop::SelfMuted);
        }
        if frame.opus.len() > max_opus_bytes(table.caps.max_bitrate) {
            return Err(Drop::TooLarge);
        }
        if !take_token(p, now, table.caps.max_frames_per_sec) {
            return Err(Drop::RateLimited);
        }
        let started = !talking(p, now);
        if started && speakers >= table.caps.max_speakers {
            return Err(Drop::TooManySpeakers);
        }
        p.last_frame = Some(now);

        let gain = table.speaker_gain(from);
        let mut done: Vec<PeerId> = Vec::new();
        let mut copies = 0;
        for r in table.routes(from) {
            if r.modes & frame.mode.bit() == 0 || r.listener == from || done.contains(&r.listener) {
                continue;
            }
            let Some(l) = self.peers.get_mut(&r.listener) else { continue };
            done.push(r.listener);
            if !l.udp || l.deafen && !(frame.mode == VoiceMode::Broadcast && table.broadcast_overrides_deafen) {
                continue;
            }
            let plain = ServerFrame {
                speaker: from,
                mode: frame.mode,
                seq: frame.seq,
                gain: gain_byte(gain, r.gain),
                pan: pan_byte(r.pan),
                muffle: muffle_byte(r.muffle),
                opus: frame.opus.clone(),
            }
            .encode();
            let Ok((nonce, sealed)) = l.session.seal(&plain) else { continue };
            out.send(r.listener, Datagram::Frame { peer: r.listener, nonce, sealed }.encode());
            copies += 1;
        }
        Ok(Forwarded { started, copies, mode: frame.mode })
    }

    /// Housekeeping: talk-stop events and expired mutes since the last call.
    pub fn tick(&mut self, now: u64, table: &mut RelayTable) -> Vec<Event> {
        let mut stopped: Vec<PeerId> = Vec::new();
        for (id, p) in &mut self.peers {
            if p.last_frame.is_some() && !talking(p, now) {
                p.last_frame = None;
                stopped.push(*id);
            }
        }
        stopped.sort_unstable();
        let mut ev: Vec<Event> = stopped.into_iter().map(Event::TalkStop).collect();
        ev.extend(table.prune_mutes(now).into_iter().map(Event::MuteExpired));
        ev
    }
}

fn talking(p: &Peer, now: u64) -> bool {
    p.last_frame.is_some_and(|t| now < t + TALK_TIMEOUT_MS)
}

/// Token bucket in milli-frames: burst of `max(fps / 5, 5)` frames, refilled at `fps`.
fn take_token(p: &mut Peer, now: u64, fps: u32) -> bool {
    let cap = u64::from((fps / 5).max(5)) * 1000;
    p.tokens = match p.refilled {
        None => cap,
        Some(t) => (p.tokens + now.saturating_sub(t) * u64::from(fps)).min(cap),
    };
    p.refilled = Some(now);
    if p.tokens < 1000 {
        return false;
    }
    p.tokens -= 1000;
    true
}
