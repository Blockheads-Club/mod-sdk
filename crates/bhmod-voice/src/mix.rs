//! Client mixing: gain, constant-power stereo pan, muffle low-pass and a soft limiter.
//!
//! Heard level = server gain (the frame's `gain` byte: speaker gain × route gain) × client master volume × client
//! per-player volume, then the soft limiter so boosted voices do not clip.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_4, PI};

use crate::PeerId;

/// Most client volume, percent.
pub const MAX_VOLUME: u16 = 200;
/// Muffle cutoff at `muffle = 1` (Hz).
pub const MUFFLE_MIN_HZ: f32 = 20_000.0;
/// Muffle cutoff at `muffle = 255` (Hz).
pub const MUFFLE_MAX_HZ: f32 = 400.0;
/// Level above which the soft limiter bends the curve.
pub const LIMIT_KNEE: f32 = 0.8;

/// Left / right gains of a pan position -1..=1 (constant power: `l² + r² = 1`).
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (a.cos(), a.sin())
}

/// The frame's `pan` byte as -1..=1.
pub fn pan_from_byte(b: i8) -> f32 {
    (f32::from(b) / 127.0).max(-1.0)
}

/// The frame's `gain` byte (percent) as a factor 0..=2.
pub fn gain_from_byte(b: u8) -> f32 {
    f32::from(b.min(200)) / 100.0
}

/// Soft limiter: linear below [`LIMIT_KNEE`], then a tanh curve that approaches ±1 without reaching it.
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= LIMIT_KNEE {
        return x;
    }
    let room = 1.0 - LIMIT_KNEE;
    (LIMIT_KNEE + room * ((a - LIMIT_KNEE) / room).tanh()).copysign(x)
}

/// One-pole low-pass coefficient for a muffle byte (`0`: bypass). The cutoff falls exponentially from
/// [`MUFFLE_MIN_HZ`] to [`MUFFLE_MAX_HZ`].
pub fn muffle_alpha(muffle: u8, sample_rate: u32) -> f32 {
    if muffle == 0 {
        return 1.0;
    }
    let t = f32::from(muffle) / 255.0;
    let fc = MUFFLE_MIN_HZ * (MUFFLE_MAX_HZ / MUFFLE_MIN_HZ).powf(t);
    1.0 - (-2.0 * PI * fc / sample_rate as f32).exp()
}

/// A one-pole low-pass filter (`y += a · (x - y)`), state kept per speaker.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Muffle {
    y: f32,
}

impl Muffle {
    /// Filters `pcm` in place with coefficient `alpha` (1 = bypass, still tracks the state).
    pub fn process(&mut self, pcm: &mut [f32], alpha: f32) {
        for s in pcm {
            self.y += alpha * (*s - self.y);
            *s = self.y;
        }
    }
}

/// The player's local volume settings (never sent to the server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volumes {
    master: u16,
    players: HashMap<PeerId, u16>,
    muted: Vec<PeerId>,
}

impl Default for Volumes {
    fn default() -> Volumes {
        Volumes { master: 100, players: HashMap::new(), muted: Vec::new() }
    }
}

impl Volumes {
    /// Voice chat master volume, percent 0..=200 (clamped).
    pub fn set_master(&mut self, percent: u16) {
        self.master = percent.min(MAX_VOLUME);
    }

    /// The master volume, percent.
    pub fn master(&self) -> u16 {
        self.master
    }

    /// Per-player volume, percent 0..=200 (clamped; 100 is the default).
    pub fn set_player(&mut self, speaker: PeerId, percent: u16) {
        self.players.insert(speaker, percent.min(MAX_VOLUME));
    }

    /// A player's volume, percent.
    pub fn player(&self, speaker: PeerId) -> u16 {
        self.players.get(&speaker).copied().unwrap_or(100)
    }

    /// Mutes or unmutes one player locally.
    pub fn set_muted(&mut self, speaker: PeerId, muted: bool) {
        self.muted.retain(|p| *p != speaker);
        if muted {
            self.muted.push(speaker);
        }
    }

    /// True if muted locally.
    pub fn is_muted(&self, speaker: PeerId) -> bool {
        self.muted.contains(&speaker)
    }

    /// The client part of the gain for a speaker: master × player (0 when muted).
    pub fn factor(&self, speaker: PeerId) -> f32 {
        if self.is_muted(speaker) {
            return 0.0;
        }
        f32::from(self.master) / 100.0 * f32::from(self.player(speaker)) / 100.0
    }
}

/// One decoded speaker frame to mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Source<'a> {
    /// The speaker.
    pub speaker: PeerId,
    /// Mono samples.
    pub pcm: &'a [f32],
    /// The frame's `gain` byte.
    pub gain: u8,
    /// The frame's `pan` byte.
    pub pan: i8,
    /// The frame's `muffle` byte.
    pub muffle: u8,
}

/// Mixes speakers into a stereo buffer.
#[derive(Debug, Clone, Default)]
pub struct Mixer {
    /// Local volumes.
    pub volumes: Volumes,
    sample_rate: u32,
    filters: HashMap<PeerId, Muffle>,
    scratch: Vec<f32>,
}

impl Mixer {
    /// A mixer at `sample_rate`.
    pub fn new(sample_rate: u32) -> Mixer {
        Mixer { sample_rate, ..Mixer::default() }
    }

    /// Mixes `sources` into `out` (interleaved stereo, `2 × frames` samples, overwritten). Sources shorter than the
    /// output are padded with silence.
    pub fn mix(&mut self, out: &mut [f32], sources: &[Source<'_>]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        for s in sources {
            let g = gain_from_byte(s.gain) * self.volumes.factor(s.speaker);
            let n = s.pcm.len().min(frames);
            self.scratch.clear();
            self.scratch.extend_from_slice(&s.pcm[..n]);
            let f = self.filters.entry(s.speaker).or_default();
            f.process(&mut self.scratch, muffle_alpha(s.muffle, self.sample_rate));
            if g == 0.0 {
                continue;
            }
            let (l, r) = pan_gains(pan_from_byte(s.pan));
            for (i, x) in self.scratch.iter().enumerate() {
                out[2 * i] += x * g * l;
                out[2 * i + 1] += x * g * r;
            }
        }
        out.iter_mut().for_each(|x| *x = soft_limit(*x));
    }

    /// Forgets a speaker's filter state (left, or the talk spurt ended).
    pub fn forget(&mut self, speaker: PeerId) {
        self.filters.remove(&speaker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn pan_is_constant_power() {
        for p in [-2.0, -1.0, -0.5, 0.0, 0.3, 1.0] {
            let (l, r) = pan_gains(p);
            assert!(close(l * l + r * r, 1.0));
        }
        let (l, r) = pan_gains(0.0);
        assert!(close(l, r));
        let (l, r) = pan_gains(-1.0);
        assert!(close(l, 1.0) && close(r, 0.0));
        assert_eq!(pan_from_byte(-128), -1.0);
        assert_eq!(pan_from_byte(127), 1.0);
        assert_eq!(gain_from_byte(255), 2.0);
        assert_eq!(gain_from_byte(50), 0.5);
    }

    #[test]
    fn limiter() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-0.8), -0.8);
        assert!(soft_limit(5.0) <= 1.0 && soft_limit(5.0) > 0.99);
        assert!(soft_limit(-5.0) >= -1.0 && soft_limit(-5.0) < -0.99);
        assert!(soft_limit(0.9) > 0.8 && soft_limit(0.9) < 0.9);
    }

    #[test]
    fn muffle_lowers_high_frequencies() {
        assert_eq!(muffle_alpha(0, 48_000), 1.0);
        assert!(muffle_alpha(255, 48_000) < muffle_alpha(1, 48_000));
        // A Nyquist-rate square wave nearly vanishes when fully muffled.
        let mut pcm: Vec<f32> = (0..960).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let mut m = Muffle::default();
        m.process(&mut pcm, muffle_alpha(255, 48_000));
        assert!(pcm[900..].iter().all(|x| x.abs() < 0.05), "{}", pcm[959]);
        // DC passes.
        let mut dc = vec![0.5; 4800];
        Muffle::default().process(&mut dc, muffle_alpha(255, 48_000));
        assert!(close(dc[4799], 0.5));
    }

    #[test]
    fn volumes() {
        let mut v = Volumes::default();
        assert_eq!((v.master(), v.player(3)), (100, 100));
        v.set_master(500);
        v.set_player(3, 50);
        assert_eq!((v.master(), v.player(3)), (200, 50));
        assert!(close(v.factor(3), 1.0));
        v.set_muted(3, true);
        v.set_muted(3, true);
        assert!(v.is_muted(3));
        assert_eq!(v.factor(3), 0.0);
        v.set_muted(3, false);
        assert!(!v.is_muted(3));
        v.set_player(4, 999);
        assert_eq!(v.player(4), 200);
    }

    #[test]
    fn mixer() {
        let mut m = Mixer::new(48_000);
        let pcm = [0.25f32; 4];
        let mut out = [9.0f32; 8];
        m.volumes.set_player(2, 200);
        m.volumes.set_muted(3, true);
        m.mix(
            &mut out,
            &[
                Source { speaker: 1, pcm: &pcm, gain: 100, pan: -127, muffle: 0 },
                Source { speaker: 2, pcm: &pcm[..2], gain: 50, pan: 127, muffle: 0 },
                Source { speaker: 3, pcm: &pcm, gain: 200, pan: 0, muffle: 0 },
            ],
        );
        // Speaker 1 hard left at 0.25; speaker 2 hard right at 0.25 × 0.5 × 2 for two frames; 3 muted.
        assert!(close(out[0], 0.25) && close(out[1], 0.25));
        assert!(close(out[4], 0.25) && close(out[5], 0.0));
        let loud = [1.0f32; 2];
        m.mix(&mut out[..4], &[Source { speaker: 1, pcm: &loud, gain: 200, pan: 0, muffle: 0 }]);
        assert!(out[0] < 1.0 && out[0] > 0.9, "limited, {}", out[0]);
        m.forget(1);
        assert!(!m.filters.contains_key(&1));
    }
}
