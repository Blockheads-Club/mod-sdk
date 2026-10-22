//! Event subscriptions and their run order.
//!
//! Handlers run from `lowest` to `highest`, then `monitor`; inside one priority in mod load order (dependencies
//! first), then in subscription order. Owners are mod slot indices, which follow the load order. Custom events are
//! subscribed by name (`<mod-id>:<name>`) and follow the same rules.

use crate::runtime::SubscriptionId;
use crate::types::{EventKind, Priority};

/// One subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// The id given to the mod.
    pub id: SubscriptionId,
    /// The owning mod (slot index = load order).
    pub owner: usize,
    /// Event kind ([`EventKind::Custom`] for custom events).
    pub kind: EventKind,
    /// The custom event name (only for [`EventKind::Custom`]).
    pub custom: Option<String>,
    /// Priority.
    pub priority: Priority,
    /// Skip while the event is cancelled.
    pub ignore_cancelled: bool,
}

/// All subscriptions.
#[derive(Debug, Default)]
pub struct EventBus {
    subs: Vec<Subscription>,
    next_id: SubscriptionId,
}

/// Most subscriptions per mod.
pub const MAX_PER_MOD: usize = 1024;

impl EventBus {
    /// Adds a subscription. `None` when the mod already has [`MAX_PER_MOD`].
    pub fn subscribe(
        &mut self,
        owner: usize,
        kind: EventKind,
        priority: Priority,
        ignore_cancelled: bool,
    ) -> Option<SubscriptionId> {
        self.add(owner, kind, None, priority, ignore_cancelled)
    }

    /// Adds a subscription to the custom event `name`. `None` when the mod already has [`MAX_PER_MOD`].
    pub fn subscribe_custom(
        &mut self,
        owner: usize,
        name: &str,
        priority: Priority,
        ignore_cancelled: bool,
    ) -> Option<SubscriptionId> {
        self.add(owner, EventKind::Custom, Some(name.to_string()), priority, ignore_cancelled)
    }

    fn add(
        &mut self,
        owner: usize,
        kind: EventKind,
        custom: Option<String>,
        priority: Priority,
        ignore_cancelled: bool,
    ) -> Option<SubscriptionId> {
        if self.subs.iter().filter(|s| s.owner == owner).count() >= MAX_PER_MOD {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.subs.push(Subscription { id, owner, kind, custom, priority, ignore_cancelled });
        Some(id)
    }

    /// Removes one of `owner`'s subscriptions.
    pub fn unsubscribe(&mut self, owner: usize, id: SubscriptionId) -> bool {
        let before = self.subs.len();
        self.subs.retain(|s| !(s.owner == owner && s.id == id));
        self.subs.len() != before
    }

    /// Removes everything `owner` subscribed.
    pub fn remove_owner(&mut self, owner: usize) {
        self.subs.retain(|s| s.owner != owner);
    }

    /// True when anyone listens to `kind`.
    pub fn has_listeners(&self, kind: EventKind) -> bool {
        self.subs.iter().any(|s| s.kind == kind)
    }

    /// The handlers for `kind` (and, for [`EventKind::Custom`], the event `custom`) in run order (a snapshot:
    /// changes made while dispatching apply to the next event).
    pub fn plan(&self, kind: EventKind, custom: Option<&str>) -> Vec<Subscription> {
        let mut v: Vec<Subscription> =
            self.subs.iter().filter(|s| s.kind == kind && s.custom.as_deref() == custom).cloned().collect();
        v.sort_by_key(|s| (s.priority, s.owner, s.id));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_and_removal() {
        let mut b = EventBus::default();
        let a = b.subscribe(1, EventKind::Chat, Priority::Monitor, false).unwrap();
        let c = b.subscribe(1, EventKind::Chat, Priority::Lowest, true).unwrap();
        let d = b.subscribe(0, EventKind::Chat, Priority::Normal, false).unwrap();
        let e = b.subscribe(0, EventKind::Chat, Priority::Lowest, false).unwrap();
        let t = b.subscribe(0, EventKind::Tick, Priority::Normal, false).unwrap();
        let order: Vec<_> = b.plan(EventKind::Chat, None).iter().map(|s| s.id).collect();
        assert_eq!(order, vec![e, c, d, a]);
        assert!(b.has_listeners(EventKind::Tick));
        assert!(!b.has_listeners(EventKind::PlayerJoin));
        assert!(!b.unsubscribe(1, t), "only the owner may unsubscribe");
        assert!(b.unsubscribe(0, t));
        assert!(!b.has_listeners(EventKind::Tick));
        b.remove_owner(1);
        let order: Vec<_> = b.plan(EventKind::Chat, None).iter().map(|s| s.id).collect();
        assert_eq!(order, vec![e, d]);
    }

    #[test]
    fn custom_events_by_name() {
        let mut b = EventBus::default();
        let x = b.subscribe_custom(1, "m:x", Priority::High, false).unwrap();
        let y = b.subscribe_custom(0, "m:x", Priority::Low, false).unwrap();
        b.subscribe_custom(0, "m:y", Priority::Low, false).unwrap();
        let order: Vec<_> = b.plan(EventKind::Custom, Some("m:x")).iter().map(|s| s.id).collect();
        assert_eq!(order, vec![y, x]);
        assert!(b.plan(EventKind::Custom, Some("m:z")).is_empty());
        assert!(b.plan(EventKind::Custom, None).is_empty());
        assert!(b.has_listeners(EventKind::Custom));
    }

    #[test]
    fn per_mod_limit() {
        let mut b = EventBus::default();
        for _ in 0..MAX_PER_MOD {
            b.subscribe(0, EventKind::Tick, Priority::Normal, false).unwrap();
        }
        assert!(b.subscribe(0, EventKind::Tick, Priority::Normal, false).is_none());
        assert!(b.subscribe(1, EventKind::Tick, Priority::Normal, false).is_some());
    }
}
