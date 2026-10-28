//! The game's own ENet 1.3.7 (exported by `libApplication.so`), used for unsequenced voice frames.
//!
//! Verified against the 1.7.6 `libApplication.so` dynamic symbols: `enet_packet_create`, `enet_peer_send`,
//! `enet_packet_destroy` and `OBJC_IVAR_$_BHNetClientMatch.enetPeer` (ivar type `^{_ENetPeer}`). Only called on the
//! loader thread (the `drawFrame` thread, which also services the game's ENet host), in a world, while connected.

use std::ffi::c_void;
use std::sync::OnceLock;

use super::objc::{self, Id};
use crate::voice::net::Enet;

/// `ENET_PACKET_FLAG_UNSEQUENCED`.
const UNSEQUENCED: u32 = 2;

struct Fns {
    create: unsafe extern "C" fn(*const c_void, usize, u32) -> *mut c_void,
    send: unsafe extern "C" fn(*mut c_void, u8, *mut c_void) -> i32,
    destroy: unsafe extern "C" fn(*mut c_void),
}

static FNS: OnceLock<Option<Fns>> = OnceLock::new();

fn fns() -> Option<&'static Fns> {
    FNS.get_or_init(|| {
        let (c, s, d) = (
            objc::app_symbol("enet_packet_create"),
            objc::app_symbol("enet_peer_send"),
            objc::app_symbol("enet_packet_destroy"),
        );
        let ivar = objc::app_symbol("OBJC_IVAR_$_BHNetClientMatch.enetPeer");
        if [c, s, d, ivar].iter().any(|p| p.is_null()) {
            return None;
        }
        // SAFETY: the game's exported ENet 1.3.7 functions with these C signatures.
        unsafe {
            Some(Fns {
                create: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*const c_void, usize, u32) -> *mut c_void,
                >(c),
                send: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void, u8, *mut c_void) -> i32>(s),
                destroy: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void)>(d),
            })
        }
    })
    .as_ref()
}

/// True when this game exports what unsequenced sends need (`HELLO.features` gets `voice-unsequenced`).
pub fn available() -> bool {
    fns().is_some()
}

/// The `ENetPeer *` of a client match.
pub struct Peer(*mut c_void);

impl Peer {
    /// `matc.enetPeer` (None when nil or the ivar is missing).
    pub fn of(matc: Id) -> Option<Peer> {
        let p = objc::ivar(matc, "BHNetClientMatch", "enetPeer");
        (!p.is_null() && available()).then_some(Peer(p))
    }
}

impl Enet for Peer {
    fn send_unsequenced(&self, packet: &[u8]) -> bool {
        let Some(f) = fns() else { return false };
        // SAFETY: loader thread only (see the module docs); the packet copies `packet`. enet_peer_send takes the
        // packet on success; on failure (-1) nothing references it, so it is destroyed here.
        unsafe {
            let pkt = (f.create)(packet.as_ptr() as *const c_void, packet.len(), UNSEQUENCED);
            if pkt.is_null() {
                return false;
            }
            if (f.send)(self.0, 0, pkt) < 0 {
                (f.destroy)(pkt);
                return false;
            }
        }
        true
    }
}
