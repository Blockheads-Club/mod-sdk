//! Transport policy: in-band (0xBC on ENet channel 0, unsequenced) or the dedicated voice UDP port.
//!
//! The same [`PathSelector`] runs on the server (per listener) and on the client (for its own sends), so both ends of
//! a peer follow one rule set:
//!
//! - `inband`: always in-band; frames over the in-band budget are dropped.
//! - `udp`: the UDP port once the peer authenticated there; in-band until then (if the frame fits).
//! - `auto`: in-band; moves to UDP (and stays) when a port is configured, the peer authenticated on it, and the
//!   peer cannot send unsequenced in-band, a frame does not fit in-band, the measured loss is over the threshold or
//!   the server's poll lag is over [`DEFAULT_LAG_MS`]. No port configured: in-band only.

use crate::frame::{Budget, Direction, Fit};

/// Loss ratio over which `auto` moves to UDP.
pub const DEFAULT_LOSS_THRESHOLD: f32 = 0.1;
/// Server poll lag (ms) over which `auto` moves to UDP.
pub const DEFAULT_LAG_MS: u32 = 30;
/// Frames per loss measurement period.
pub const LOSS_PERIOD: u32 = 50;

/// The admin's `transport` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportMode {
    /// In-band, switch per peer to UDP when needed (default).
    #[default]
    Auto,
    /// In-band only.
    InBand,
    /// Dedicated UDP port.
    Udp,
}

impl TransportMode {
    /// Parses the setting value (`auto`, `inband`, `udp`).
    pub fn parse(s: &str) -> Option<TransportMode> {
        match s {
            "auto" => Some(TransportMode::Auto),
            "inband" => Some(TransportMode::InBand),
            "udp" => Some(TransportMode::Udp),
            _ => None,
        }
    }
}

/// The path one frame takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Path {
    /// 0xBC `VOICE_FRAME` inside the game's ENet connection.
    InBand,
    /// A [`crate::udp::Datagram::Frame`] on the dedicated port.
    Udp,
}

/// Loss estimate from the sequence numbers of received frames (smoothed per [`LOSS_PERIOD`] frames).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LossMeter {
    last: Option<u16>,
    received: u32,
    lost: u32,
    loss: f32,
}

impl LossMeter {
    /// Records a received frame. Late or duplicate frames are ignored; a jump of more than 100 frames restarts
    /// the count (a new talk spurt after a reset).
    pub fn record(&mut self, seq: u16) {
        let Some(last) = self.last else {
            self.last = Some(seq);
            return;
        };
        let gap = seq.wrapping_sub(last) as i16;
        if gap <= 0 {
            return;
        }
        self.last = Some(seq);
        if gap > 100 {
            return;
        }
        self.received += 1;
        self.lost += gap as u32 - 1;
        let n = self.received + self.lost;
        if n >= LOSS_PERIOD {
            self.loss = (self.loss + self.lost as f32 / n as f32) / 2.0;
            self.received = 0;
            self.lost = 0;
        }
    }

    /// Smoothed loss ratio 0..=1.
    pub fn loss(&self) -> f32 {
        self.loss
    }
}

/// Chooses the path of one peer's frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathSelector {
    /// The admin's setting.
    pub mode: TransportMode,
    /// The server has a voice UDP port.
    pub udp_configured: bool,
    /// The peer authenticated on the UDP port.
    pub udp_ready: bool,
    /// The peer can send/receive unsequenced in-band frames.
    pub inband_capable: bool,
    /// Size limits.
    pub budget: Budget,
    /// `auto` loss threshold.
    pub loss_threshold: f32,
    /// `auto` lag threshold (ms).
    pub lag_ms: u32,
    switched: bool,
}

impl PathSelector {
    /// A selector with the default thresholds; the peer is in-band capable and not on UDP yet.
    pub fn new(mode: TransportMode, udp_configured: bool) -> PathSelector {
        PathSelector {
            mode,
            udp_configured,
            udp_ready: false,
            inband_capable: true,
            budget: Budget::default(),
            loss_threshold: DEFAULT_LOSS_THRESHOLD,
            lag_ms: DEFAULT_LAG_MS,
            switched: false,
        }
    }

    /// True once `auto` moved this peer to UDP (sticky).
    pub fn switched(&self) -> bool {
        self.switched
    }

    /// The path for a frame with `opus` bytes, given the measured `loss` and the server's poll `lag_ms`; `None`:
    /// drop the frame.
    pub fn choose(&mut self, dir: Direction, opus: usize, loss: f32, lag_ms: u32) -> Option<Path> {
        let fit = self.budget.fit(dir, opus);
        let udp = self.udp_configured && self.udp_ready && fit != Fit::TooLarge;
        let inband = self.inband_capable && fit == Fit::InBand;
        match self.mode {
            TransportMode::InBand => inband.then_some(Path::InBand),
            TransportMode::Udp if udp => Some(Path::Udp),
            TransportMode::Udp => inband.then_some(Path::InBand),
            TransportMode::Auto => {
                if udp && (self.switched || !inband || loss > self.loss_threshold || lag_ms > self.lag_ms) {
                    self.switched = true;
                    Some(Path::Udp)
                } else {
                    inband.then_some(Path::InBand)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        assert_eq!(TransportMode::parse("auto"), Some(TransportMode::Auto));
        assert_eq!(TransportMode::parse("inband"), Some(TransportMode::InBand));
        assert_eq!(TransportMode::parse("udp"), Some(TransportMode::Udp));
        assert_eq!(TransportMode::parse("tcp"), None);
        assert_eq!(TransportMode::default(), TransportMode::Auto);
    }

    #[test]
    fn loss_meter() {
        let mut m = LossMeter::default();
        m.record(65530);
        // Every 5th frame lost: after 51 expected (10 lost) the period closes at 10/51, smoothed to half.
        let mut seq = 65530u16;
        for i in 0..51 {
            seq = seq.wrapping_add(1);
            if i % 5 != 4 {
                m.record(seq);
            }
        }
        let want = 10.0 / 51.0 / 2.0;
        assert!((m.loss() - want).abs() < 1e-6, "{}", m.loss());
        m.record(seq.wrapping_sub(3)); // late
        m.record(seq); // duplicate
        m.record(seq.wrapping_add(500)); // jump: restart
        assert!((m.loss() - want).abs() < 1e-6);
    }

    #[test]
    fn inband_and_udp_modes() {
        let mut s = PathSelector::new(TransportMode::InBand, true);
        s.udp_ready = true;
        assert_eq!(s.choose(Direction::Up, 80, 0.5, 100), Some(Path::InBand));
        assert_eq!(s.choose(Direction::Up, 300, 0.0, 0), None);

        let mut s = PathSelector::new(TransportMode::Udp, true);
        assert_eq!(s.choose(Direction::Up, 80, 0.0, 0), Some(Path::InBand), "until UDP auth");
        assert_eq!(s.choose(Direction::Up, 300, 0.0, 0), None);
        s.udp_ready = true;
        assert_eq!(s.choose(Direction::Up, 300, 0.0, 0), Some(Path::Udp));
        assert_eq!(s.choose(Direction::Up, 401, 0.0, 0), None);
    }

    #[test]
    fn auto() {
        let mut s = PathSelector::new(TransportMode::Auto, false);
        s.udp_ready = true; // no port configured: never UDP
        assert_eq!(s.choose(Direction::Down, 80, 0.9, 100), Some(Path::InBand));

        let mut s = PathSelector::new(TransportMode::Auto, true);
        assert_eq!(s.choose(Direction::Down, 80, 0.9, 100), Some(Path::InBand), "not authenticated yet");
        s.udp_ready = true;
        assert_eq!(s.choose(Direction::Down, 80, 0.05, 10), Some(Path::InBand));
        assert!(!s.switched());
        assert_eq!(s.choose(Direction::Down, 80, 0.05, 31), Some(Path::Udp), "lag");
        assert!(s.switched());
        assert_eq!(s.choose(Direction::Down, 80, 0.0, 0), Some(Path::Udp), "sticky");

        for (loss, opus, capable) in [(0.2, 80, true), (0.0, 300, true), (0.0, 80, false)] {
            let mut s = PathSelector::new(TransportMode::Auto, true);
            s.udp_ready = true;
            s.inband_capable = capable;
            assert_eq!(s.choose(Direction::Up, opus, loss, 0), Some(Path::Udp));
        }
    }
}
