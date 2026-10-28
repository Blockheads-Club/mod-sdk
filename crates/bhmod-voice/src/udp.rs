//! Datagrams of the dedicated voice UDP port (fallback transport, `voiceUdpPort`).
//!
//! ```text
//! 01 AUTH    C->S  peer:u16le token[32]     first datagram, repeated every KEEPALIVE_MS (NAT keepalive)
//! 02 AUTH_OK S->C  peer:u16le               the server learned the client's address
//! 03 FRAME   both  peer:u16le nonce:u64le sealed
//! ```
//!
//! `peer` is the client's id from `VOICE_KEY` (sender C->S, recipient S->C). `nonce || sealed` are the exact bytes of
//! an in-band `VOICE_FRAME` body, sharing its counter and replay window.

use crate::crypto::{TAG_LEN, TOKEN_LEN};
use crate::PeerId;

/// Bytes before the nonce of a `FRAME` datagram (kind + peer).
pub const FRAME_HEADER: usize = 3;
/// How often a client resends `AUTH` to keep NAT bindings open.
pub const KEEPALIVE_MS: u64 = 15_000;

const AUTH: u8 = 1;
const AUTH_OK: u8 = 2;
const FRAME: u8 = 3;

/// One voice UDP datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Datagram {
    /// Client proves it holds the session ([`crate::crypto::Session::udp_token`]).
    Auth {
        /// The client's peer id.
        peer: PeerId,
        /// HMAC token.
        token: [u8; TOKEN_LEN],
    },
    /// Server confirms the address.
    AuthOk {
        /// The client's peer id.
        peer: PeerId,
    },
    /// A sealed voice frame.
    Frame {
        /// The client's peer id.
        peer: PeerId,
        /// AEAD nonce.
        nonce: u64,
        /// Ciphertext + tag.
        sealed: Vec<u8>,
    },
}

impl Datagram {
    /// The datagram bytes.
    pub fn encode(&self) -> Vec<u8> {
        let (kind, peer) = match self {
            Datagram::Auth { peer, .. } => (AUTH, peer),
            Datagram::AuthOk { peer } => (AUTH_OK, peer),
            Datagram::Frame { peer, .. } => (FRAME, peer),
        };
        let mut b = vec![kind];
        b.extend(peer.to_le_bytes());
        match self {
            Datagram::Auth { token, .. } => b.extend(token),
            Datagram::AuthOk { .. } => {}
            Datagram::Frame { nonce, sealed, .. } => {
                b.extend(nonce.to_le_bytes());
                b.extend(sealed);
            }
        }
        b
    }

    /// Parses a datagram; `None` for anything malformed (drop it silently).
    pub fn decode(b: &[u8], max_len: usize) -> Option<Datagram> {
        if b.len() < FRAME_HEADER || b.len() > max_len {
            return None;
        }
        let peer = u16::from_le_bytes([b[1], b[2]]);
        let rest = &b[FRAME_HEADER..];
        match b[0] {
            AUTH => Some(Datagram::Auth { peer, token: rest.try_into().ok()? }),
            AUTH_OK if rest.is_empty() => Some(Datagram::AuthOk { peer }),
            FRAME if rest.len() >= 8 + TAG_LEN => Some(Datagram::Frame {
                peer,
                nonce: u64::from_le_bytes(rest[..8].try_into().expect("8 bytes")),
                sealed: rest[8..].to_vec(),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for d in [
            Datagram::Auth { peer: 7, token: [5; 32] },
            Datagram::AuthOk { peer: 0x0102 },
            Datagram::Frame { peer: 3, nonce: 9, sealed: vec![1; 16] },
        ] {
            assert_eq!(Datagram::decode(&d.encode(), 1200), Some(d));
        }
        assert_eq!(&Datagram::AuthOk { peer: 0x0102 }.encode(), &[2, 2, 1]);
    }

    #[test]
    fn rejects() {
        let frame = Datagram::Frame { peer: 3, nonce: 9, sealed: vec![1; 16] }.encode();
        assert_eq!(Datagram::decode(&frame, frame.len() - 1), None);
        assert_eq!(Datagram::decode(&frame[..frame.len() - 1], 1200), None);
        assert_eq!(Datagram::decode(&[1, 0], 1200), None);
        assert_eq!(Datagram::decode(&[1, 0, 0, 1], 1200), None);
        assert_eq!(Datagram::decode(&[2, 0, 0, 1], 1200), None);
        assert_eq!(Datagram::decode(&[9, 0, 0], 1200), None);
    }
}
