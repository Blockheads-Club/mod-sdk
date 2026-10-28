//! Transport-agnostic voice chat core shared by the Blockheads Club hosts (server, PC, Android).
//!
//! Pure and deterministic: no audio devices, no sockets, no clocks (callers pass `now` in milliseconds). The wire
//! format is the 0xBC `VOICE_*` ops of `docs/protocol-0xbc.md` plus the voice UDP datagrams of [`udp`].
//!
//! - [`crypto`]: X25519 + HKDF-SHA256 session keys, ChaCha20-Poly1305 frames with a replay window, UDP auth token.
//! - [`frame`]: plaintext frame layouts and sizes.
//! - [`udp`]: datagrams of the dedicated voice UDP port, the only path of voice frames (maintainer decision
//!   2026-10-28); the control ops stay on reliable 0xBC.
//! - [`relay`]: the server relay (route table, mutes, speaker gain, caps). It never decodes audio.
//! - [`jitter`], [`mix`], [`vad`], [`activation`], [`codec`]: client pipeline pieces.
//! - [`server`] and [`client`] (API 0.3.4): the host-agnostic halves the server and the loaders glue in, including
//!   the [`bhmod_core::VoiceHost`] seam of the voice mod API.
#![warn(missing_docs)]

pub mod activation;
pub mod client;
pub mod codec;
pub mod crypto;
pub mod frame;
pub mod jitter;
pub mod link;
pub mod mix;
pub mod relay;
pub mod server;
pub mod udp;
pub mod vad;

/// Sample rate of capture, codec and mixing.
pub const SAMPLE_RATE: u32 = 48_000;
/// Length of one voice frame in milliseconds.
pub const FRAME_MS: u32 = 20;
/// Mono samples in one frame (20 ms at 48 kHz).
pub const FRAME_SAMPLES: usize = 960;

/// A speaker / listener id on the voice wire (`VOICE_KEY.peer`, the `speaker` of S->C frames).
pub type PeerId = u16;

/// The talk mode a frame was sent in (`mode` byte of a frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum VoiceMode {
    /// Players nearby, with distance falloff, pan and muffling.
    Proximity = 0,
    /// Everyone.
    Global = 1,
    /// The speaker's channel (team).
    Channel = 2,
    /// Admin broadcast.
    Broadcast = 3,
}

impl VoiceMode {
    /// All modes, in wire order.
    pub const ALL: [VoiceMode; 4] = [VoiceMode::Proximity, VoiceMode::Global, VoiceMode::Channel, VoiceMode::Broadcast];

    /// Maps the wire byte to a mode.
    pub fn from_u8(b: u8) -> Option<VoiceMode> {
        VoiceMode::ALL.get(usize::from(b)).copied()
    }

    /// Bit of this mode in a [`relay::Route::modes`] mask.
    pub fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        for (i, m) in VoiceMode::ALL.iter().enumerate() {
            assert_eq!(VoiceMode::from_u8(i as u8), Some(*m));
            assert_eq!(m.bit(), 1 << i);
        }
        assert_eq!(VoiceMode::from_u8(4), None);
        assert_eq!(FRAME_SAMPLES as u32, SAMPLE_RATE / 1000 * FRAME_MS);
    }
}
