//! Time budgets and strikes.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Budget settings. Defaults follow the design: 5 ms soft / 25 ms hard per handler, 1 s hard for `on-enable` and a
/// mod's first call, 20 ms of mod time per world step, 3 strikes within 60 s disable a mod, 32 MiB of guest memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budgets {
    /// A call longer than this is logged as a warning.
    pub soft: Duration,
    /// A call is stopped at this point (a strike).
    pub hard: Duration,
    /// The hard limit of `on-enable` and of the first call into a freshly loaded mod: a cold start (lazy
    /// compilation, first allocations, an interpreter on a slow 32-bit phone) can take far longer than an event.
    /// Never below `hard`.
    pub cold: Duration,
    /// Mod time per world step before scheduled tasks are postponed to the next step.
    pub step: Duration,
    /// Strikes that disable a mod...
    pub strikes: usize,
    /// ...within this window.
    pub window: Duration,
    /// Guest memory limit per mod.
    pub memory_bytes: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            soft: Duration::from_millis(5),
            hard: Duration::from_millis(25),
            cold: Duration::from_secs(1),
            step: Duration::from_millis(20),
            strikes: 3,
            window: Duration::from_secs(60),
            memory_bytes: 32 * 1024 * 1024,
        }
    }
}

impl Budgets {
    /// The hard limit of a call: [`Budgets::cold`] for `on-enable` and a mod's first call, else [`Budgets::hard`].
    pub fn hard_for(&self, cold: bool) -> Duration {
        if cold {
            self.cold.max(self.hard)
        } else {
            self.hard
        }
    }
}

/// A monotonic clock (replaceable in tests).
pub trait Clock {
    /// Now.
    fn now(&self) -> Instant;
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Recent strikes of one mod.
#[derive(Debug, Default, Clone)]
pub struct Strikes {
    times: VecDeque<Instant>,
}

impl Strikes {
    /// Records a strike at `now`; true when the mod must be disabled.
    pub fn record(&mut self, now: Instant, b: &Budgets) -> bool {
        self.times.push_back(now);
        while self.times.front().is_some_and(|t| now.duration_since(*t) > b.window) {
            self.times.pop_front();
        }
        self.times.len() >= b.strikes
    }

    /// Strikes inside the window as of the last record.
    pub fn count(&self) -> usize {
        self.times.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_calls_get_the_wider_limit() {
        let b = Budgets::default();
        assert_eq!(b.hard_for(false), b.hard);
        assert_eq!(b.hard_for(true), Duration::from_secs(1));
        let tight = Budgets { cold: Duration::from_millis(1), ..b };
        assert_eq!(tight.hard_for(true), b.hard, "never below the event limit");
    }

    #[test]
    fn strikes_window() {
        let b = Budgets::default();
        let t0 = SystemClock.now();
        let mut s = Strikes::default();
        assert!(!s.record(t0, &b));
        assert!(!s.record(t0 + Duration::from_secs(30), &b));
        // The first strike falls out of the window.
        assert!(!s.record(t0 + Duration::from_secs(61), &b));
        assert_eq!(s.count(), 2);
        assert!(s.record(t0 + Duration::from_secs(62), &b));
    }
}
