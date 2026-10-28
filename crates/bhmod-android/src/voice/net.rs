//! Unsequenced (unreliable, unordered) 0xBC sends for voice frames.
//!
//! The game only sends reliable packets (`-[BHNetClientMatch sendData:toPlayers:withDataMode:error:]` ignores the
//! mode). Voice frames instead go straight to the game's ENet 1.3.7: `enet_packet_create(data, len,
//! ENET_PACKET_FLAG_UNSEQUENCED)` + `enet_peer_send(enetPeer, 0, packet)` (see `android::enet`). ENet is not
//! thread-safe, so that call happens only on the loader thread (the one that runs `drawFrame`, which also services
//! the game's ENet host) and only while a world is open. Frames produced anywhere else (the encoder worker) wait in
//! the bounded [`Outbox`] and leave at the next frame.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// `HELLO.features` entry of a client that can send unsequenced 0xBC packets.
pub const FEATURE: &str = "voice-unsequenced";

/// Largest packet sent unsequenced. ENet 1.3.7 fragments anything longer than `mtu - 28` (no checksum) and an
/// unsequenced packet without `UNRELIABLE_FRAGMENT` is then sent as *reliable* fragments; the server's MTU is 1392.
pub const MAX_PACKET: usize = 1364;

/// Frames kept for the next `drawFrame` (16 x 20 ms = 320 ms; older ones are useless for voice).
pub const QUEUE_FRAMES: usize = 16;

/// The game's ENet peer, reachable right now (loader thread, in a world, connected).
pub trait Enet {
    /// Sends one packet unsequenced on channel 0. False when ENet refused it (peer not connected...).
    fn send_unsequenced(&self, packet: &[u8]) -> bool;
}

/// What happened to a packet handed to [`Outbox::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// Given to ENet now.
    Now,
    /// Queued for the next frame.
    Queued,
    /// ENet refused it.
    Refused,
}

/// A packet that can never be sent unsequenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// Nothing to send.
    Empty,
    /// Longer than [`MAX_PACKET`] (the length).
    TooBig(usize),
}

/// Unsequenced packets waiting for the loader thread.
#[derive(Debug, Default)]
pub struct Outbox {
    queue: Mutex<VecDeque<Vec<u8>>>,
    dropped: AtomicU64,
}

impl Outbox {
    /// An empty outbox.
    pub const fn new() -> Outbox {
        Outbox { queue: Mutex::new(VecDeque::new()), dropped: AtomicU64::new(0) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Vec<u8>>> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sends `packet` (a whole 0xBC packet, `bc 16 ...`) now when `link` is given (loader thread, in a world; queued
    /// packets go first so the order holds), otherwise queues it. A full queue drops its oldest packet.
    pub fn send(&self, packet: &[u8], link: Option<&dyn Enet>) -> Result<Sent, SendError> {
        if packet.is_empty() {
            return Err(SendError::Empty);
        }
        if packet.len() > MAX_PACKET {
            return Err(SendError::TooBig(packet.len()));
        }
        let Some(link) = link else {
            let mut q = self.lock();
            if q.len() == QUEUE_FRAMES {
                q.pop_front();
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            q.push_back(packet.to_vec());
            return Ok(Sent::Queued);
        };
        self.flush(Some(link));
        Ok(if self.send_one(link, packet) { Sent::Now } else { Sent::Refused })
    }

    fn send_one(&self, link: &dyn Enet, packet: &[u8]) -> bool {
        let ok = link.send_unsequenced(packet);
        if !ok {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        ok
    }

    /// At `drawFrame`: sends the queued packets through `link`, or drops them without one (no world, no server:
    /// stale voice must not leave later). Returns how many ENet took.
    pub fn flush(&self, link: Option<&dyn Enet>) -> usize {
        let packets = std::mem::take(&mut *self.lock());
        match link {
            Some(link) => packets.iter().filter(|p| self.send_one(link, p)).count(),
            None => {
                self.dropped.fetch_add(packets.len() as u64, Ordering::Relaxed);
                0
            }
        }
    }

    /// Packets waiting.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when nothing waits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Packets lost so far (queue overflow, refused by ENet, dropped outside a world).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeEnet {
        sent: RefCell<Vec<Vec<u8>>>,
        refuse: bool,
    }

    impl Enet for FakeEnet {
        fn send_unsequenced(&self, packet: &[u8]) -> bool {
            if !self.refuse {
                self.sent.borrow_mut().push(packet.to_vec());
            }
            !self.refuse
        }
    }

    #[test]
    fn rejects_empty_and_oversized_packets() {
        let o = Outbox::new();
        assert_eq!(o.send(&[], None), Err(SendError::Empty));
        assert_eq!(o.send(&[0; MAX_PACKET + 1], None), Err(SendError::TooBig(MAX_PACKET + 1)));
        assert_eq!(o.send(&[0; MAX_PACKET], None), Ok(Sent::Queued));
        assert!(!o.is_empty());
        assert_eq!(FEATURE, "voice-unsequenced");
    }

    #[test]
    fn direct_sends_keep_the_order_of_queued_packets() {
        let o = Outbox::default();
        let e = FakeEnet::default();
        o.send(&[0xBC, 0x16, 1], None).unwrap();
        assert_eq!(o.send(&[0xBC, 0x16, 2], Some(&e)), Ok(Sent::Now));
        assert_eq!(*e.sent.borrow(), vec![vec![0xBC, 0x16, 1], vec![0xBC, 0x16, 2]]);
        assert!(o.is_empty());
        assert_eq!(o.dropped(), 0);
    }

    #[test]
    fn a_full_queue_drops_the_oldest() {
        let o = Outbox::new();
        for i in 0..=QUEUE_FRAMES as u8 {
            o.send(&[i], None).unwrap();
        }
        assert_eq!(o.len(), QUEUE_FRAMES);
        assert_eq!(o.dropped(), 1);
        let e = FakeEnet::default();
        assert_eq!(o.flush(Some(&e)), QUEUE_FRAMES);
        assert_eq!(e.sent.borrow()[0], vec![1]);
    }

    #[test]
    fn flush_without_a_link_drops_and_refusals_count() {
        let o = Outbox::new();
        o.send(&[1], None).unwrap();
        o.send(&[2], None).unwrap();
        assert_eq!(o.flush(None), 0);
        assert_eq!(o.dropped(), 2);
        let e = FakeEnet { refuse: true, ..Default::default() };
        o.send(&[3], None).unwrap();
        assert_eq!(o.send(&[4], Some(&e)), Ok(Sent::Refused));
        assert_eq!(o.dropped(), 4);
    }

    #[test]
    fn a_poisoned_queue_still_works() {
        let o = std::sync::Arc::new(Outbox::new());
        let other = std::sync::Arc::clone(&o);
        let _ = std::thread::spawn(move || {
            let _g = other.queue.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert_eq!(o.send(&[1], None), Ok(Sent::Queued));
        assert_eq!(o.len(), 1);
    }
}
