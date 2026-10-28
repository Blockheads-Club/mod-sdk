//! Voice activity detection: frame energy plus zero-crossing rate, with attack and hangover.
//!
//! A frame is speech-like when its RMS level is over the threshold and its zero-crossing rate is below `zcr_max`
//! (hiss and fan noise cross zero far more often than voiced speech). Activity starts after `attack` speech-like
//! frames in a row and holds for `hangover` frames after the last one, so word gaps do not cut the voice.

/// VAD settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadConfig {
    /// RMS level (dBFS) a frame must exceed.
    pub threshold_db: f32,
    /// Most zero crossings per sample of a speech-like frame.
    pub zcr_max: f32,
    /// Speech-like frames in a row before activity starts.
    pub attack: u32,
    /// Frames activity holds after the last speech-like frame (15 × 20 ms = 300 ms).
    pub hangover: u32,
}

impl Default for VadConfig {
    fn default() -> VadConfig {
        VadConfig { threshold_db: -45.0, zcr_max: 0.35, attack: 2, hangover: 15 }
    }
}

/// RMS level in dBFS (silence: -inf clamps to -120) and zero-crossing rate per sample of a frame.
pub fn frame_stats(pcm: &[f32]) -> (f32, f32) {
    if pcm.is_empty() {
        return (-120.0, 0.0);
    }
    let energy = pcm.iter().map(|x| x * x).sum::<f32>() / pcm.len() as f32;
    let db = (10.0 * energy.log10()).max(-120.0);
    let crossings = pcm.windows(2).filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0)).count();
    (db, crossings as f32 / pcm.len() as f32)
}

/// A voice activity detector.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vad {
    /// Settings (the threshold is a player setting).
    pub config: VadConfig,
    run: u32,
    hold: u32,
    active: bool,
}

impl Vad {
    /// A detector with these settings.
    pub fn new(config: VadConfig) -> Vad {
        Vad { config, ..Vad::default() }
    }

    /// Feeds one frame; returns whether voice is active.
    pub fn process(&mut self, pcm: &[f32]) -> bool {
        let (db, zcr) = frame_stats(pcm);
        if db > self.config.threshold_db && zcr <= self.config.zcr_max {
            self.run += 1;
            if self.run >= self.config.attack {
                self.active = true;
                self.hold = self.config.hangover;
            }
        } else {
            self.run = 0;
            if self.hold > 0 {
                self.hold -= 1;
            } else {
                self.active = false;
            }
        }
        self.active
    }

    /// The current state.
    pub fn active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amp: f32, hz: f32) -> Vec<f32> {
        (0..960).map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / 48_000.0).sin()).collect()
    }

    #[test]
    fn stats() {
        assert_eq!(frame_stats(&[]), (-120.0, 0.0));
        assert_eq!(frame_stats(&[0.0; 10]).0, -120.0);
        let (db, zcr) = frame_stats(&tone(1.0, 200.0));
        assert!((db + 3.01).abs() < 0.1, "{db}");
        assert!((zcr - 400.0 / 48_000.0).abs() < 0.002, "{zcr}");
    }

    #[test]
    fn attack_and_hangover() {
        let mut v = Vad::new(VadConfig { hangover: 2, ..VadConfig::default() });
        let voice = tone(0.3, 200.0);
        let quiet = tone(0.001, 200.0);
        let hiss: Vec<f32> = (0..960).map(|i| if i % 2 == 0 { 0.3 } else { -0.3 }).collect();
        assert!(!v.process(&voice), "attack needs two frames");
        assert!(v.process(&voice));
        assert!(v.process(&quiet));
        assert!(v.process(&hiss), "hiss is not speech, but the hangover holds");
        assert!(!v.process(&quiet));
        assert!(!v.active());
        assert!(!v.process(&hiss) && !v.process(&hiss));
    }
}
