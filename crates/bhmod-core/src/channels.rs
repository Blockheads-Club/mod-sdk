//! Named channels: pub/sub byte messages between mods (and, from milestone M5, between a server mod and its client
//! half over the network).
//!
//! A mod opens `<mod-id>:<name>` (its own id or one it depends on) and gets a channel id of its own. Local messages
//! are queued and delivered on the next poll or tick, never synchronously, so a send can never re-enter a mod.
//! Network targets go to [`crate::GameBridge::channel_send`]; inbound network messages enter through
//! [`crate::ModHost::channel_receive`].

use std::collections::VecDeque;

use crate::types::ChannelSource;

/// A channel id as given to one mod (`channels.channel-id`).
pub type ChannelId = u32;

/// Largest message in bytes.
pub const MAX_MESSAGE: usize = 32 * 1024;
/// Most channels one mod may have open.
pub const MAX_PER_MOD: usize = 64;
/// Most queued messages (all mods together).
pub const MAX_QUEUED: usize = 4096;

/// One open channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    /// The id given to the owner.
    pub id: ChannelId,
    /// The mod that opened it.
    pub owner: usize,
    /// Full name.
    pub name: String,
}

/// A queued message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Channel name.
    pub name: String,
    /// The sending mod for local messages (it does not receive its own message).
    pub from_owner: Option<usize>,
    /// Where it came from, as the receivers see it.
    pub source: ChannelSource,
    /// Payload.
    pub data: Vec<u8>,
}

/// Open channels and the local delivery queue.
#[derive(Debug, Default)]
pub struct Channels {
    open: Vec<Open>,
    queue: VecDeque<Message>,
    next_id: ChannelId,
}

impl Channels {
    /// Opens `name` for `owner` (the same id again if it is already open). `None` when the mod has too many.
    pub fn open(&mut self, owner: usize, name: &str) -> Option<ChannelId> {
        if let Some(o) = self.open.iter().find(|o| o.owner == owner && o.name == name) {
            return Some(o.id);
        }
        if self.open.iter().filter(|o| o.owner == owner).count() >= MAX_PER_MOD {
            return None;
        }
        self.next_id += 1;
        self.open.push(Open { id: self.next_id, owner, name: name.to_string() });
        Some(self.next_id)
    }

    /// The name of `owner`'s channel `id`.
    pub fn name_of(&self, owner: usize, id: ChannelId) -> Option<&str> {
        self.open.iter().find(|o| o.owner == owner && o.id == id).map(|o| o.name.as_str())
    }

    /// Closes one of `owner`'s channels.
    pub fn close(&mut self, owner: usize, id: ChannelId) -> bool {
        let before = self.open.len();
        self.open.retain(|o| !(o.owner == owner && o.id == id));
        before != self.open.len()
    }

    /// Closes everything `owner` opened.
    pub fn remove_owner(&mut self, owner: usize) {
        self.open.retain(|o| o.owner != owner);
    }

    /// Queues a message. False when the queue is full.
    pub fn enqueue(&mut self, msg: Message) -> bool {
        if self.queue.len() >= MAX_QUEUED {
            return false;
        }
        self.queue.push_back(msg);
        true
    }

    /// True when messages are waiting.
    pub fn has_queued(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Takes every queued message with its receivers `(owner, channel id)`, in order.
    pub fn drain(&mut self) -> Vec<(Message, Vec<(usize, ChannelId)>)> {
        let msgs: Vec<Message> = self.queue.drain(..).collect();
        msgs.into_iter()
            .map(|m| {
                let to = self
                    .open
                    .iter()
                    .filter(|o| o.name == m.name && Some(o.owner) != m.from_owner)
                    .map(|o| (o.owner, o.id))
                    .collect();
                (m, to)
            })
            .collect()
    }
}

/// True for a valid channel name `<mod-id>:<name>` (`name`: `[a-z0-9_.-]{1,64}`); returns the mod id part.
pub fn split_name(name: &str) -> Option<&str> {
    let (m, rest) = name.split_once(':')?;
    let ok = bhmod_manifest::rules::is_mod_id(m)
        && (1..=64).contains(&rest.len())
        && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
    ok.then_some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(name: &str, from: Option<usize>) -> Message {
        Message { name: name.into(), from_owner: from, source: ChannelSource::Server, data: vec![1] }
    }

    #[test]
    fn open_close_and_deliver() {
        let mut c = Channels::default();
        let a = c.open(0, "m:x").unwrap();
        assert_eq!(c.open(0, "m:x"), Some(a));
        let b = c.open(1, "m:x").unwrap();
        c.open(2, "m:y").unwrap();
        assert_eq!(c.name_of(1, b), Some("m:x"));
        assert_eq!(c.name_of(0, b), None);
        assert!(!c.has_queued());
        assert!(c.enqueue(msg("m:x", Some(0))));
        assert!(c.enqueue(msg("m:x", None)));
        assert!(c.has_queued());
        let d = c.drain();
        assert_eq!(d[0].1, vec![(1, b)]);
        assert_eq!(d[1].1, vec![(0, a), (1, b)]);
        assert!(!c.has_queued());
        assert!(c.close(1, b));
        assert!(!c.close(1, b));
        c.remove_owner(0);
        assert!(c.name_of(0, a).is_none());
    }

    #[test]
    fn limits() {
        let mut c = Channels::default();
        for i in 0..MAX_PER_MOD {
            c.open(0, &format!("m:c{i}")).unwrap();
        }
        assert!(c.open(0, "m:one-more").is_none());
        for _ in 0..MAX_QUEUED {
            assert!(c.enqueue(msg("m:x", None)));
        }
        assert!(!c.enqueue(msg("m:x", None)));
    }

    #[test]
    fn names() {
        assert_eq!(split_name("eco:balance-changed"), Some("eco"));
        assert_eq!(split_name("eco:a.b_c-1"), Some("eco"));
        for bad in ["eco", "eco:", "Eco:x", "eco:X", "e:x", "eco:a b", &format!("eco:{}", "a".repeat(65))] {
            assert_eq!(split_name(bad), None, "{bad}");
        }
    }
}
