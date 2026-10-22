//! Network callbacks from the game's other threads.
//!
//! The loader (and every mod VM in it) lives on the game thread that runs `drawFrame`. The game does not always
//! receive packets there: on a real phone, packets that arrive while a world loads come in on another thread. Those
//! callbacks queue a [`NetEvent`] here, and the loader thread takes them at its next frame, in arrival order.

use std::sync::{Mutex, OnceLock};
use std::thread::{self, ThreadId};

/// A network callback for the loader thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetEvent {
    /// The game reached a server (`host:port`): the 0xBC detection starts.
    Contact(String),
    /// A 0xBC packet from the server.
    Packet(Vec<u8>),
    /// The connection closed.
    Disconnect,
}

/// The loader thread and the events queued for it by other threads.
#[derive(Debug, Default)]
pub struct Inbox {
    owner: OnceLock<ThreadId>,
    queue: Mutex<Vec<NetEvent>>,
}

impl Inbox {
    /// An empty inbox without an owner.
    pub const fn new() -> Inbox {
        Inbox { owner: OnceLock::new(), queue: Mutex::new(Vec::new()) }
    }

    /// Makes the calling thread the loader thread unless one is set already.
    pub fn claim(&self) {
        self.owner.get_or_init(|| thread::current().id());
    }

    /// True on the loader thread (false before [`Inbox::claim`]).
    pub fn is_owner(&self) -> bool {
        self.owner.get() == Some(&thread::current().id())
    }

    /// Queues `ev` for the loader thread.
    pub fn push(&self, ev: NetEvent) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
    }

    /// Takes the queued events, oldest first.
    pub fn drain(&self) -> Vec<NetEvent> {
        std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn owner_is_the_first_claiming_thread() {
        let inbox = Arc::new(Inbox::new());
        assert!(!inbox.is_owner());
        inbox.claim();
        assert!(inbox.is_owner());
        let other = Arc::clone(&inbox);
        let owner = thread::spawn(move || {
            other.claim();
            other.is_owner()
        })
        .join()
        .unwrap();
        assert!(!owner);
        assert!(inbox.is_owner());
    }

    #[test]
    fn events_from_other_threads_drain_in_order() {
        let inbox = Arc::new(Inbox::default());
        let other = Arc::clone(&inbox);
        thread::spawn(move || {
            other.push(NetEvent::Contact("h:1".into()));
            other.push(NetEvent::Packet(vec![0xBC, 0x10]));
            other.push(NetEvent::Disconnect);
        })
        .join()
        .unwrap();
        assert_eq!(
            inbox.drain(),
            vec![NetEvent::Contact("h:1".into()), NetEvent::Packet(vec![0xBC, 0x10]), NetEvent::Disconnect]
        );
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn a_poisoned_queue_still_works() {
        let inbox = Arc::new(Inbox::new());
        let other = Arc::clone(&inbox);
        let _ = thread::spawn(move || {
            let _g = other.queue.lock().unwrap();
            panic!("poison");
        })
        .join();
        inbox.push(NetEvent::Disconnect);
        assert_eq!(inbox.drain(), vec![NetEvent::Disconnect]);
    }
}
