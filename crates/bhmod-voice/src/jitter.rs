//! Per-speaker jitter buffer: reorders frames by `seq`, plays one per 20 ms tick and reports gaps so the decoder can
//! conceal them (Opus packet loss concealment: decode with no packet).

use std::collections::HashMap;

/// Default frames buffered before playout starts: 4 × 20 ms = 80 ms (the server's 0.2 s world step adds jitter).
pub const DEFAULT_TARGET: u16 = 4;
/// Default most frames held; older ones are skipped when it overflows.
pub const DEFAULT_MAX: u16 = 25;
/// Default lost frames concealed in a row before the stream counts as ended (rebuffers on the next frame).
pub const DEFAULT_MAX_CONCEAL: u16 = 5;

/// What to play this tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playout<T> {
    /// The frame.
    Frame(T),
    /// A frame is missing: conceal it (call the decoder with no packet).
    Lost,
    /// Nothing to play (not started, or the talk spurt ended).
    Idle,
}

/// A jitter buffer for one speaker.
#[derive(Debug, Clone)]
pub struct JitterBuffer<T> {
    target: u16,
    max: u16,
    max_conceal: u16,
    frames: HashMap<u16, T>,
    next: Option<u16>,
    missed: u16,
}

impl<T> Default for JitterBuffer<T> {
    fn default() -> Self {
        JitterBuffer::new(DEFAULT_TARGET, DEFAULT_MAX, DEFAULT_MAX_CONCEAL)
    }
}

fn ahead(seq: u16, of: u16) -> i16 {
    seq.wrapping_sub(of) as i16
}

impl<T> JitterBuffer<T> {
    /// A buffer that starts after `target` frames, holds at most `max`, and conceals at most `max_conceal` in a row.
    pub fn new(target: u16, max: u16, max_conceal: u16) -> JitterBuffer<T> {
        let target = target.max(1);
        JitterBuffer { target, max: max.max(target), max_conceal, frames: HashMap::new(), next: None, missed: 0 }
    }

    /// Adds a received frame. Returns false if it was late or a duplicate.
    pub fn push(&mut self, seq: u16, frame: T) -> bool {
        if let Some(next) = self.next {
            if ahead(seq, next) < 0 {
                return false;
            }
            if ahead(seq, next) >= self.max as i16 {
                // Far ahead: skip what we would never play in time.
                let start = seq.wrapping_sub(self.max - 1);
                self.frames.retain(|s, _| ahead(*s, start) >= 0);
                self.next = Some(start);
            }
        }
        if self.frames.contains_key(&seq) {
            return false;
        }
        self.frames.insert(seq, frame);
        if self.next.is_none() && self.frames.len() >= usize::from(self.target) {
            self.next = self.frames.keys().copied().min_by_key(|s| ahead(*s, seq));
        }
        true
    }

    /// The frame for this 20 ms tick.
    pub fn pop(&mut self) -> Playout<T> {
        let Some(next) = self.next else { return Playout::Idle };
        if let Some(f) = self.frames.remove(&next) {
            self.next = Some(next.wrapping_add(1));
            self.missed = 0;
            return Playout::Frame(f);
        }
        if self.frames.is_empty() && self.missed >= self.max_conceal {
            self.next = None;
            self.missed = 0;
            return Playout::Idle;
        }
        self.missed += 1;
        self.next = Some(next.wrapping_add(1));
        Playout::Lost
    }

    /// Frames waiting.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when no frame waits.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// True while playing out (after the target was reached).
    pub fn playing(&self) -> bool {
        self.next.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_then_plays_in_order() {
        let mut j = JitterBuffer::new(3, 10, 2);
        assert_eq!(j.pop(), Playout::Idle);
        assert!(j.push(11, b'b'));
        assert!(j.push(10, b'a'));
        assert!(!j.playing());
        assert!(!j.push(10, b'x'), "duplicate");
        assert!(j.push(12, b'c'));
        assert!(j.playing());
        assert_eq!(j.len(), 3);
        assert_eq!(j.pop(), Playout::Frame(b'a'));
        assert!(!j.push(10, b'z'), "late");
        assert_eq!(j.pop(), Playout::Frame(b'b'));
        assert!(j.push(14, b'e'));
        assert_eq!(j.pop(), Playout::Frame(b'c'));
        assert_eq!(j.pop(), Playout::Lost, "13 missing: conceal");
        assert_eq!(j.pop(), Playout::Frame(b'e'));
        assert!(j.is_empty());
        assert_eq!(j.pop(), Playout::Lost);
        assert_eq!(j.pop(), Playout::Lost);
        assert_eq!(j.pop(), Playout::Idle, "spurt ended after 2 concealed");
        assert!(!j.playing());
    }

    #[test]
    fn wraps_and_skips_ahead() {
        let mut j: JitterBuffer<u8> = JitterBuffer::new(2, 4, 1);
        assert!(j.push(65535, 1));
        assert!(j.push(0, 2));
        assert_eq!(j.pop(), Playout::Frame(1));
        assert!(j.push(10, 9), "far ahead");
        assert_eq!(j.len(), 1);
        assert_eq!(j.pop(), Playout::Lost); // 7
        for _ in 8..10 {
            assert_eq!(j.pop(), Playout::Lost);
        }
        assert_eq!(j.pop(), Playout::Frame(9));
        let d: JitterBuffer<u8> = JitterBuffer::default();
        assert_eq!((d.target, d.max), (DEFAULT_TARGET, DEFAULT_MAX));
        let z: JitterBuffer<u8> = JitterBuffer::new(0, 0, 0);
        assert_eq!((z.target, z.max), (1, 1));
    }
}
