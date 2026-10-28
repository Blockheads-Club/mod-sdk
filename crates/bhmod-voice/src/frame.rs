//! Plaintext layouts of voice frames and their sizes.
//!
//! ```text
//! C->S: mode:u8 seq:u16le opus
//! S->C: speaker:u16le mode:u8 seq:u16le gain:u8 pan:i8 muffle:u8 opus
//! ```
//!
//! On the wire the plaintext is sealed (+16-byte tag) behind an 8-byte nonce and a 3-byte [`crate::udp`] header, in
//! one datagram of the dedicated voice UDP port.

use std::fmt;

use crate::{PeerId, VoiceMode};

/// C->S plaintext header bytes.
pub const C2S_HEADER: usize = 3;
/// S->C plaintext header bytes.
pub const S2C_HEADER: usize = 8;
/// The `nonce:u64le` of a sealed frame.
pub const NONCE_LEN: usize = 8;
/// IPv4 (20) + UDP (8) headers.
pub const IP_UDP_OVERHEAD: usize = 28;
/// Hard cap on the Opus payload of one frame (≈160 kbps for 20 ms); every frame datagram stays far below
/// [`DEFAULT_UDP_MAX`].
pub const MAX_OPUS_BYTES: usize = 400;
/// Largest dedicated-UDP datagram we send or accept (well below a 1280-byte IPv6 minimum MTU path).
pub const DEFAULT_UDP_MAX: usize = 1200;

/// Frame parsing errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Shorter than its header.
    Truncated,
    /// Unknown `mode` byte.
    BadMode(u8),
    /// Opus payload over [`MAX_OPUS_BYTES`].
    TooLarge,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Truncated => write!(f, "voice frame truncated"),
            FrameError::BadMode(m) => write!(f, "unknown voice mode {m}"),
            FrameError::TooLarge => write!(f, "opus payload over {MAX_OPUS_BYTES} bytes"),
        }
    }
}

impl std::error::Error for FrameError {}

fn mode(b: u8) -> Result<VoiceMode, FrameError> {
    VoiceMode::from_u8(b).ok_or(FrameError::BadMode(b))
}

fn opus(b: &[u8]) -> Result<Vec<u8>, FrameError> {
    if b.len() > MAX_OPUS_BYTES {
        return Err(FrameError::TooLarge);
    }
    Ok(b.to_vec())
}

/// A client's frame (C->S plaintext).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientFrame {
    /// The talk mode.
    pub mode: VoiceMode,
    /// The speaker's frame counter.
    pub seq: u16,
    /// One 20 ms Opus packet.
    pub opus: Vec<u8>,
}

impl ClientFrame {
    /// The plaintext bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(C2S_HEADER + self.opus.len());
        b.push(self.mode as u8);
        b.extend(self.seq.to_le_bytes());
        b.extend(&self.opus);
        b
    }

    /// Parses a plaintext.
    pub fn decode(b: &[u8]) -> Result<ClientFrame, FrameError> {
        if b.len() < C2S_HEADER {
            return Err(FrameError::Truncated);
        }
        Ok(ClientFrame { mode: mode(b[0])?, seq: u16::from_le_bytes([b[1], b[2]]), opus: opus(&b[3..])? })
    }
}

/// A relayed frame (S->C plaintext).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFrame {
    /// Who talks.
    pub speaker: PeerId,
    /// The talk mode.
    pub mode: VoiceMode,
    /// The speaker's frame counter.
    pub seq: u16,
    /// Server speaker gain × route gain, percent (0..=200).
    pub gain: u8,
    /// -127 (left) ..= 127 (right).
    pub pan: i8,
    /// 0 (clear) ..= 255 (fully muffled).
    pub muffle: u8,
    /// One 20 ms Opus packet, unchanged.
    pub opus: Vec<u8>,
}

impl ServerFrame {
    /// The plaintext bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(S2C_HEADER + self.opus.len());
        b.extend(self.speaker.to_le_bytes());
        b.push(self.mode as u8);
        b.extend(self.seq.to_le_bytes());
        b.extend([self.gain, self.pan as u8, self.muffle]);
        b.extend(&self.opus);
        b
    }

    /// Parses a plaintext.
    pub fn decode(b: &[u8]) -> Result<ServerFrame, FrameError> {
        if b.len() < S2C_HEADER {
            return Err(FrameError::Truncated);
        }
        Ok(ServerFrame {
            speaker: u16::from_le_bytes([b[0], b[1]]),
            mode: mode(b[2])?,
            seq: u16::from_le_bytes([b[3], b[4]]),
            gain: b[5],
            pan: b[6] as i8,
            muffle: b[7],
            opus: opus(&b[8..])?,
        })
    }
}

/// Direction of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Client to server.
    Up,
    /// Server to client.
    Down,
}

impl Direction {
    /// Plaintext header bytes.
    pub fn header(self) -> usize {
        match self {
            Direction::Up => C2S_HEADER,
            Direction::Down => S2C_HEADER,
        }
    }
}

/// Average Opus bytes of one 20 ms frame at `bitrate` bits per second (CBR), rounded up.
pub fn opus_bytes(bitrate: u32) -> usize {
    (bitrate as usize * crate::FRAME_MS as usize).div_ceil(8000)
}

/// Largest Opus payload a relay accepts under a bitrate cap: twice the CBR size (VBR peaks), never more than
/// [`MAX_OPUS_BYTES`].
pub fn max_opus_bytes(max_bitrate: u32) -> usize {
    (opus_bytes(max_bitrate) * 2).min(MAX_OPUS_BYTES)
}

/// Bytes of a voice datagram for an Opus payload.
pub fn udp_datagram_len(dir: Direction, opus: usize) -> usize {
    crate::udp::FRAME_HEADER + NONCE_LEN + crate::crypto::TAG_LEN + dir.header() + opus
}

/// IP datagram bytes of one voice frame (IPv4 + UDP headers included).
pub fn udp_ip_len(dir: Direction, opus: usize) -> usize {
    udp_datagram_len(dir, opus) + IP_UDP_OVERHEAD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_frame_round_trip() {
        let f = ClientFrame { mode: VoiceMode::Channel, seq: 0x1234, opus: vec![9; 40] };
        let b = f.encode();
        assert_eq!(&b[..3], &[2, 0x34, 0x12]);
        assert_eq!(ClientFrame::decode(&b).unwrap(), f);
        assert_eq!(ClientFrame::decode(&b[..2]), Err(FrameError::Truncated));
        assert_eq!(ClientFrame::decode(&[9, 0, 0]), Err(FrameError::BadMode(9)));
        let mut big = vec![0, 0, 0];
        big.extend([0; MAX_OPUS_BYTES + 1]);
        assert_eq!(ClientFrame::decode(&big), Err(FrameError::TooLarge));
    }

    #[test]
    fn server_frame_round_trip() {
        let f = ServerFrame {
            speaker: 0x0102,
            mode: VoiceMode::Proximity,
            seq: 7,
            gain: 150,
            pan: -100,
            muffle: 30,
            opus: vec![1, 2, 3],
        };
        let b = f.encode();
        assert_eq!(&b[..8], &[2, 1, 0, 7, 0, 150, 156, 30]);
        assert_eq!(ServerFrame::decode(&b).unwrap(), f);
        assert_eq!(ServerFrame::decode(&b[..7]), Err(FrameError::Truncated));
        let mut bad = b.clone();
        bad[2] = 4;
        assert_eq!(ServerFrame::decode(&bad), Err(FrameError::BadMode(4)));
        for (e, s) in
            [(FrameError::Truncated, "truncated"), (FrameError::BadMode(4), "4"), (FrameError::TooLarge, "400")]
        {
            assert!(e.to_string().contains(s));
        }
        let _: &dyn std::error::Error = &FrameError::TooLarge;
    }

    #[test]
    fn sizes() {
        // 20 ms CBR frames at 16/24/32 kbps: datagrams C->S 98/118/138, S->C 103/123/143 bytes on the IP layer.
        let rates = [16_000, 24_000, 32_000];
        let opus: Vec<usize> = rates.iter().map(|r| opus_bytes(*r)).collect();
        assert_eq!(opus, [40, 60, 80]);
        let up: Vec<usize> = opus.iter().map(|o| udp_ip_len(Direction::Up, *o)).collect();
        let down: Vec<usize> = opus.iter().map(|o| udp_ip_len(Direction::Down, *o)).collect();
        assert_eq!(up, [98, 118, 138]);
        assert_eq!(down, [103, 123, 143]);
        assert_eq!(udp_datagram_len(Direction::Down, 80), 115);
        assert!(udp_datagram_len(Direction::Down, MAX_OPUS_BYTES) < DEFAULT_UDP_MAX);
        assert_eq!(max_opus_bytes(24_000), 120);
        assert_eq!(max_opus_bytes(1_000_000), MAX_OPUS_BYTES);
    }
}
