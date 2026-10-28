//! The codec seam. Hosts plug in Opus (the vendored libopus of `bhmod-opus-sys`, shared by PC and Android); tests use
//! [`TestCodec`]. All PCM is mono `f32` at 48 kHz, one call per 20 ms frame ([`crate::FRAME_SAMPLES`]).

use std::fmt;

use crate::FRAME_SAMPLES;

/// Codec failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// The input is not one frame long.
    FrameSize(usize),
    /// The packet does not decode.
    Corrupt,
    /// The codec library failed (its message).
    Backend(String),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::FrameSize(n) => write!(f, "expected {FRAME_SAMPLES} samples, got {n}"),
            CodecError::Corrupt => write!(f, "corrupt packet"),
            CodecError::Backend(m) => write!(f, "codec: {m}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Encodes the microphone stream.
pub trait Encoder: Send {
    /// Encodes one frame of [`FRAME_SAMPLES`] samples; replaces `out` with the packet.
    fn encode(&mut self, pcm: &[f32], out: &mut Vec<u8>) -> Result<(), CodecError>;
    /// Changes the target bitrate (bits per second).
    fn set_bitrate(&mut self, bitrate: u32);
}

/// Decodes one speaker's stream.
pub trait Decoder: Send {
    /// Decodes one packet into `out` ([`FRAME_SAMPLES`] samples). `None` asks for loss concealment of a missing
    /// frame ([`crate::jitter::Playout::Lost`]).
    fn decode(&mut self, packet: Option<&[u8]>, out: &mut [f32]) -> Result<(), CodecError>;
}

/// A codec: makes encoders and decoders.
pub trait Codec: Send + Sync {
    /// A new encoder at `bitrate` bits per second.
    fn encoder(&self, bitrate: u32) -> Result<Box<dyn Encoder>, CodecError>;
    /// A new decoder.
    fn decoder(&self) -> Result<Box<dyn Decoder>, CodecError>;
}

/// A tiny deterministic lossy codec for tests: one byte per `step` samples (i8 of the average), `step` chosen from
/// the bitrate so packet sizes scale like Opus. Concealment repeats the last frame at half level.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestCodec;

/// Samples per byte for a bitrate: 960 samples in `bitrate / 400` bytes.
fn step(bitrate: u32) -> usize {
    let bytes = crate::frame::opus_bytes(bitrate).clamp(1, FRAME_SAMPLES);
    FRAME_SAMPLES / bytes
}

struct TestEncoder {
    step: usize,
}

impl Encoder for TestEncoder {
    fn encode(&mut self, pcm: &[f32], out: &mut Vec<u8>) -> Result<(), CodecError> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(CodecError::FrameSize(pcm.len()));
        }
        out.clear();
        out.push(self.step as u8);
        for c in pcm.chunks(self.step) {
            let avg = c.iter().sum::<f32>() / c.len() as f32;
            out.push((avg.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8);
        }
        Ok(())
    }

    fn set_bitrate(&mut self, bitrate: u32) {
        self.step = step(bitrate);
    }
}

#[derive(Default)]
struct TestDecoder {
    last: Vec<f32>,
}

impl Decoder for TestDecoder {
    fn decode(&mut self, packet: Option<&[u8]>, out: &mut [f32]) -> Result<(), CodecError> {
        if out.len() != FRAME_SAMPLES {
            return Err(CodecError::FrameSize(out.len()));
        }
        let Some(p) = packet else {
            self.last.iter_mut().for_each(|x| *x *= 0.5);
            self.last.resize(FRAME_SAMPLES, 0.0);
            out.copy_from_slice(&self.last);
            return Ok(());
        };
        let (&s, body) = p.split_first().ok_or(CodecError::Corrupt)?;
        let s = usize::from(s);
        if s == 0 || body.len() != FRAME_SAMPLES.div_ceil(s) {
            return Err(CodecError::Corrupt);
        }
        for (i, x) in out.iter_mut().enumerate() {
            *x = f32::from(body[i / s] as i8) / 127.0;
        }
        self.last = out.to_vec();
        Ok(())
    }
}

impl Codec for TestCodec {
    fn encoder(&self, bitrate: u32) -> Result<Box<dyn Encoder>, CodecError> {
        Ok(Box::new(TestEncoder { step: step(bitrate) }))
    }

    fn decoder(&self) -> Result<Box<dyn Decoder>, CodecError> {
        Ok(Box::new(TestDecoder::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_sizes() {
        let c = TestCodec;
        let mut e = c.encoder(24_000).unwrap();
        let mut d = c.decoder().unwrap();
        let pcm = vec![0.5f32; FRAME_SAMPLES];
        let mut pkt = Vec::new();
        e.encode(&pcm, &mut pkt).unwrap();
        assert_eq!(pkt.len(), 61, "60 bytes at 24 kbps + the step byte");
        e.set_bitrate(16_000);
        e.encode(&pcm, &mut pkt).unwrap();
        assert_eq!(pkt.len(), 41);
        let mut out = vec![0.0; FRAME_SAMPLES];
        d.decode(Some(&pkt), &mut out).unwrap();
        assert!((out[0] - 64.0 / 127.0).abs() < 1e-6);
        d.decode(None, &mut out).unwrap();
        assert!((out[0] - 32.0 / 127.0).abs() < 1e-6, "concealment halves");
    }

    #[test]
    fn errors() {
        let c = TestCodec;
        let mut e = c.encoder(0).unwrap();
        let mut d = c.decoder().unwrap();
        let mut pkt = Vec::new();
        assert_eq!(e.encode(&[0.0; 10], &mut pkt), Err(CodecError::FrameSize(10)));
        let mut out = vec![0.0; FRAME_SAMPLES];
        d.decode(None, &mut out).unwrap();
        assert!(out.iter().all(|x| *x == 0.0), "concealment before any frame is silence");
        assert_eq!(d.decode(Some(&[]), &mut out), Err(CodecError::Corrupt));
        assert_eq!(d.decode(Some(&[0]), &mut out), Err(CodecError::Corrupt));
        assert_eq!(d.decode(Some(&[24, 1]), &mut out), Err(CodecError::Corrupt));
        assert_eq!(d.decode(None, &mut [0.0; 3]), Err(CodecError::FrameSize(3)));
        for (e, s) in [
            (CodecError::FrameSize(3), "960"),
            (CodecError::Corrupt, "corrupt"),
            (CodecError::Backend("x".into()), "codec: x"),
        ] {
            assert!(e.to_string().contains(s));
        }
        let _: &dyn std::error::Error = &CodecError::Corrupt;
    }
}
