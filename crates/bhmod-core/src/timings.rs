//! Per-mod time accounting (`/mods timings`): for every kind of guest call the number of calls, the total, average
//! and longest time over a rolling window, and how often the soft budget was passed and the call failed.
//!
//! Times are kept in one-second buckets, so the memory per mod is bounded (window length x handler kinds) however
//! busy the mod is. A call's time includes the time of the mods it called (services, custom events).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::runtime::CallError;

/// What a guest call was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerKind {
    /// An event handler (game events, custom events and the answers of remote calls).
    Event,
    /// A mod command.
    Command,
    /// A scheduled task.
    Task,
    /// A service call into the mod (from another mod or from the other side).
    Service,
    /// A channel message.
    Channel,
    /// `on-enable` and `on-disable`.
    Lifecycle,
}

impl HandlerKind {
    /// Every kind, in display order.
    pub const ALL: [HandlerKind; 6] = [
        HandlerKind::Event,
        HandlerKind::Command,
        HandlerKind::Task,
        HandlerKind::Service,
        HandlerKind::Channel,
        HandlerKind::Lifecycle,
    ];

    /// The name shown to admins.
    pub fn name(self) -> &'static str {
        match self {
            HandlerKind::Event => "event",
            HandlerKind::Command => "command",
            HandlerKind::Task => "task",
            HandlerKind::Service => "service",
            HandlerKind::Channel => "channel",
            HandlerKind::Lifecycle => "lifecycle",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The numbers of one handler kind of one mod over the window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindStats {
    /// Which kind.
    pub kind: Option<HandlerKind>,
    /// Calls in the window.
    pub calls: u64,
    /// Sum of their times.
    pub total: Duration,
    /// The longest one.
    pub max: Duration,
    /// Calls that took longer than the soft budget.
    pub soft_hits: u64,
    /// Calls that were stopped at the hard deadline.
    pub timeouts: u64,
    /// Calls that trapped, panicked or returned an error.
    pub errors: u64,
}

impl KindStats {
    /// The average time of a call (zero without calls).
    pub fn avg(&self) -> Duration {
        match u32::try_from(self.calls) {
            Ok(n) if n > 0 => self.total / n,
            _ => Duration::ZERO,
        }
    }

    /// Soft budget hits plus timeouts.
    pub fn budget_hits(&self) -> u64 {
        self.soft_hits + self.timeouts
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    sec: u64,
    stats: KindStats,
}

/// The accounting of one mod.
#[derive(Debug, Clone)]
pub struct Timings {
    epoch: Instant,
    buckets: [VecDeque<Bucket>; 6],
    strikes_total: u64,
}

impl Timings {
    /// A fresh accounting that starts at `now`.
    pub fn new(now: Instant) -> Timings {
        Timings { epoch: now, buckets: Default::default(), strikes_total: 0 }
    }

    fn second(&self, t: Instant) -> u64 {
        t.saturating_duration_since(self.epoch).as_secs()
    }

    /// Records one call that started at `start` and took `elapsed`.
    pub fn record(
        &mut self,
        kind: HandlerKind,
        start: Instant,
        elapsed: Duration,
        soft: Duration,
        outcome: Option<&CallError>,
    ) {
        let sec = self.second(start);
        let q = &mut self.buckets[kind.index()];
        if q.back().is_none_or(|b| b.sec != sec) {
            q.push_back(Bucket { sec, stats: KindStats::default() });
        }
        let s = &mut q.back_mut().expect("a bucket was just pushed").stats;
        s.calls += 1;
        s.total += elapsed;
        s.max = s.max.max(elapsed);
        s.soft_hits += u64::from(elapsed > soft);
        s.timeouts += u64::from(matches!(outcome, Some(CallError::Timeout)));
        s.errors += u64::from(matches!(outcome, Some(CallError::Trap(_) | CallError::Failed(_))));
    }

    /// A strike was recorded.
    pub fn strike(&mut self) {
        self.strikes_total += 1;
    }

    /// Strikes since the accounting started (not limited to the window).
    pub fn strikes_total(&self) -> u64 {
        self.strikes_total
    }

    /// The per-kind numbers over the `window` ending at `now`, for the kinds that were called at all.
    pub fn snapshot(&self, now: Instant, window: Duration) -> Vec<KindStats> {
        let cur = self.second(now);
        let secs = window.as_secs().max(1);
        let mut out = Vec::new();
        for kind in HandlerKind::ALL {
            let mut acc = KindStats { kind: Some(kind), ..KindStats::default() };
            for b in self.buckets[kind.index()].iter().filter(|b| b.sec + secs > cur) {
                acc.calls += b.stats.calls;
                acc.total += b.stats.total;
                acc.max = acc.max.max(b.stats.max);
                acc.soft_hits += b.stats.soft_hits;
                acc.timeouts += b.stats.timeouts;
                acc.errors += b.stats.errors;
            }
            if acc.calls > 0 {
                out.push(acc);
            }
        }
        out
    }

    /// Drops the buckets that left the window (called as calls are recorded, so memory stays bounded).
    pub fn prune(&mut self, now: Instant, window: Duration) {
        let cur = self.second(now);
        let secs = window.as_secs().max(1);
        for q in &mut self.buckets {
            while q.front().is_some_and(|b| b.sec + secs <= cur) {
                q.pop_front();
            }
        }
    }
}

/// The timings of one mod as shown to admins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModTimings {
    /// Mod id.
    pub id: String,
    /// The mod's state, as text (`enabled`, `disabled: ...`).
    pub state: String,
    /// The window the numbers cover.
    pub window: Duration,
    /// Numbers per handler kind (only kinds that were called).
    pub kinds: Vec<KindStats>,
    /// Strikes inside the strike window right now.
    pub strikes: usize,
    /// Strikes since the last reset.
    pub strikes_total: u64,
}

impl ModTimings {
    /// Total calls over all kinds.
    pub fn calls(&self) -> u64 {
        self.kinds.iter().map(|k| k.calls).sum()
    }

    /// Total time over all kinds.
    pub fn total(&self) -> Duration {
        self.kinds.iter().map(|k| k.total).sum()
    }

    /// Soft budget hits and timeouts over all kinds.
    pub fn budget_hits(&self) -> u64 {
        self.kinds.iter().map(KindStats::budget_hits).sum()
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2}", d.as_secs_f64() * 1000.0)
}

/// The text lines of `/mods timings` (one block per mod).
pub fn format_lines(all: &[ModTimings]) -> Vec<String> {
    if all.is_empty() {
        return vec!["No mods are loaded.".to_string()];
    }
    let window = all[0].window.as_secs();
    let mut out = vec![format!("Mod timings over the last {window} s (times in ms, inclusive of nested calls):")];
    for m in all {
        out.push(format!(
            "{} [{}]: {} calls, {} ms total, {} budget hits, strikes {} now / {} since reset",
            m.id,
            m.state,
            m.calls(),
            ms(m.total()),
            m.budget_hits(),
            m.strikes,
            m.strikes_total
        ));
        for k in &m.kinds {
            let name = k.kind.map_or("", HandlerKind::name);
            out.push(format!(
                "  {name}: {} calls, total {}, avg {}, max {}, over soft {}, timeouts {}, errors {}",
                k.calls,
                ms(k.total),
                ms(k.avg()),
                ms(k.max),
                k.soft_hits,
                k.timeouts,
                k.errors
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn records_and_rolls_the_window() {
        let t0 = Instant::now();
        let soft = Duration::from_millis(5);
        let mut t = Timings::new(t0);
        t.record(HandlerKind::Event, t0, Duration::from_millis(2), soft, None);
        t.record(HandlerKind::Event, t0, Duration::from_millis(8), soft, None);
        t.record(HandlerKind::Event, t0 + secs(1), Duration::from_millis(30), soft, Some(&CallError::Timeout));
        t.record(HandlerKind::Task, t0 + secs(1), Duration::from_millis(1), soft, Some(&CallError::Trap("x".into())));
        t.record(HandlerKind::Task, t0 + secs(1), Duration::from_millis(1), soft, Some(&CallError::Failed("x".into())));
        t.strike();
        let snap = t.snapshot(t0 + secs(2), secs(60));
        assert_eq!(snap.len(), 2);
        let e = snap[0];
        assert_eq!((e.kind, e.calls, e.soft_hits, e.timeouts, e.errors), (Some(HandlerKind::Event), 3, 2, 1, 0));
        assert_eq!(e.total, Duration::from_millis(40));
        assert_eq!(e.max, Duration::from_millis(30));
        assert_eq!(e.avg(), Duration::from_nanos(13_333_333));
        assert_eq!(e.budget_hits(), 3);
        assert_eq!((snap[1].kind, snap[1].errors, snap[1].budget_hits()), (Some(HandlerKind::Task), 2, 0));
        // The first second leaves a 2 s window at t0 + 2.
        let late = t.snapshot(t0 + secs(2), secs(2));
        assert_eq!(late[0].calls, 1);
        // A zero window still shows the current second.
        assert_eq!(t.snapshot(t0 + secs(1), Duration::ZERO).len(), 2);
        t.prune(t0 + secs(100), secs(60));
        assert!(t.snapshot(t0 + secs(100), secs(60)).is_empty());
        assert!(t.buckets.iter().all(VecDeque::is_empty));
        assert_eq!(t.strikes_total(), 1);
        assert_eq!(KindStats::default().avg(), Duration::ZERO);
        let names = HandlerKind::ALL.map(HandlerKind::name);
        assert_eq!(names, ["event", "command", "task", "service", "channel", "lifecycle"]);
        let huge = KindStats { calls: u64::MAX, total: secs(1), ..KindStats::default() };
        assert_eq!(huge.avg(), Duration::ZERO);
    }

    #[test]
    fn report_lines() {
        assert_eq!(format_lines(&[]), vec!["No mods are loaded.".to_string()]);
        let m = ModTimings {
            id: "eco".into(),
            state: "enabled".into(),
            window: secs(60),
            kinds: vec![KindStats {
                kind: Some(HandlerKind::Command),
                calls: 2,
                total: Duration::from_millis(3),
                max: Duration::from_millis(2),
                soft_hits: 1,
                timeouts: 0,
                errors: 1,
            }],
            strikes: 1,
            strikes_total: 4,
        };
        assert_eq!((m.calls(), m.total(), m.budget_hits()), (2, Duration::from_millis(3), 1));
        let lines = format_lines(&[m]);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("last 60 s"));
        assert_eq!(lines[1], "eco [enabled]: 2 calls, 3.00 ms total, 1 budget hits, strikes 1 now / 4 since reset");
        assert_eq!(lines[2], "  command: 2 calls, total 3.00, avg 1.50, max 2.00, over soft 1, timeouts 0, errors 1");
        let empty = ModTimings {
            id: "x".into(),
            state: "enabled".into(),
            window: secs(60),
            kinds: vec![KindStats::default()],
            strikes: 0,
            strikes_total: 0,
        };
        assert!(format_lines(&[empty])[2].starts_with("  : 0 calls"));
    }
}
