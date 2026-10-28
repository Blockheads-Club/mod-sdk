//! The client end of a voice session: seals the microphone frames into voice UDP datagrams and opens relayed ones.

use crate::crypto::{CryptoError, Session};
use crate::frame::{ClientFrame, FrameError, ServerFrame};
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

/// The client's voice link on the server's voice UDP port.
pub struct ClientLink {
    session: Session,
    peer: PeerId,
    authed: bool,
    seq: u16,
}

impl ClientLink {
    /// A link after `VOICE_KEY`: `peer` comes from the server's key message.
    pub fn new(session: Session, peer: PeerId) -> ClientLink {
        ClientLink { session, peer, authed: false, seq: 0 }
    }

    /// Our peer id.
    pub fn peer(&self) -> PeerId {
        self.peer
    }

    /// True once the server confirmed our address (`AUTH_OK`); frames are sent only after that.
    pub fn authed(&self) -> bool {
        self.authed
    }

    /// The UDP `AUTH` datagram (send first, then every [`crate::udp::KEEPALIVE_MS`]).
    pub fn auth_datagram(&self) -> Vec<u8> {
        Datagram::Auth { peer: self.peer, token: self.session.udp_token(self.peer) }.encode()
    }

    /// Handles a datagram from the voice port: `AUTH_OK` enables sending and returns `None`; a frame is opened.
    pub fn receive_udp(&mut self, datagram: &[u8]) -> Option<Result<ServerFrame, LinkError>> {
        match Datagram::decode(datagram, crate::frame::DEFAULT_UDP_MAX)? {
            Datagram::AuthOk { peer } if peer == self.peer => {
                self.authed = true;
                None
            }
            Datagram::Frame { peer, nonce, sealed } if peer == self.peer => Some(self.open(nonce, &sealed)),
            _ => None,
        }
    }

    /// Seals one Opus frame into a datagram; `None` before `AUTH_OK` or when sealing fails.
    pub fn send(&mut self, mode: VoiceMode, opus: &[u8]) -> Option<Vec<u8>> {
        if !self.authed {
            return None;
        }
        let plain = ClientFrame { mode, seq: self.seq, opus: opus.to_vec() }.encode();
        let (nonce, sealed) = self.session.seal(&plain).ok()?;
        self.seq = self.seq.wrapping_add(1);
        Some(Datagram::Frame { peer: self.peer, nonce, sealed }.encode())
    }

    fn open(&mut self, nonce: u64, sealed: &[u8]) -> Result<ServerFrame, LinkError> {
        let plain = self.session.open(nonce, sealed).map_err(LinkError::Crypto)?;
        ServerFrame::decode(&plain).map_err(LinkError::Frame)
    }
}
