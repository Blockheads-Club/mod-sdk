// SPDX-License-Identifier: GPL-3.0-or-later
//! [`OpusCodec`]: libopus behind the `bhmod_voice::codec::Codec` seam the voice client uses (API 0.3.4). It lives
//! here, not in `bhmod-voice`, so the server (which relays without decoding) never builds libopus.
//!
//! PCM is `f32` in -1..=1 on the voice side and `i16` for libopus; frames are 20 ms at 48 kHz mono. The encoder
//! uses the VoIP preset, VBR, in-band FEC and 10 % expected loss.

use bhmod_voice::codec::{Codec, CodecError, Decoder as VDecoder, Encoder as VEncoder};

use crate::{Application, Decoder, Encoder, Error, FRAME_SAMPLES, MAX_PACKET};

fn backend(e: Error) -> CodecError {
    CodecError::Backend(e.to_string())
}

/// libopus as a voice codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpusCodec;

struct OpusEncoder {
    enc: Encoder,
    pcm: Vec<i16>,
}

struct OpusDecoder {
    dec: Decoder,
    pcm: Vec<i16>,
}

/// `f32` -1..=1 to `i16` (clamped).
pub fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

impl VEncoder for OpusEncoder {
    fn encode(&mut self, pcm: &[f32], out: &mut Vec<u8>) -> Result<(), CodecError> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(CodecError::FrameSize(pcm.len()));
        }
        self.pcm.clear();
        self.pcm.extend(pcm.iter().map(|x| to_i16(*x)));
        out.resize(MAX_PACKET, 0);
        let n = self.enc.encode(&self.pcm, out).map_err(backend)?;
        out.truncate(n);
        Ok(())
    }

    fn set_bitrate(&mut self, bitrate: u32) {
        // Out-of-range values are clamped by the voice side's caps; a refusal keeps the old rate.
        let _ = self.enc.set_bitrate(bitrate.min(512_000) as i32);
    }
}

impl VDecoder for OpusDecoder {
    fn decode(&mut self, packet: Option<&[u8]>, out: &mut [f32]) -> Result<(), CodecError> {
        if out.len() != FRAME_SAMPLES {
            return Err(CodecError::FrameSize(out.len()));
        }
        let n = match packet {
            Some(p) => self.dec.decode(p, &mut self.pcm),
            None => self.dec.decode_lost(&mut self.pcm),
        }
        .map_err(|_| CodecError::Corrupt)?;
        if n != FRAME_SAMPLES {
            return Err(CodecError::FrameSize(n));
        }
        for (o, s) in out.iter_mut().zip(&self.pcm) {
            *o = f32::from(*s) / 32768.0;
        }
        Ok(())
    }
}

impl Codec for OpusCodec {
    fn encoder(&self, bitrate: u32) -> Result<Box<dyn VEncoder>, CodecError> {
        let mut enc = Encoder::new(Application::Voip).map_err(backend)?;
        enc.set_inband_fec(true).map_err(backend)?;
        enc.set_packet_loss_perc(10).map_err(backend)?;
        let mut e = OpusEncoder { enc, pcm: Vec::with_capacity(FRAME_SAMPLES) };
        e.set_bitrate(bitrate);
        Ok(Box::new(e))
    }

    fn decoder(&self) -> Result<Box<dyn VDecoder>, CodecError> {
        let dec = Decoder::new().map_err(backend)?;
        Ok(Box::new(OpusDecoder { dec, pcm: vec![0; crate::MAX_FRAME_SAMPLES] }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> Vec<f32> {
        (0..FRAME_SAMPLES).map(|i| (i as f32 * 0.05).sin() * 0.5).collect()
    }

    #[test]
    fn round_trip_and_errors() {
        let c = OpusCodec;
        let mut e = c.encoder(24_000).unwrap();
        let mut d = c.decoder().unwrap();
        let mut pkt = Vec::new();
        let mut out = vec![0.0; FRAME_SAMPLES];
        for _ in 0..5 {
            e.encode(&tone(), &mut pkt).unwrap();
            assert!(!pkt.is_empty() && pkt.len() < 200);
            d.decode(Some(&pkt), &mut out).unwrap();
        }
        assert!(out.iter().any(|x| x.abs() > 0.05), "decoded audio");
        d.decode(None, &mut out).unwrap();
        e.set_bitrate(16_000);
        e.set_bitrate(u32::MAX);
        assert_eq!(e.encode(&[0.0; 10], &mut pkt), Err(CodecError::FrameSize(10)));
        assert_eq!(d.decode(Some(&pkt), &mut [0.0; 10]), Err(CodecError::FrameSize(10)));
        assert_eq!(d.decode(Some(&[0xff, 0xff, 0xff]), &mut out), Err(CodecError::Corrupt));
        // a 40 ms packet does not fit a 20 ms frame
        let mut raw = Encoder::new(Application::Voip).unwrap();
        let mut long = vec![0u8; MAX_PACKET];
        let n = long_packet(&mut raw, &mut long);
        assert!(matches!(d.decode(Some(&long[..n]), &mut out), Err(CodecError::FrameSize(1920))));
        assert_eq!(to_i16(2.0), 32767);
        assert_eq!(to_i16(-2.0), -32767);
        assert!(backend(Error(-1)).to_string().starts_with("codec:"));
    }

    /// Two 20 ms frames repacketized into one 40 ms packet, by hand: Opus code 1 (two equal frames).
    fn long_packet(enc: &mut Encoder, out: &mut [u8]) -> usize {
        let mut a = vec![0u8; MAX_PACKET];
        let pcm = vec![0i16; FRAME_SAMPLES];
        let n = enc.encode(&pcm, &mut a).unwrap();
        // TOC with code 1: same config, two frames of equal size.
        out[0] = (a[0] & !0x03) | 0x01;
        out[1..n].copy_from_slice(&a[1..n]);
        out[n..2 * n - 1].copy_from_slice(&a[1..n]);
        2 * n - 1
    }
}
