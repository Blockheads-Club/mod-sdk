//! Voice session crypto (spec: `docs/protocol-0xbc.md`, `VOICE_KEY` / `VOICE_FRAME`).
//!
//! ```text
//! shared    = X25519(own secret, peer pub)                 (all-zero: rejected, low-order key)
//! prk       = HKDF-SHA256-Extract(salt = "bhvoice-v1", shared)
//! c2s / s2c = HKDF-Expand(prk, "bhvoice c2s" / "bhvoice s2c" || client pub || server pub, 32)
//! frame     = ChaCha20-Poly1305(key of the direction, nonce = 0u32 || counter:u64le, no AAD)
//! udp token = HMAC-SHA256(c2s, "bhvoice-udp" || peer:u16le)
//! ```

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::PeerId;

/// Bytes of a Poly1305 tag.
pub const TAG_LEN: usize = 16;
/// Bytes of the UDP auth token.
pub const TOKEN_LEN: usize = 32;
/// Frames the replay window remembers below the highest nonce seen.
pub const REPLAY_WINDOW: u64 = 64;

const SALT: &[u8] = b"bhvoice-v1";
const TOKEN_LABEL: &[u8] = b"bhvoice-udp";

/// Crypto failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// The peer's public key gives an all-zero shared secret (low-order point).
    WeakKey,
    /// The operating system has no randomness.
    NoRandom,
    /// The frame does not open with this key (forged, corrupted or for another session).
    Auth,
    /// The nonce was already seen or is older than the replay window.
    Replay,
    /// The sending counter ran out (2^64 frames).
    NonceExhausted,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CryptoError::WeakKey => "weak peer key",
            CryptoError::NoRandom => "no randomness",
            CryptoError::Auth => "frame does not authenticate",
            CryptoError::Replay => "replayed or too old frame",
            CryptoError::NonceExhausted => "nonce counter exhausted",
        })
    }
}

impl std::error::Error for CryptoError {}

/// Which end of the session this side is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The game client (sends c2s, receives s2c).
    Client,
    /// The server relay (sends s2c, receives c2s).
    Server,
}

/// An X25519 key pair, fresh per connection.
pub struct KeyPair {
    secret: StaticSecret,
    public: [u8; 32],
}

impl KeyPair {
    /// A key pair from 32 secret bytes (deterministic; tests and callers with their own RNG).
    pub fn from_secret(secret: [u8; 32]) -> KeyPair {
        let secret = StaticSecret::from(secret);
        let public = PublicKey::from(&secret).to_bytes();
        KeyPair { secret, public }
    }

    /// A random key pair from the operating system's RNG.
    pub fn generate() -> Result<KeyPair, CryptoError> {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).map_err(|_| CryptoError::NoRandom)?;
        Ok(KeyPair::from_secret(b))
    }

    /// The public key for `VOICE_KEY.pub`.
    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    /// Derives the session with the peer's public key. `role` is this side's role.
    pub fn session(&self, role: Role, peer_public: &[u8; 32]) -> Result<Session, CryptoError> {
        let shared = self.secret.diffie_hellman(&PublicKey::from(*peer_public));
        if !shared.was_contributory() {
            return Err(CryptoError::WeakKey);
        }
        let (client, server) = match role {
            Role::Client => (self.public, *peer_public),
            Role::Server => (*peer_public, self.public),
        };
        let hk = Hkdf::<Sha256>::new(Some(SALT), shared.as_bytes());
        let key = |label: &[u8]| {
            let mut info = label.to_vec();
            info.extend(client);
            info.extend(server);
            let mut k = [0u8; 32];
            hk.expand(&info, &mut k).expect("32 bytes is a valid HKDF length");
            k
        };
        let (c2s, s2c) = (key(b"bhvoice c2s"), key(b"bhvoice s2c"));
        let (tx, rx) = match role {
            Role::Client => (c2s, s2c),
            Role::Server => (s2c, c2s),
        };
        Ok(Session { tx: Sealer::new(&tx), rx: Opener::new(&rx), c2s })
    }
}

/// The receive-side replay window: accepts each nonce once, up to [`REPLAY_WINDOW`] behind the highest seen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayWindow {
    top: u64,
    bits: u64,
}

impl ReplayWindow {
    /// True if `n` would be accepted (nonce 0 is never valid).
    pub fn check(&self, n: u64) -> bool {
        if n == 0 {
            return false;
        }
        if n > self.top {
            return true;
        }
        let back = self.top - n;
        back < REPLAY_WINDOW && self.bits & (1 << back) == 0
    }

    /// Marks `n` as seen (call only after the frame authenticated).
    pub fn commit(&mut self, n: u64) {
        if n > self.top {
            let shift = n - self.top;
            self.bits = if shift >= REPLAY_WINDOW { 0 } else { self.bits << shift };
            self.top = n;
            self.bits |= 1;
        } else {
            self.bits |= 1 << (self.top - n);
        }
    }

    /// The highest nonce accepted so far (0: none).
    pub fn top(&self) -> u64 {
        self.top
    }
}

fn nonce(n: u64) -> Nonce {
    let mut b = [0u8; 12];
    b[4..].copy_from_slice(&n.to_le_bytes());
    Nonce::from(b)
}

struct Sealer {
    aead: ChaCha20Poly1305,
    next: u64,
}

impl Sealer {
    fn new(key: &[u8; 32]) -> Sealer {
        Sealer { aead: ChaCha20Poly1305::new(Key::from_slice(key)), next: 1 }
    }
}

struct Opener {
    aead: ChaCha20Poly1305,
    window: ReplayWindow,
}

impl Opener {
    fn new(key: &[u8; 32]) -> Opener {
        Opener { aead: ChaCha20Poly1305::new(Key::from_slice(key)), window: ReplayWindow::default() }
    }
}

/// One side of a voice session: seals outgoing frames and opens incoming ones.
pub struct Session {
    tx: Sealer,
    rx: Opener,
    c2s: [u8; 32],
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("next", &self.tx.next).field("window", &self.rx.window).finish()
    }
}

impl Session {
    /// Seals a plaintext: returns the nonce and the ciphertext with its tag (`VOICE_FRAME` fields).
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<(u64, Vec<u8>), CryptoError> {
        let n = self.tx.next;
        if n == u64::MAX {
            return Err(CryptoError::NonceExhausted);
        }
        let sealed = self.tx.aead.encrypt(&nonce(n), plaintext).expect("ChaCha20-Poly1305 seals any size we send");
        self.tx.next += 1;
        Ok((n, sealed))
    }

    /// Opens an incoming frame, rejecting replays and forgeries; the window advances only on success.
    pub fn open(&mut self, n: u64, sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if !self.rx.window.check(n) {
            return Err(CryptoError::Replay);
        }
        let plain = self.rx.aead.decrypt(&nonce(n), sealed).map_err(|_| CryptoError::Auth)?;
        self.rx.window.commit(n);
        Ok(plain)
    }

    /// The UDP auth token of `peer` for this session (both sides compute the same value).
    pub fn udp_token(&self, peer: PeerId) -> [u8; TOKEN_LEN] {
        self.token_mac(peer).finalize().into_bytes().into()
    }

    /// Checks a UDP auth token in constant time.
    pub fn verify_udp_token(&self, peer: PeerId, token: &[u8]) -> bool {
        self.token_mac(peer).verify_slice(token).is_ok()
    }

    fn token_mac(&self, peer: PeerId) -> Hmac<Sha256> {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.c2s).expect("HMAC takes any key length");
        m.update(TOKEN_LABEL);
        m.update(&peer.to_le_bytes());
        m
    }

    /// The next nonce this side will send.
    pub fn next_nonce(&self) -> u64 {
        self.tx.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn pair() -> (Session, Session) {
        let c = KeyPair::from_secret([1; 32]);
        let s = KeyPair::from_secret([2; 32]);
        (c.session(Role::Client, &s.public()).unwrap(), s.session(Role::Server, &c.public()).unwrap())
    }

    #[test]
    fn round_trip_both_directions() {
        let (mut c, mut s) = pair();
        let (n, sealed) = c.seal(b"hello").unwrap();
        assert_eq!((n, sealed.len()), (1, 5 + TAG_LEN));
        assert_eq!(s.open(n, &sealed).unwrap(), b"hello");
        let (n, sealed) = s.seal(b"back").unwrap();
        assert_eq!(c.open(n, &sealed).unwrap(), b"back");
        assert_eq!(c.next_nonce(), 2);
        // Directions use different keys: a client frame does not open as a server frame on the client.
        let (n, sealed) = c.seal(b"x").unwrap();
        assert_eq!(c.open(n, &sealed), Err(CryptoError::Auth));
        assert!(format!("{c:?}").contains("Session"));
    }

    #[test]
    fn replay_and_forgery() {
        let (mut c, mut s) = pair();
        let frames: Vec<_> = (0..3).map(|_| c.seal(b"f").unwrap()).collect();
        assert!(s.open(frames[2].0, &frames[2].1).is_ok());
        assert!(s.open(frames[0].0, &frames[0].1).is_ok(), "reordered within the window");
        assert_eq!(s.open(frames[0].0, &frames[0].1), Err(CryptoError::Replay));
        let mut bad = frames[1].1.clone();
        bad[0] ^= 1;
        assert_eq!(s.open(frames[1].0, &bad), Err(CryptoError::Auth));
        assert!(s.open(frames[1].0, &frames[1].1).is_ok(), "a forgery does not burn the nonce");
        assert_eq!(s.open(0, &frames[1].1), Err(CryptoError::Replay));
    }

    #[test]
    fn window_edges() {
        let mut w = ReplayWindow::default();
        assert!(!w.check(0));
        w.commit(100);
        assert_eq!(w.top(), 100);
        assert!(w.check(37) && !w.check(36), "63 back is inside, 64 back is outside");
        w.commit(40);
        assert!(!w.check(40));
        w.commit(101);
        assert!(!w.check(40) && !w.check(100) && w.check(99));
        w.commit(1000);
        assert!(w.check(999) && !w.check(1000) && !w.check(101));
    }

    #[test]
    fn tokens() {
        let (c, s) = pair();
        let t = c.udp_token(7);
        assert_eq!(t, s.udp_token(7));
        assert!(s.verify_udp_token(7, &t));
        assert!(!s.verify_udp_token(8, &t));
        assert!(!s.verify_udp_token(7, &t[..31]));
        let (other, _) = {
            let c = KeyPair::from_secret([3; 32]);
            let s = KeyPair::from_secret([2; 32]);
            (c.session(Role::Client, &s.public()).unwrap(), ())
        };
        assert_ne!(other.udp_token(7), t);
    }

    #[test]
    fn weak_key_and_exhaustion() {
        let k = KeyPair::generate().unwrap();
        assert_eq!(k.session(Role::Server, &[0; 32]).unwrap_err(), CryptoError::WeakKey);
        let (mut c, _) = pair();
        c.tx.next = u64::MAX;
        assert_eq!(c.seal(b"x"), Err(CryptoError::NonceExhausted));
        for (e, s) in [
            (CryptoError::WeakKey, "weak"),
            (CryptoError::NoRandom, "random"),
            (CryptoError::Auth, "authenticate"),
            (CryptoError::Replay, "replay"),
            (CryptoError::NonceExhausted, "exhausted"),
        ] {
            assert!(e.to_string().contains(s));
        }
        let _: &dyn std::error::Error = &CryptoError::Auth;
    }

    #[test]
    fn known_answer() {
        // Pins the key schedule: changing it breaks interop with other implementations.
        let (mut c, _) = pair();
        let (_, sealed) = c.seal(&[]).unwrap();
        let hex: String = sealed.iter().map(|b| format!("{b:02x}")).collect();
        let token: String = c.udp_token(7).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex.len(), 32);
        assert_eq!(token.len(), 64);
        assert_eq!((hex, token), (KAT_TAG.to_string(), KAT_TOKEN.to_string()));
    }

    const KAT_TAG: &str = "22575c995747b0e5518cb8a3bc7ef135";
    const KAT_TOKEN: &str = "b28a905d5c765810f108f03c0070995ef106acbbf706a5021010c34363869cbf";
}
