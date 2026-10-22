//! Delayed and repeating tasks on the host tick, plus next-poll tasks.

use std::collections::BTreeMap;

use crate::runtime::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Task {
    owner: usize,
    /// `None` = next poll.
    due: Option<u64>,
    period: Option<u32>,
}

/// Most pending tasks per mod.
pub const MAX_PER_MOD: usize = 10_000;

/// All tasks. Ticks count from 0; [`Scheduler::advance`] moves to the next tick.
#[derive(Debug, Default)]
pub struct Scheduler {
    now: u64,
    tasks: BTreeMap<TaskId, Task>,
    next_id: TaskId,
}

impl Scheduler {
    /// The current tick.
    pub fn now(&self) -> u64 {
        self.now
    }

    fn add(&mut self, owner: usize, due: Option<u64>, period: Option<u32>) -> Option<TaskId> {
        if self.tasks.values().filter(|t| t.owner == owner).count() >= MAX_PER_MOD {
            return None;
        }
        self.next_id += 1;
        self.tasks.insert(self.next_id, Task { owner, due, period });
        Some(self.next_id)
    }

    /// Runs once after `delay` ticks (0 = the next tick). `None` when over the per-mod limit.
    pub fn run_later(&mut self, owner: usize, delay: u32) -> Option<TaskId> {
        self.add(owner, Some(self.now + u64::from(delay.max(1))), None)
    }

    /// Runs after `delay` ticks, then every `period` ticks (`period` >= 1, checked by the caller).
    pub fn run_every(&mut self, owner: usize, delay: u32, period: u32) -> Option<TaskId> {
        self.add(owner, Some(self.now + u64::from(delay.max(1))), Some(period))
    }

    /// Runs once at the next poll.
    pub fn run_next_poll(&mut self, owner: usize) -> Option<TaskId> {
        self.add(owner, None, None)
    }

    /// Cancels one of `owner`'s tasks.
    pub fn cancel(&mut self, owner: usize, id: TaskId) -> bool {
        if self.tasks.get(&id).is_some_and(|t| t.owner == owner) {
            self.tasks.remove(&id);
            true
        } else {
            false
        }
    }

    /// Drops every task of `owner`.
    pub fn remove_owner(&mut self, owner: usize) {
        self.tasks.retain(|_, t| t.owner != owner);
    }

    /// Moves to the next tick and returns `(owner, id)` of the tasks due, in (due tick, id) order. One-shot tasks
    /// are removed, repeating ones rescheduled.
    pub fn advance(&mut self) -> Vec<(usize, TaskId)> {
        self.now += 1;
        let now = self.now;
        let mut due: Vec<(u64, TaskId, usize)> =
            self.tasks.iter().filter_map(|(id, t)| t.due.filter(|d| *d <= now).map(|d| (d, *id, t.owner))).collect();
        due.sort();
        for (_, id, _) in &due {
            let t = self.tasks.get_mut(id).expect("present");
            match t.period {
                Some(p) => t.due = Some(now + u64::from(p)),
                None => {
                    self.tasks.remove(id);
                }
            }
        }
        due.into_iter().map(|(_, id, owner)| (owner, id)).collect()
    }

    /// Puts a task taken by [`Scheduler::advance`] back for the next tick (used when the step budget ran out).
    pub fn postpone(&mut self, owner: usize, id: TaskId) {
        let next = self.now + 1;
        self.tasks.entry(id).and_modify(|t| t.due = Some(next)).or_insert(Task {
            owner,
            due: Some(next),
            period: None,
        });
    }

    /// Takes every next-poll task, in id order.
    pub fn take_poll(&mut self) -> Vec<(usize, TaskId)> {
        let ids: Vec<(usize, TaskId)> =
            self.tasks.iter().filter(|(_, t)| t.due.is_none()).map(|(id, t)| (t.owner, *id)).collect();
        for (_, id) in &ids {
            self.tasks.remove(id);
        }
        ids
    }

    /// True when next-poll tasks are waiting.
    pub fn has_poll(&self) -> bool {
        self.tasks.values().any(|t| t.due.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_every_cancel() {
        let mut s = Scheduler::default();
        let a = s.run_later(0, 0).unwrap();
        let b = s.run_later(1, 2).unwrap();
        let c = s.run_every(0, 1, 2).unwrap();
        let d = s.run_later(0, 5).unwrap();
        assert!(!s.cancel(1, d));
        assert!(s.cancel(0, d));
        assert!(!s.cancel(0, d));
        assert_eq!(s.advance(), vec![(0, a), (0, c)]);
        assert_eq!(s.now(), 1);
        assert_eq!(s.advance(), vec![(1, b)]);
        assert_eq!(s.advance(), vec![(0, c)]);
        assert_eq!(s.advance(), vec![]);
        s.remove_owner(0);
        assert_eq!(s.advance(), vec![]);
    }

    #[test]
    fn poll_and_postpone() {
        let mut s = Scheduler::default();
        assert!(!s.has_poll());
        let p = s.run_next_poll(3).unwrap();
        let q = s.run_next_poll(4).unwrap();
        assert!(s.has_poll());
        assert_eq!(s.advance(), vec![]);
        assert_eq!(s.take_poll(), vec![(3, p), (4, q)]);
        assert!(!s.has_poll());
        let t = s.run_later(0, 1).unwrap();
        let r = s.run_every(0, 1, 5).unwrap();
        assert_eq!(s.advance(), vec![(0, t), (0, r)]);
        s.postpone(0, t);
        s.postpone(0, r);
        assert_eq!(s.advance(), vec![(0, t), (0, r)]);
        assert_eq!(s.advance(), vec![]);
    }

    #[test]
    fn per_mod_limit() {
        let mut s = Scheduler::default();
        for _ in 0..MAX_PER_MOD {
            s.run_next_poll(0).unwrap();
        }
        assert!(s.run_later(0, 1).is_none());
        assert!(s.run_later(1, 1).is_some());
    }
}
