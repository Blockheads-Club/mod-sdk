//! The client end of a voice session: seals the microphone frames, opens relayed ones and follows the same path
//! policy as the server.

use crate::crypto::{CryptoError, Session};
use crate::frame::{ClientFrame, Direction, FrameError, ServerFrame};
use crate::path::{Path, PathSelector, TransportMode};
use crate::relay::wrap;
use crate::udp::Datagram;
use crate::{PeerId, VoiceMode};

/// Why an incoming frame was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    /// Did not authenticate or replayed.
    Crypto(CryptoError),
    /// Malformed plaintext.
    Frame(FrameError),
}

/// The client's voice link.
pub struct ClientLink {
    session: Session,
    peer: PeerId,
    selector: PathSelector,
    seq: u16,
}

impl ClientLink {
    /// A link after `VOICE_KEY`: `peer` and `udp_port` come from the server's key message; `mode` is the transport
    /// the server announced (`auto` when unknown).
    pub fn new(session: Session, peer: PeerId, mode: TransportMode, udp_port: Option<u16>) -> ClientLink {
        ClientLink { session, peer, selector: PathSelector::new(mode, udp_port.is_some()), seq: 0 }
    }

    /// Our peer id.
    pub fn peer(&self) -> PeerId {
        self.peer
    }

    /// The path policy (set `inband_capable = false` when this client cannot send unsequenced in-band).
    pub fn selector_mut(&mut self) -> &mut PathSelector {
        &mut self.selector
    }

    /// The UDP `AUTH` datagram (send first, then every [`crate::udp::KEEPALIVE_MS`]).
    pub fn auth_datagram(&self) -> Vec<u8> {
        Datagram::Auth { peer: self.peer, token: self.session.udp_token(self.peer) }.encode()
    }

    /// Handles a datagram from the voice port: `AUTH_OK` enables UDP and returns `None`; a frame is opened.
    pub fn receive_udp(&mut self, datagram: &[u8]) -> Option<Result<ServerFrame, LinkError>> {
        match Datagram::decode(datagram, crate::frame::DEFAULT_UDP_MAX)? {
            Datagram::AuthOk { peer } if peer == self.peer => {
                self.selector.udp_ready = true;
                None
            }
            Datagram::Frame { peer, nonce, sealed } if peer == self.peer => Some(self.open(nonce, &sealed)),
            _ => None,
        }
    }

    /// Seals one Opus frame; returns the path and the packet (0xBC packet or datagram), or `None` when it cannot
    /// be sent (too big for every allowed path). `loss` is the host's measured loss of relayed frames (e.g. the
    /// worst [`crate::jitter::JitterBuffer::loss`]), used by `auto`.
    pub fn send(&mut self, mode: VoiceMode, opus: &[u8], loss: f32) -> Option<(Path, Vec<u8>)> {
        let path = self.selector.choose(Direction::Up, opus.len(), loss, 0)?;
        let plain = ClientFrame { mode, seq: self.seq, opus: opus.to_vec() }.encode();
        let (n, sealed) = self.session.seal(&plain).ok()?;
        self.seq = self.seq.wrapping_add(1);
        Some((path, wrap(path, self.peer, n, sealed)))
    }

    /// Opens a relayed frame (in-band `VOICE_FRAME` fields or a UDP frame).
    pub fn open(&mut self, nonce: u64, sealed: &[u8]) -> Result<ServerFrame, LinkError> {
        let plain = self.session.open(nonce, sealed).map_err(LinkError::Crypto)?;
        ServerFrame::decode(&plain).map_err(LinkError::Frame)
    }
}
