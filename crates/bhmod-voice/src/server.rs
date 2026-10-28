//! The server half of voice chat, host-agnostic (API 0.3.4): the `VOICE_KEY` handshake, the relay, `VOICE_STATE`
//! deltas and the mod policy behind [`bhmod_core::VoiceHost`]. The server program only glues it:
//!
//! - [`VoiceServer::join`] / [`VoiceServer::leave`] when a player is in the game / gone (with `HELLO.features`);
//! - [`VoiceServer::receive`] for every 0xBC `VOICE_KEY` and `VOICE_CTRL` of a player, and
//!   [`VoiceServer::receive_udp`] for datagrams of the voice UDP port (the only path of voice frames);
//! - [`VoiceServer::set_transport`] with the port the host really has open (`None`: no port, voice stays off);
//! - [`VoiceServer::tick`] once per poll; [`VoiceServer::take_events`] into `ModHost::voice_event`;
//! - [`SharedVoiceServer`] as `GameBridge::voice`.
//!
//! Output goes to a [`ServerSink`]. `now` is Unix time in milliseconds everywhere (admin mutes end at a Unix time).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use bhmod_core::types::ApiError;
use bhmod_core::voice::{
    Activation, Caps as CoreCaps, Icons, Mode, Route as CoreRoute, TransportConfig, VoiceEvent, VoiceHost,
    VoiceState as State,
};
use bhmod_proto::{feature, Message, VoiceKey, VoiceState};

use crate::crypto::{KeyPair, Role};
use crate::relay::{Caps, Event, Relay, RelayTable, Route, Status, Transport as RelayTransport, UdpReceived};
use crate::udp::Datagram;
use crate::{PeerId, VoiceMode};

/// Where the server's voice output goes.
pub trait ServerSink {
    /// A reliable 0xBC packet to a player (`VOICE_KEY`, `VOICE_STATE`).
    fn reliable(&mut self, player: &str, packet: Vec<u8>);
    /// A datagram to the address the peer authenticated from on the voice UDP port.
    fn udp(&mut self, peer: PeerId, datagram: Vec<u8>);
}

/// A [`VoiceMode`] as the core [`Mode`] (same order).
pub fn mode_of(m: VoiceMode) -> Mode {
    Mode::ALL[m as usize]
}

/// A relay [`Status`] as the core [`State`].
pub fn state_of(s: Status) -> State {
    match s {
        Status::None => State::None,
        Status::Ready => State::Ready,
        Status::Talking => State::Talking,
        Status::SelfMuted => State::SelfMuted,
        Status::Deafened => State::Deafened,
        Status::AdminMuted => State::AdminMuted,
    }
}

#[derive(Debug, Default)]
struct Player {
    alias: String,
    voice: bool,
    peer: Option<PeerId>,
    modes: u8,
    activations: u8,
    /// `(modes, activations, bitrate)` last told to the client.
    told: Option<(u8, u8, u32)>,
    /// The mode of the current talk spurt.
    talking: Option<Mode>,
}

/// The server's voice state for every player.
pub struct VoiceServer {
    relay: Relay,
    table: RelayTable,
    transport: TransportConfig,
    players: BTreeMap<String, Player>,
    peers: HashMap<PeerId, String>,
    next_peer: PeerId,
    routes: HashMap<String, Vec<CoreRoute>>,
    mutes: HashMap<String, Option<u64>>,
    gains: HashMap<String, u16>,
    icons: Icons,
    icons_dirty: bool,
    /// Last state sent / reported per player.
    states: BTreeMap<String, State>,
    /// Players whose speaker id the others do not know yet.
    new_speakers: BTreeSet<String>,
    /// Players who left, to announce as `gone`.
    gone: BTreeSet<String>,
    /// Players whose name the others do not know yet.
    new_names: BTreeSet<String>,
    events: Vec<VoiceEvent>,
    now: u64,
    keys: Box<dyn FnMut() -> Option<KeyPair> + Send>,
}

/// Adapts a [`ServerSink`] to the relay.
struct Out<'a>(&'a mut dyn ServerSink);

impl RelayTransport for Out<'_> {
    fn send(&mut self, to: PeerId, datagram: Vec<u8>) {
        self.0.udp(to, datagram);
    }
}

impl Default for VoiceServer {
    fn default() -> VoiceServer {
        VoiceServer::with_keys(Box::new(|| KeyPair::generate().ok()))
    }
}

impl VoiceServer {
    /// A server with the defaults (no UDP port yet, default caps, all icons) and random session keys.
    pub fn new() -> VoiceServer {
        VoiceServer::default()
    }

    /// A server whose session keys come from `keys` (tests; `None` = no randomness, the key exchange is refused).
    pub fn with_keys(keys: Box<dyn FnMut() -> Option<KeyPair> + Send>) -> VoiceServer {
        VoiceServer {
            relay: Relay::new(None),
            table: RelayTable::new(Caps::default()),
            transport: TransportConfig::default(),
            players: BTreeMap::new(),
            peers: HashMap::new(),
            next_peer: 1,
            routes: HashMap::new(),
            mutes: HashMap::new(),
            gains: HashMap::new(),
            icons: Icons::default(),
            icons_dirty: false,
            states: BTreeMap::new(),
            new_speakers: BTreeSet::new(),
            gone: BTreeSet::new(),
            new_names: BTreeSet::new(),
            events: Vec::new(),
            now: 0,
            keys,
        }
    }

    /// The voice UDP port announced to clients (`None`: no port, voice is off).
    pub fn udp_port(&self) -> Option<u16> {
        self.relay.udp_port()
    }

    /// The transport last given to [`VoiceServer::set_transport`].
    pub fn transport(&self) -> TransportConfig {
        self.transport
    }

    /// The speaker id of a player with a voice session.
    pub fn peer_of(&self, player: &str) -> Option<PeerId> {
        self.players.get(player)?.peer
    }

    /// A player is in the game, with their alias; `features` is their `HELLO.features`. Players without `voice` stay
    /// `none`. Every voice client learns the alias (`VOICE_STATE.names`), so it knows all players.
    ///
    /// A join for a player id that is still here (a reconnect whose quit has not arrived yet) keeps what the mod set
    /// for them (allowed modes and activations) and their speaker id; the client is told its allowance again.
    pub fn join(&mut self, player: &str, alias: &str, features: &[String]) {
        let has = |f: &str| features.iter().any(|x| x == f);
        let p = self
            .players
            .entry(player.to_string())
            .or_insert_with(|| Player { activations: Activation::mask(Activation::ALL), ..Player::default() });
        p.voice = has(feature::VOICE);
        p.alias = alias.to_string();
        p.told = None;
        self.gone.remove(player);
        self.new_names.insert(player.to_string());
    }

    /// A player left: their session ends; the others learn it with the next tick (`gone`). Admin mutes and speaker
    /// gains stay for a reconnect.
    pub fn leave(&mut self, player: &str) {
        let Some(p) = self.players.remove(player) else { return };
        if let Some(peer) = p.peer {
            self.relay.remove_peer(peer);
            self.table.remove_peer(peer);
            // The player's mute stays in `mutes` (by player id) for a reconnect.
            self.table.unmute(peer);
            self.peers.remove(&peer);
        }
        self.routes.remove(player);
        self.states.remove(player);
        self.new_speakers.remove(player);
        self.new_names.remove(player);
        self.gone.insert(player.to_string());
        self.apply_routes();
    }

    fn status(&self, player: &str) -> State {
        match self.peer_of(player) {
            Some(peer) => state_of(self.relay.status(peer, &self.table, self.now)),
            None => State::None,
        }
    }

    /// Rebuilds the relay routes from the mod's routes (by player id) for the players with a session.
    fn apply_routes(&mut self) {
        let speakers: Vec<(String, PeerId)> = self.peers.iter().map(|(p, n)| (n.clone(), *p)).collect();
        for (player, peer) in speakers {
            let routes = self.routes.get(&player).map_or_else(Vec::new, |rs| {
                rs.iter()
                    .filter_map(|r| {
                        let listener = self.peer_of(&r.listener)?;
                        Some(Route { listener, modes: r.mode_mask(), gain: r.gain, pan: r.pan, muffle: r.muffle })
                    })
                    .collect()
            });
            self.table.set_routes(peer, routes);
        }
    }

    /// A 0xBC voice op from `player`: `VOICE_KEY` (answered with the server's key, the voice UDP port and a full
    /// `VOICE_STATE`) or `VOICE_CTRL`. Other messages, and every op of a player who did not list `voice`, are
    /// ignored.
    pub fn receive(&mut self, now: u64, player: &str, msg: &Message, sink: &mut dyn ServerSink) {
        self.now = now;
        if !self.players.get(player).is_some_and(|p| p.voice) {
            return;
        }
        match msg {
            Message::VoiceKey(k) => self.key(player, k, sink),
            Message::VoiceCtrl(c) => {
                if let Some(peer) = self.peer_of(player) {
                    self.relay.set_ctrl(peer, c.self_mute, c.deafen);
                }
            }
            _ => {}
        }
    }

    fn forwarded(&mut self, peer: PeerId, f: crate::relay::Forwarded) {
        if !f.started {
            return;
        }
        let player = self.peers[&peer].clone();
        let mode = mode_of(f.mode);
        if let Some(p) = self.players.get_mut(&player) {
            p.talking = Some(mode);
        }
        self.events.push(VoiceEvent::TalkStart { player, mode });
    }

    fn key(&mut self, player: &str, k: &VoiceKey, sink: &mut dyn ServerSink) {
        let Some(kp) = (self.keys)() else { return };
        let Ok(session) = kp.session(Role::Server, &k.public) else { return };
        let p = self.players.get_mut(player).expect("checked by receive");
        let peer = match p.peer {
            Some(peer) => peer,
            None => {
                let peer = self.next_peer;
                self.next_peer = self.next_peer.checked_add(1).unwrap_or(1);
                p.peer = Some(peer);
                peer
            }
        };
        p.told = None;
        self.relay.add_peer(peer, session);
        self.peers.insert(peer, player.to_string());
        self.table.unmute(peer);
        if let Some(until) = self.mutes.get(player) {
            self.table.mute(peer, *until);
        }
        self.table.set_speaker_gain(peer, self.gains.get(player).copied().unwrap_or(100));
        self.apply_routes();
        self.new_speakers.insert(player.to_string());
        let reply = VoiceKey { public: kp.public(), peer: Some(peer), udp_port: self.relay.udp_port() };
        sink.reliable(player, Message::VoiceKey(reply).encode());
        let full = self.full_state(player);
        sink.reliable(player, Message::VoiceState(full).encode());
    }

    /// The full `VOICE_STATE` for `to`.
    fn full_state(&mut self, to: &str) -> VoiceState {
        let players = self.players.keys().map(|p| (p.clone(), self.status(p).wire().to_string())).collect();
        let speakers = self.peers.iter().map(|(peer, p)| (p.clone(), *peer)).collect();
        let names = self.players.iter().map(|(p, x)| (p.clone(), x.alias.clone())).collect();
        let mut s = VoiceState { full: true, players, speakers, names, ..VoiceState::default() };
        s.icons = self.icons.wire().iter().map(|(k, v)| (k.to_string(), *v)).collect();
        self.tell_allowed(to, &mut s);
        s
    }

    /// Adds `modes`, `activations` and `bitrate` when they changed for `to`.
    fn tell_allowed(&mut self, to: &str, s: &mut VoiceState) {
        let bitrate = self.table.caps.max_bitrate;
        let p = self.players.get_mut(to).expect("only players get states");
        let now = (p.modes, p.activations, bitrate);
        if p.told != Some(now) {
            p.told = Some(now);
            (s.modes, s.activations, s.bitrate) = (Some(now.0), Some(now.1), Some(now.2));
        }
    }

    /// A datagram on the voice UDP port. `bound(peer)` must say whether the datagram's source address is the one
    /// that peer authenticated from (frames from other addresses are dropped). Returns the peer to bind to the
    /// source address after a valid `AUTH` (its `AUTH_OK` already went to [`ServerSink::udp`]).
    pub fn receive_udp(
        &mut self,
        now: u64,
        datagram: &[u8],
        bound: &dyn Fn(PeerId) -> bool,
        sink: &mut dyn ServerSink,
    ) -> Option<PeerId> {
        self.now = now;
        let frame_peer = match Datagram::decode(datagram, crate::frame::DEFAULT_UDP_MAX) {
            Some(Datagram::Frame { peer, .. }) => Some(peer),
            _ => None,
        };
        if frame_peer.is_some_and(|p| !bound(p)) {
            return None;
        }
        match self.relay.receive_udp(now, datagram, &self.table, &mut Out(&mut *sink)) {
            Ok(UdpReceived::Auth { peer, reply }) => {
                sink.udp(peer, reply);
                Some(peer)
            }
            Ok(UdpReceived::Frame(f)) => {
                // Only a frame datagram is relayed, so `frame_peer` is set.
                self.forwarded(frame_peer.unwrap_or_default(), f);
                None
            }
            Err(_) => None,
        }
    }

    /// Once per poll: talk-stop, expired mutes, state changes (`voice:state` events) and the `VOICE_STATE` deltas
    /// to players with a session.
    pub fn tick(&mut self, now: u64, sink: &mut dyn ServerSink) {
        self.now = now;
        for e in self.relay.tick(now, &mut self.table) {
            match e {
                Event::TalkStop(peer) => {
                    let player = self.peers[&peer].clone();
                    let mode = self.players.get_mut(&player).and_then(|p| p.talking.take()).unwrap_or(Mode::Proximity);
                    self.events.push(VoiceEvent::TalkStop { player, mode });
                }
                // Player-level mutes expire below.
                Event::MuteExpired(_) => {}
            }
        }
        self.mutes.retain(|_, u| u.is_none_or(|t| now < t));

        let mut changed: BTreeMap<String, String> =
            std::mem::take(&mut self.gone).into_iter().map(|p| (p, "gone".to_string())).collect();
        let names: Vec<String> = self.players.keys().cloned().collect();
        for p in names {
            let s = self.status(&p);
            if self.states.insert(p.clone(), s) != Some(s) {
                self.events.push(VoiceEvent::State { player: p.clone(), state: s });
                changed.insert(p, s.wire().to_string());
            }
        }
        let speakers: BTreeMap<String, PeerId> = std::mem::take(&mut self.new_speakers)
            .into_iter()
            .filter_map(|p| Some((p.clone(), self.peer_of(&p)?)))
            .collect();
        let names: BTreeMap<String, String> = std::mem::take(&mut self.new_names)
            .into_iter()
            .map(|p| {
                let alias = self.players[&p].alias.clone();
                (p, alias)
            })
            .collect();
        let icons = std::mem::take(&mut self.icons_dirty);
        let to: Vec<String> = self.peers.values().cloned().collect();
        for player in to {
            let mut s = VoiceState {
                players: changed.clone(),
                speakers: speakers.clone(),
                names: names.clone(),
                ..VoiceState::default()
            };
            if icons {
                s.icons = self.icons.wire().iter().map(|(k, v)| (k.to_string(), *v)).collect();
            }
            self.tell_allowed(&player, &mut s);
            if s != VoiceState::default() {
                sink.reliable(&player, Message::VoiceState(s).encode());
            }
        }
    }

    /// The events since the last call, for `ModHost::voice_event`.
    pub fn take_events(&mut self) -> Vec<VoiceEvent> {
        std::mem::take(&mut self.events)
    }

    // ---- the mod's policy (VoiceHost) ----

    /// `voice.state`.
    pub fn state(&self, player: &str) -> State {
        self.status(player)
    }

    /// `voice.set-routes`.
    pub fn set_routes(&mut self, speaker: &str, routes: &[CoreRoute]) {
        self.routes.insert(speaker.to_string(), routes.to_vec());
        self.apply_routes();
    }

    /// `voice.mute`.
    pub fn mute(&mut self, player: &str, until_ms: Option<u64>) {
        self.mutes.insert(player.to_string(), until_ms);
        if let Some(peer) = self.peer_of(player) {
            self.table.mute(peer, until_ms);
        }
    }

    /// `voice.unmute`.
    pub fn unmute(&mut self, player: &str) -> bool {
        if let Some(peer) = self.peer_of(player) {
            self.table.unmute(peer);
        }
        self.mutes.remove(player).is_some()
    }

    /// `voice.set-speaker-gain`.
    pub fn set_speaker_gain(&mut self, player: &str, percent: u16) {
        self.gains.insert(player.to_string(), percent);
        if let Some(peer) = self.peer_of(player) {
            self.table.set_speaker_gain(peer, percent);
        }
    }

    /// `voice.set-caps`.
    pub fn set_caps(&mut self, c: CoreCaps) {
        self.table.caps = Caps {
            max_bitrate: c.max_bitrate,
            max_speakers: c.max_speakers as usize,
            max_frames_per_sec: c.max_frames_per_sec,
        };
    }

    /// `voice.set-transport`, with the port the host really opened. Only `udp_port` matters (frames always use the
    /// voice UDP port; `mode` is kept for 0.3.x compatibility and ignored). `None`: no port, voice stays off for
    /// sessions that start afterwards.
    pub fn set_transport(&mut self, t: TransportConfig) {
        self.transport = t;
        self.relay.set_udp_port(t.udp_port);
    }

    /// `voice.set-allowed`.
    pub fn set_allowed(&mut self, player: &str, modes: u8, activations: u8) {
        if let Some(p) = self.players.get_mut(player) {
            (p.modes, p.activations) = (modes, activations);
        }
    }

    /// `voice.set-icons`.
    pub fn set_icons(&mut self, icons: Icons) {
        self.icons_dirty |= self.icons != icons;
        self.icons = icons;
    }

    /// `voice.icons`.
    pub fn icons(&self) -> Icons {
        self.icons
    }

    /// `voice.set-broadcast-overrides-deafen`.
    pub fn set_broadcast_overrides_deafen(&mut self, on: bool) {
        self.table.broadcast_overrides_deafen = on;
    }
}

/// A [`VoiceServer`] shared between the game thread, the bridge ([`VoiceHost`]) and a UDP thread.
#[derive(Clone, Default)]
pub struct SharedVoiceServer(pub Arc<Mutex<VoiceServer>>);

impl SharedVoiceServer {
    /// A new shared server.
    pub fn new(server: VoiceServer) -> SharedVoiceServer {
        SharedVoiceServer(Arc::new(Mutex::new(server)))
    }

    /// Locks it (a poisoned lock is taken over: the state stays usable).
    pub fn lock(&self) -> MutexGuard<'_, VoiceServer> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl VoiceHost for SharedVoiceServer {
    fn state(&self, player: &str) -> Result<State, ApiError> {
        Ok(self.lock().state(player))
    }
    fn icons(&self) -> Result<Icons, ApiError> {
        Ok(self.lock().icons())
    }
    fn set_routes(&self, speaker: &str, routes: &[CoreRoute]) -> Result<(), ApiError> {
        self.lock().set_routes(speaker, routes);
        Ok(())
    }
    fn mute(&self, player: &str, until_ms: Option<u64>) -> Result<(), ApiError> {
        self.lock().mute(player, until_ms);
        Ok(())
    }
    fn unmute(&self, player: &str) -> Result<bool, ApiError> {
        Ok(self.lock().unmute(player))
    }
    fn set_speaker_gain(&self, player: &str, percent: u16) -> Result<(), ApiError> {
        self.lock().set_speaker_gain(player, percent);
        Ok(())
    }
    fn set_caps(&self, caps: CoreCaps) -> Result<(), ApiError> {
        self.lock().set_caps(caps);
        Ok(())
    }
    fn set_transport(&self, config: TransportConfig) -> Result<(), ApiError> {
        self.lock().set_transport(config);
        Ok(())
    }
    fn set_allowed(&self, player: &str, modes: u8, activations: u8) -> Result<(), ApiError> {
        self.lock().set_allowed(player, modes, activations);
        Ok(())
    }
    fn set_icons(&self, icons: Icons) -> Result<(), ApiError> {
        self.lock().set_icons(icons);
        Ok(())
    }
    fn set_broadcast_overrides_deafen(&self, on: bool) -> Result<(), ApiError> {
        self.lock().set_broadcast_overrides_deafen(on);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
