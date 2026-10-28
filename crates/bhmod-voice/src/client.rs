//! The client half of voice chat, host-agnostic (API 0.3.4): the `VOICE_KEY` handshake, the microphone pipeline
//! (VAD, activation, Opus, sealing), per-speaker jitter buffers and decoders, the mix, `VOICE_STATE`, local volumes
//! and the mod controls behind [`bhmod_core::VoiceHost`]. The loader (PC, Android) glues it:
//!
//! - when `bhmod_client::Session::voice_enabled` turns true: send [`VoiceClient::start`] (through
//!   `Session::voice_packet`);
//! - every `Output::Voice(msg)` of the session -> [`VoiceClient::on_message`]; datagrams of the voice UDP port ->
//!   [`VoiceClient::on_udp`];
//! - every 20 ms of microphone (48 kHz mono, 960 samples) -> [`VoiceClient::on_mic_frame`] while
//!   [`VoiceClient::mic_wanted`] (report the device with [`VoiceClient::mic_state`]);
//! - [`VoiceClient::poll_send`] -> reliable 0xBC / unsequenced ENet channel 0 / UDP datagrams;
//! - every 20 ms of playback -> [`VoiceClient::mix`] (interleaved stereo, 1920 samples);
//! - [`VoiceClient::take_events`] -> `ModHost::voice_event`; [`SharedVoiceClient`] as `GameBridge::voice`;
//! - [`VoiceClient::prefs`] / [`VoiceClient::load_prefs`] for the device file of volumes and local mutes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use bhmod_core::types::ApiError;
use bhmod_core::voice::{
    Activation as Act, Allowance, Icons, MicState, Mode, VoiceEvent, VoiceHost, VoiceState as State,
};
use bhmod_proto::{Message, VoiceCtrl, VoiceKey, VoiceState};

use crate::activation::{Activation, Allowed, Input, Kind};
use crate::codec::{Codec, Decoder, Encoder};
use crate::crypto::{KeyPair, Role};
use crate::frame::ServerFrame;
use crate::jitter::{JitterBuffer, Playout, DEFAULT_MAX, DEFAULT_MAX_CONCEAL, DEFAULT_TARGET};
use crate::link::ClientLink;
use crate::mix::{Mixer, Source, MAX_VOLUME};
use crate::path::{Path, TransportMode};
use crate::server::mode_of;
use crate::udp::KEEPALIVE_MS;
use crate::vad::{Vad, VadConfig};
use crate::{PeerId, VoiceMode, FRAME_SAMPLES, SAMPLE_RATE};

/// A packet for the loader to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// A reliable 0xBC packet (`VOICE_KEY`, `VOICE_CTRL`).
    Reliable(Vec<u8>),
    /// An in-band `VOICE_FRAME`: unreliable + unsequenced on ENet channel 0.
    Unsequenced(Vec<u8>),
    /// A datagram for the server's voice UDP port.
    Udp(Vec<u8>),
}

struct Speaker {
    player: String,
    jitter: JitterBuffer<ServerFrame>,
    decoder: Box<dyn Decoder>,
    last: Option<ServerFrame>,
    pcm: Vec<f32>,
    talking: Option<Mode>,
}

fn kind_of(a: Act) -> Kind {
    match a {
        Act::Ptt => Kind::Ptt,
        Act::Vad => Kind::Vad,
        Act::Toggle => Kind::Toggle,
    }
}

fn voice_mode(m: Mode) -> VoiceMode {
    VoiceMode::ALL[m as usize]
}

/// The client's voice state.
pub struct VoiceClient {
    codec: Arc<dyn Codec>,
    unsequenced: bool,
    keypair: Option<KeyPair>,
    link: Option<ClientLink>,
    udp_port: Option<u16>,
    last_auth: Option<u64>,
    encoder: Option<Box<dyn Encoder>>,
    bitrate: u32,
    opus: Vec<u8>,
    speakers: HashMap<PeerId, Speaker>,
    peer_of: BTreeMap<String, PeerId>,
    states: BTreeMap<String, State>,
    icons: Icons,
    activation: Activation,
    vad: Vad,
    mixer: Mixer,
    master: u16,
    volumes: BTreeMap<String, u16>,
    muted: BTreeSet<String>,
    mic_wanted: bool,
    mic: MicState,
    self_mute: bool,
    deafen: bool,
    transmitting: Option<VoiceMode>,
    out: Vec<Outgoing>,
    events: Vec<VoiceEvent>,
}

impl VoiceClient {
    /// A client. `unsequenced`: this loader can send in-band `VOICE_FRAME`s (it listed `voice-unsequenced`).
    pub fn new(codec: Arc<dyn Codec>, unsequenced: bool) -> VoiceClient {
        let activation = Activation::new(Allowed::default(), 0, Kind::Ptt, VoiceMode::Proximity);
        VoiceClient {
            codec,
            unsequenced,
            keypair: None,
            link: None,
            udp_port: None,
            last_auth: None,
            encoder: None,
            bitrate: 24_000,
            opus: Vec::new(),
            speakers: HashMap::new(),
            peer_of: BTreeMap::new(),
            states: BTreeMap::new(),
            icons: Icons::default(),
            activation,
            vad: Vad::new(VadConfig::default()),
            mixer: Mixer::new(SAMPLE_RATE),
            master: 100,
            volumes: BTreeMap::new(),
            muted: BTreeSet::new(),
            mic_wanted: false,
            mic: MicState::Off,
            self_mute: false,
            deafen: false,
            transmitting: None,
            out: Vec::new(),
            events: Vec::new(),
        }
    }

    /// The client's `VOICE_KEY` (a fresh key pair; `None` without randomness). Send it once voice is enabled.
    pub fn start(&mut self) -> Option<Vec<u8>> {
        Some(self.start_with(KeyPair::generate().ok()?))
    }

    /// [`VoiceClient::start`] with a given key pair (tests).
    pub fn start_with(&mut self, kp: KeyPair) -> Vec<u8> {
        let pkt = Message::VoiceKey(VoiceKey { public: kp.public(), peer: None, udp_port: None }).encode();
        self.keypair = Some(kp);
        pkt
    }

    /// True once the server answered `VOICE_KEY`.
    pub fn connected(&self) -> bool {
        self.link.is_some()
    }

    /// A raw 0xBC packet from the server (decoded and passed to [`VoiceClient::on_message`]).
    pub fn on_packet(&mut self, packet: &[u8]) {
        if let Ok(m) = Message::decode(packet) {
            self.on_message(&m);
        }
    }

    /// A voice op from the server (the session's `Output::Voice`). Other messages are ignored.
    pub fn on_message(&mut self, msg: &Message) {
        match msg {
            Message::VoiceKey(k) => self.on_key(k),
            Message::VoiceState(s) => self.on_state(s),
            Message::VoiceFrame(f) => {
                let Some(link) = self.link.as_mut() else { return };
                if let Ok(frame) = link.open(f.nonce, &f.sealed) {
                    self.heard(frame);
                }
            }
            _ => {}
        }
    }

    fn on_key(&mut self, k: &VoiceKey) {
        let (Some(kp), Some(peer)) = (self.keypair.as_ref(), k.peer) else { return };
        let Ok(session) = kp.session(Role::Client, &k.public) else { return };
        let mut link = ClientLink::new(session, peer, TransportMode::Auto, k.udp_port);
        link.selector_mut().inband_capable = self.unsequenced;
        self.link = Some(link);
        self.udp_port = k.udp_port;
        self.last_auth = None;
        self.encoder = self.codec.encoder(self.bitrate).ok();
    }

    fn on_state(&mut self, s: &VoiceState) {
        let before = self.allowed();
        if s.full {
            self.peer_of.clear();
        }
        self.peer_of.extend(s.speakers.iter().map(|(p, id)| (p.clone(), *id)));
        let mut seen = BTreeSet::new();
        for (p, w) in &s.players {
            seen.insert(p.clone());
            if w == "gone" {
                self.states.remove(p);
                if let Some(id) = self.peer_of.remove(p) {
                    self.speakers.remove(&id);
                    self.mixer.forget(id);
                }
                continue;
            }
            let state = State::from_wire(w).unwrap_or(State::None);
            if self.states.insert(p.clone(), state) != Some(state) {
                self.events.push(VoiceEvent::State { player: p.clone(), state });
            }
        }
        if s.full {
            self.states.retain(|p, _| seen.contains(p));
        }
        self.icons.apply_wire(s.icons.iter().map(|(k, v)| (k.as_str(), *v)));
        if let Some(m) = s.modes {
            self.activation.modes = m & 0x0f;
        }
        if let Some(a) = s.activations {
            self.activation.allowed = Allowed { ptt: a & 1 != 0, vad: a & 2 != 0, toggle: a & 4 != 0 };
        }
        if let Some(b) = s.bitrate {
            self.bitrate = b;
            if let Some(e) = self.encoder.as_mut() {
                e.set_bitrate(b);
            }
        }
        if self.allowed() != before {
            self.events.push(VoiceEvent::Allowed(self.allowed()));
        }
        self.sync_volumes();
    }

    /// A datagram from the server's voice UDP port.
    pub fn on_udp(&mut self, datagram: &[u8]) {
        let Some(link) = self.link.as_mut() else { return };
        if let Some(Ok(frame)) = link.receive_udp(datagram) {
            self.heard(frame);
        }
    }

    fn player_of(&self, peer: PeerId) -> Option<String> {
        self.peer_of.iter().find(|(_, id)| **id == peer).map(|(p, _)| p.clone())
    }

    fn heard(&mut self, frame: ServerFrame) {
        if self.deafen {
            return;
        }
        let Some(player) = self.player_of(frame.speaker) else { return };
        if !self.speakers.contains_key(&frame.speaker) {
            let Ok(decoder) = self.codec.decoder() else { return };
            let s = Speaker {
                player,
                jitter: JitterBuffer::new(DEFAULT_TARGET, DEFAULT_MAX, DEFAULT_MAX_CONCEAL),
                decoder,
                last: None,
                pcm: vec![0.0; FRAME_SAMPLES],
                talking: None,
            };
            self.speakers.insert(frame.speaker, s);
        }
        let s = self.speakers.get_mut(&frame.speaker).expect("inserted above");
        if s.talking.is_none() {
            let mode = mode_of(frame.mode);
            s.talking = Some(mode);
            self.events.push(VoiceEvent::TalkStart { player: s.player.clone(), mode });
        }
        s.jitter.push(frame.seq, frame);
    }

    /// One 20 ms microphone frame (48 kHz mono, [`FRAME_SAMPLES`] samples; other lengths are ignored). Runs VAD and
    /// the activation; while transmitting, encodes and seals it for [`VoiceClient::poll_send`].
    pub fn on_mic_frame(&mut self, pcm: &[f32]) {
        if pcm.len() != FRAME_SAMPLES || !self.mic_wanted {
            return;
        }
        let active = self.vad.process(pcm);
        self.transmitting = self.activation.handle(Input::Vad(active));
        let (Some(mode), Some(enc), Some(link)) = (self.transmitting, self.encoder.as_mut(), self.link.as_mut()) else {
            return;
        };
        if enc.encode(pcm, &mut self.opus).is_err() {
            return;
        }
        let loss = self.speakers.values().map(|s| s.jitter.loss()).fold(0.0, f32::max);
        if let Some((path, pkt)) = link.send(mode, &self.opus, loss) {
            self.out.push(match path {
                Path::InBand => Outgoing::Unsequenced(pkt),
                Path::Udp => Outgoing::Udp(pkt),
            });
        }
    }

    /// The packets to send now (`now` in ms, for the UDP `AUTH` keepalive).
    pub fn poll_send(&mut self, now: u64) -> Vec<Outgoing> {
        if let (Some(link), Some(_)) = (self.link.as_ref(), self.udp_port) {
            if self.last_auth.is_none_or(|t| now >= t + KEEPALIVE_MS) {
                self.last_auth = Some(now);
                self.out.push(Outgoing::Udp(link.auth_datagram()));
            }
        }
        std::mem::take(&mut self.out)
    }

    /// The server's voice UDP port, when it announced one (send [`Outgoing::Udp`] there).
    pub fn udp_port(&self) -> Option<u16> {
        self.udp_port
    }

    /// Mixes 20 ms of every speaker into `out` (interleaved stereo; 2 × [`FRAME_SAMPLES`] samples, overwritten).
    pub fn mix(&mut self, out: &mut [f32]) {
        let mut ids: Vec<PeerId> = self.speakers.keys().copied().collect();
        ids.sort_unstable();
        for id in &ids {
            let s = self.speakers.get_mut(id).expect("listed above");
            let ok = match s.jitter.pop() {
                Playout::Frame(f) => {
                    let r = s.decoder.decode(Some(&f.opus), &mut s.pcm);
                    s.last = Some(f);
                    r.is_ok()
                }
                Playout::Lost => s.decoder.decode(None, &mut s.pcm).is_ok() && s.last.is_some(),
                Playout::Idle => {
                    if let Some(mode) = s.talking.take().filter(|_| !s.jitter.playing() && s.jitter.is_empty()) {
                        self.events.push(VoiceEvent::TalkStop { player: s.player.clone(), mode });
                    }
                    false
                }
            };
            if !ok {
                s.last = None;
            }
        }
        let sources: Vec<Source<'_>> = ids
            .iter()
            .filter_map(|id| {
                let s = &self.speakers[id];
                let f = s.last.as_ref()?;
                Some(Source { speaker: *id, pcm: &s.pcm, gain: f.gain, pan: f.pan, muffle: f.muffle })
            })
            .collect();
        self.mixer.mix(out, &sources);
    }

    /// The events since the last call, for `ModHost::voice_event`.
    pub fn take_events(&mut self) -> Vec<VoiceEvent> {
        std::mem::take(&mut self.events)
    }

    /// True while a mod wants the microphone open.
    pub fn mic_wanted(&self) -> bool {
        self.mic_wanted
    }

    /// The loader reports the microphone device (`voice:mic` to the mods). `denied` also turns the wish off.
    pub fn mic_state(&mut self, state: MicState) {
        if state == MicState::Denied {
            self.mic_wanted = false;
        }
        self.mic = state;
        self.events.push(VoiceEvent::Mic(state));
    }

    /// The mode the microphone transmits in now (the loader draws the mic-on indicator while `Some`).
    pub fn transmitting(&self) -> Option<Mode> {
        self.transmitting.filter(|_| self.mic_wanted).map(mode_of)
    }

    // ---- volumes ----

    fn sync_volumes(&mut self) {
        let v = &mut self.mixer.volumes;
        v.set_master(self.master);
        for (p, id) in &self.peer_of {
            v.set_player(*id, self.volumes.get(p).copied().unwrap_or(100));
            v.set_muted(*id, self.muted.contains(p));
        }
    }

    /// The device file text of the volumes and local mutes: `master <n>` and `player <id> <n> <muted 0|1>` lines.
    pub fn prefs(&self) -> String {
        let mut s = format!("master {}\n", self.master);
        let ids: BTreeSet<&String> = self.volumes.keys().chain(self.muted.iter()).collect();
        for p in ids {
            let v = self.volumes.get(p).copied().unwrap_or(100);
            s.push_str(&format!("player {p} {v} {}\n", u8::from(self.muted.contains(p))));
        }
        s
    }

    /// Reads [`VoiceClient::prefs`] text (lines that do not parse are skipped).
    pub fn load_prefs(&mut self, text: &str) {
        for line in text.lines() {
            match line.split_whitespace().collect::<Vec<_>>()[..] {
                ["master", n] => {
                    if let Ok(n) = n.parse::<u16>() {
                        self.master = n.min(MAX_VOLUME);
                    }
                }
                ["player", p, n, m] => {
                    if let Ok(n) = n.parse::<u16>() {
                        self.volumes.insert(p.to_string(), n.min(MAX_VOLUME));
                        if m == "1" {
                            self.muted.insert(p.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        self.sync_volumes();
    }

    // ---- the mod's controls (VoiceHost) ----

    /// `voice.state` (from the server's `VOICE_STATE`).
    pub fn state(&self, player: &str) -> State {
        self.states.get(player).copied().unwrap_or(State::None)
    }

    /// `voice.icons`.
    pub fn icons(&self) -> Icons {
        self.icons
    }

    /// `voice.set-default-mode`.
    pub fn set_default_mode(&mut self, m: Mode) {
        self.activation.default_mode = voice_mode(m);
    }

    /// `voice.set-activation`: false when the server does not allow it.
    pub fn set_activation(&mut self, a: Act) -> bool {
        let allowed = self.activation.allowed;
        let ok = match a {
            Act::Ptt => allowed.ptt,
            Act::Vad => allowed.vad,
            Act::Toggle => allowed.toggle,
        };
        if ok {
            self.activation.chosen = kind_of(a);
        }
        ok
    }

    /// `voice.talk`: PTT for `mode`; `None` is the default mode, which toggles when toggle is in effect.
    pub fn talk(&mut self, mode: Option<Mode>, pressed: bool) {
        let input = match (mode, self.activation.effective()) {
            (None, Some(Kind::Toggle)) if pressed => Input::Toggle,
            (None, Some(Kind::Toggle)) => return,
            (m, _) => {
                let m = m.map_or(self.activation.default_mode, voice_mode);
                if pressed {
                    Input::PttDown(m)
                } else {
                    Input::PttUp(m)
                }
            }
        };
        self.transmitting = self.activation.handle(input);
    }

    /// `voice.set-mic`.
    pub fn set_mic(&mut self, on: bool) {
        self.mic_wanted = on;
    }

    fn ctrl(&mut self) {
        if self.link.is_some() {
            let c = VoiceCtrl { self_mute: self.self_mute, deafen: self.deafen };
            self.out.push(Outgoing::Reliable(Message::VoiceCtrl(c).encode()));
        }
    }

    /// `voice.set-self-mute`.
    pub fn set_self_mute(&mut self, on: bool) {
        self.self_mute = on;
        self.transmitting = self.activation.handle(Input::SelfMute(on));
        self.ctrl();
    }

    /// `voice.set-deafen`.
    pub fn set_deafen(&mut self, on: bool) {
        self.deafen = on;
        if on {
            self.speakers.clear();
        }
        self.ctrl();
    }

    /// `voice.set-vad-threshold`.
    pub fn set_vad_threshold(&mut self, db: f32) {
        self.vad.config.threshold_db = db;
    }

    /// `voice.set-master`.
    pub fn set_master(&mut self, percent: u16) {
        self.master = percent.min(MAX_VOLUME);
        self.sync_volumes();
    }

    /// `voice.master`.
    pub fn master(&self) -> u16 {
        self.master
    }

    /// `voice.set-player-volume`.
    pub fn set_player_volume(&mut self, player: &str, percent: u16) {
        self.volumes.insert(player.to_string(), percent.min(MAX_VOLUME));
        self.sync_volumes();
    }

    /// `voice.player-volume`.
    pub fn player_volume(&self, player: &str) -> u16 {
        self.volumes.get(player).copied().unwrap_or(100)
    }

    /// `voice.mute-player`.
    pub fn mute_player(&mut self, player: &str, muted: bool) {
        if muted {
            self.muted.insert(player.to_string());
        } else {
            self.muted.remove(player);
        }
        self.sync_volumes();
    }

    /// `voice.player-muted`.
    pub fn player_muted(&self, player: &str) -> bool {
        self.muted.contains(player)
    }

    /// `voice.allowed`: what the server allows this player.
    pub fn allowed(&self) -> Allowance {
        let a = self.activation.allowed;
        let acts = u8::from(a.ptt) | u8::from(a.vad) << 1 | u8::from(a.toggle) << 2;
        Allowance { modes: self.activation.modes, activations: acts, max_bitrate: self.bitrate }
    }

    /// `voice.self-muted`.
    pub fn self_muted(&self) -> bool {
        self.self_mute
    }

    /// `voice.deafened`.
    pub fn deafened(&self) -> bool {
        self.deafen
    }

    /// `voice.mic`: the device state the loader last reported.
    pub fn mic(&self) -> MicState {
        self.mic
    }
}

/// A [`VoiceClient`] shared between the game thread, the bridge ([`VoiceHost`]) and the audio threads.
#[derive(Clone)]
pub struct SharedVoiceClient(pub Arc<Mutex<VoiceClient>>);

impl SharedVoiceClient {
    /// A new shared client.
    pub fn new(client: VoiceClient) -> SharedVoiceClient {
        SharedVoiceClient(Arc::new(Mutex::new(client)))
    }

    /// Locks it (a poisoned lock is taken over).
    pub fn lock(&self) -> MutexGuard<'_, VoiceClient> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl VoiceHost for SharedVoiceClient {
    fn state(&self, player: &str) -> Result<State, ApiError> {
        Ok(self.lock().state(player))
    }
    fn icons(&self) -> Result<Icons, ApiError> {
        Ok(self.lock().icons())
    }
    fn set_default_mode(&self, mode: Mode) -> Result<(), ApiError> {
        self.lock().set_default_mode(mode);
        Ok(())
    }
    fn set_activation(&self, activation: Act) -> Result<(), ApiError> {
        if self.lock().set_activation(activation) {
            return Ok(());
        }
        Err(ApiError::InvalidArgument(format!("the server does not allow {}", activation.name())))
    }
    fn talk(&self, mode: Option<Mode>, pressed: bool) -> Result<(), ApiError> {
        self.lock().talk(mode, pressed);
        Ok(())
    }
    fn set_mic(&self, on: bool) -> Result<(), ApiError> {
        self.lock().set_mic(on);
        Ok(())
    }
    fn set_self_mute(&self, on: bool) -> Result<(), ApiError> {
        self.lock().set_self_mute(on);
        Ok(())
    }
    fn set_deafen(&self, on: bool) -> Result<(), ApiError> {
        self.lock().set_deafen(on);
        Ok(())
    }
    fn set_vad_threshold(&self, db: f32) -> Result<(), ApiError> {
        self.lock().set_vad_threshold(db);
        Ok(())
    }
    fn set_master(&self, percent: u16) -> Result<(), ApiError> {
        self.lock().set_master(percent);
        Ok(())
    }
    fn master(&self) -> Result<u16, ApiError> {
        Ok(self.lock().master())
    }
    fn set_player_volume(&self, player: &str, percent: u16) -> Result<(), ApiError> {
        self.lock().set_player_volume(player, percent);
        Ok(())
    }
    fn player_volume(&self, player: &str) -> Result<u16, ApiError> {
        Ok(self.lock().player_volume(player))
    }
    fn mute_player(&self, player: &str, muted: bool) -> Result<(), ApiError> {
        self.lock().mute_player(player, muted);
        Ok(())
    }
    fn player_muted(&self, player: &str) -> Result<bool, ApiError> {
        Ok(self.lock().player_muted(player))
    }
    fn allowed(&self) -> Result<Allowance, ApiError> {
        Ok(self.lock().allowed())
    }
    fn self_muted(&self) -> Result<bool, ApiError> {
        Ok(self.lock().self_muted())
    }
    fn deafened(&self) -> Result<bool, ApiError> {
        Ok(self.lock().deafened())
    }
    fn mic(&self) -> Result<MicState, ApiError> {
        Ok(self.lock().mic())
    }
}

#[cfg(test)]
mod tests;
