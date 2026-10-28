// SPDX-License-Identifier: GPL-3.0-or-later
//! Vendored libopus 1.5.2 (built by `build.rs`: fixed point, NEON on ARM) plus a small safe wrapper for voice chat:
//! 48 kHz, mono, 20 ms frames ([`FRAME_SAMPLES`] samples of `i16`).
//!
//! [`ffi`] holds the raw declarations for anyone who needs more than [`Encoder`] / [`Decoder`] offer.

pub mod codec;
pub mod ffi;

pub use codec::OpusCodec;

use std::ffi::CStr;
use std::fmt;
use std::os::raw::c_int;
use std::ptr::{self, NonNull};

/// Sample rate of [`Encoder`] and [`Decoder`].
pub const SAMPLE_RATE: i32 = 48_000;
/// Samples in one 20 ms mono frame at 48 kHz.
pub const FRAME_SAMPLES: usize = 960;
/// Largest packet libopus produces for one frame; an `out` buffer this big never fails with "buffer too small".
pub const MAX_PACKET: usize = 1275;
/// Longest frame an Opus packet can carry (120 ms at 48 kHz), the most [`Decoder::decode`] uses of `out`.
pub const MAX_FRAME_SAMPLES: usize = 5760;

/// A libopus error code (`OPUS_BAD_ARG`, `OPUS_INVALID_PACKET`, ...; see [`ffi`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(i32);

impl Error {
    /// The raw negative libopus code.
    pub fn code(self) -> i32 {
        self.0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: opus_strerror returns a static NUL-terminated string for every input.
        let msg = unsafe { CStr::from_ptr(ffi::opus_strerror(self.0)) };
        write!(f, "opus: {} ({})", msg.to_string_lossy(), self.0)
    }
}

impl std::error::Error for Error {}

fn check(ret: c_int) -> Result<c_int, Error> {
    if ret < 0 {
        Err(Error(ret))
    } else {
        Ok(ret)
    }
}

/// libopus version string, e.g. `"libopus 1.5.2-fixed"`.
pub fn version() -> &'static str {
    // SAFETY: a static NUL-terminated ASCII string.
    unsafe { CStr::from_ptr(ffi::opus_get_version_string()) }.to_str().unwrap_or("libopus")
}

/// Encoder tuning preset (`OPUS_APPLICATION_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Application {
    /// Speech intelligibility first (SILK at low bitrates); what voice chat wants.
    Voip,
    /// Faithful general audio.
    Audio,
    /// CELT only, lowest latency (no SILK, no in-band FEC).
    LowDelay,
}

impl Application {
    fn raw(self) -> c_int {
        match self {
            Application::Voip => ffi::OPUS_APPLICATION_VOIP,
            Application::Audio => ffi::OPUS_APPLICATION_AUDIO,
            Application::LowDelay => ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY,
        }
    }
}

/// A 48 kHz mono Opus encoder taking 20 ms frames.
#[derive(Debug)]
pub struct Encoder {
    st: NonNull<ffi::OpusEncoder>,
}

// SAFETY: the encoder state is plain heap memory owned by this value; libopus has no thread affinity.
unsafe impl Send for Encoder {}

impl Encoder {
    /// A new encoder at 48 kHz, mono.
    pub fn new(app: Application) -> Result<Encoder, Error> {
        Encoder::create(SAMPLE_RATE, 1, app.raw())
    }

    fn create(fs: i32, channels: c_int, app: c_int) -> Result<Encoder, Error> {
        let mut err = ffi::OPUS_OK;
        // SAFETY: `err` is a valid out pointer; a non-null result is owned by the returned Encoder.
        let st = unsafe { ffi::opus_encoder_create(fs, channels, app, &mut err) };
        check(err)?;
        Ok(Encoder { st: NonNull::new(st).ok_or(Error(ffi::OPUS_ALLOC_FAIL))? })
    }

    /// Encodes one frame of exactly [`FRAME_SAMPLES`] samples into `out` and returns the packet length. A length of
    /// 1 or 2 bytes during DTX means "nothing to send".
    pub fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, Error> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(Error(ffi::OPUS_BAD_ARG));
        }
        let max = out.len().min(i32::MAX as usize) as i32;
        // SAFETY: `pcm` holds FRAME_SAMPLES mono samples and `out` at least `max` bytes.
        let n =
            unsafe { ffi::opus_encode(self.st.as_ptr(), pcm.as_ptr(), FRAME_SAMPLES as c_int, out.as_mut_ptr(), max) };
        check(n).map(|n| n as usize)
    }

    fn set(&mut self, request: c_int, value: i32) -> Result<(), Error> {
        // SAFETY: `request` is a SET request that takes one opus_int32.
        check(unsafe { ffi::opus_encoder_ctl(self.st.as_ptr(), request, value) }).map(drop)
    }

    fn get(&self, request: c_int) -> i32 {
        let mut v: i32 = 0;
        // SAFETY: `request` is a GET request that writes one opus_int32; GETs on a valid encoder cannot fail.
        unsafe { ffi::opus_encoder_ctl(self.st.as_ptr(), request, &mut v as *mut i32) };
        v
    }

    /// Target bitrate in bits per second (500..=512000), or [`ffi::OPUS_AUTO`] / [`ffi::OPUS_BITRATE_MAX`].
    pub fn set_bitrate(&mut self, bps: i32) -> Result<(), Error> {
        self.set(ffi::OPUS_SET_BITRATE_REQUEST, bps)
    }

    /// The bitrate in use (the resolved value when set to auto).
    pub fn bitrate(&self) -> i32 {
        self.get(ffi::OPUS_GET_BITRATE_REQUEST)
    }

    /// `false` gives constant bitrate: every packet is `bitrate / 400` bytes.
    pub fn set_vbr(&mut self, on: bool) -> Result<(), Error> {
        self.set(ffi::OPUS_SET_VBR_REQUEST, on as i32)
    }

    /// Whether variable bitrate is on (the default).
    pub fn vbr(&self) -> bool {
        self.get(ffi::OPUS_GET_VBR_REQUEST) != 0
    }

    /// CPU / quality trade-off, 0..=10.
    pub fn set_complexity(&mut self, complexity: u8) -> Result<(), Error> {
        self.set(ffi::OPUS_SET_COMPLEXITY_REQUEST, complexity.into())
    }

    /// The complexity setting.
    pub fn complexity(&self) -> u8 {
        self.get(ffi::OPUS_GET_COMPLEXITY_REQUEST) as u8
    }

    /// In-band forward error correction: each packet also carries a low-rate copy of the previous frame, which
    /// [`Decoder::decode_fec`] can recover. Only used in SILK/hybrid modes and with a packet loss percentage > 0.
    pub fn set_inband_fec(&mut self, on: bool) -> Result<(), Error> {
        self.set(ffi::OPUS_SET_INBAND_FEC_REQUEST, on as i32)
    }

    /// Whether in-band FEC is on.
    pub fn inband_fec(&self) -> bool {
        self.get(ffi::OPUS_GET_INBAND_FEC_REQUEST) != 0
    }

    /// Expected packet loss, 0..=100 %; tunes FEC strength.
    pub fn set_packet_loss_perc(&mut self, percent: u8) -> Result<(), Error> {
        self.set(ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, percent.into())
    }

    /// The expected packet loss setting.
    pub fn packet_loss_perc(&self) -> u8 {
        self.get(ffi::OPUS_GET_PACKET_LOSS_PERC_REQUEST) as u8
    }

    /// Discontinuous transmission: silence encodes to 1–2 byte packets that need not be sent.
    pub fn set_dtx(&mut self, on: bool) -> Result<(), Error> {
        self.set(ffi::OPUS_SET_DTX_REQUEST, on as i32)
    }

    /// Whether DTX is on.
    pub fn dtx(&self) -> bool {
        self.get(ffi::OPUS_GET_DTX_REQUEST) != 0
    }

    /// Whether the last encoded frame was a DTX (silence) frame.
    pub fn in_dtx(&self) -> bool {
        self.get(ffi::OPUS_GET_IN_DTX_REQUEST) != 0
    }

    /// Algorithmic delay in samples at 48 kHz: decoded audio lags the input by this much.
    pub fn lookahead(&self) -> usize {
        self.get(ffi::OPUS_GET_LOOKAHEAD_REQUEST) as usize
    }

    /// Back to the freshly created state, keeping the settings (for a new talk spurt / stream).
    pub fn reset(&mut self) {
        // SAFETY: OPUS_RESET_STATE takes no argument and cannot fail.
        unsafe { ffi::opus_encoder_ctl(self.st.as_ptr(), ffi::OPUS_RESET_STATE) };
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: created by opus_encoder_create and destroyed once.
        unsafe { ffi::opus_encoder_destroy(self.st.as_ptr()) }
    }
}

/// A 48 kHz mono Opus decoder.
#[derive(Debug)]
pub struct Decoder {
    st: NonNull<ffi::OpusDecoder>,
}

// SAFETY: as for Encoder.
unsafe impl Send for Decoder {}

impl Decoder {
    /// A new decoder at 48 kHz, mono.
    pub fn new() -> Result<Decoder, Error> {
        Decoder::create(SAMPLE_RATE, 1)
    }

    fn create(fs: i32, channels: c_int) -> Result<Decoder, Error> {
        let mut err = ffi::OPUS_OK;
        // SAFETY: as in Encoder::create.
        let st = unsafe { ffi::opus_decoder_create(fs, channels, &mut err) };
        check(err)?;
        Ok(Decoder { st: NonNull::new(st).ok_or(Error(ffi::OPUS_ALLOC_FAIL))? })
    }

    fn run(&mut self, packet: Option<&[u8]>, out: &mut [i16], frame: usize, fec: bool) -> Result<usize, Error> {
        if out.len() < frame {
            return Err(Error(ffi::OPUS_BUFFER_TOO_SMALL));
        }
        let (data, len) = match packet {
            Some(p) => (p.as_ptr(), p.len().min(i32::MAX as usize) as i32),
            None => (ptr::null(), 0),
        };
        // SAFETY: `data` is null (loss) or `len` readable bytes; `out` holds at least `frame` samples.
        let n =
            unsafe { ffi::opus_decode(self.st.as_ptr(), data, len, out.as_mut_ptr(), frame as c_int, fec as c_int) };
        check(n).map(|n| n as usize)
    }

    /// Decodes one packet into `out` and returns the samples written (960 for our 20 ms frames). `out` needs room for
    /// the packet's duration; up to [`MAX_FRAME_SAMPLES`] of it are used.
    pub fn decode(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize, Error> {
        let frame = out.len().min(MAX_FRAME_SAMPLES);
        self.run(Some(packet), out, frame, false)
    }

    /// Packet loss concealment: synthesises the lost 20 ms frame into `out[..FRAME_SAMPLES]`.
    pub fn decode_lost(&mut self, out: &mut [i16]) -> Result<usize, Error> {
        self.run(None, out, FRAME_SAMPLES, false)
    }

    /// Recovers the frame *before* `packet` (which was lost) from `packet`'s in-band FEC data into
    /// `out[..FRAME_SAMPLES]`; falls back to concealment when `packet` has no FEC. Decode `packet` itself afterwards.
    pub fn decode_fec(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize, Error> {
        self.run(Some(packet), out, FRAME_SAMPLES, true)
    }

    /// Back to the freshly created state (for a new stream).
    pub fn reset(&mut self) {
        // SAFETY: OPUS_RESET_STATE takes no argument and cannot fail.
        unsafe { ffi::opus_decoder_ctl(self.st.as_ptr(), ffi::OPUS_RESET_STATE) };
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: created by opus_decoder_create and destroyed once.
        unsafe { ffi::opus_decoder_destroy(self.st.as_ptr()) }
    }
}

#[cfg(test)]
mod tests;
