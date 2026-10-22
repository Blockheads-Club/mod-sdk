//! Cross-language interop: the Rust `shop-rust` WASM mod pays through the `economy` service of the Luau economy mod.
//!
//! Needs the WASM runtime (feature `wasm`, milestone M2) and the built component: run
//! `examples/shop-rust/build.sh` first (CI does). Without the feature this file compiles to nothing.
#![cfg(feature = "wasm")]

mod common;

use bhmod_core::types::{CommandSender, Role, Side};
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;

#[test]
fn rust_shop_pays_through_the_luau_economy() {
    let wasm = examples().join("shop-rust/server/mod.wasm");
    assert!(wasm.is_file(), "build {} first (examples/shop-rust/build.sh)", wasm.display());
    let tmp = tempfile::tempdir().unwrap();
    for e in ["economy", "shop-rust"] {
        copy_dir(&examples().join(e), &tmp.path().join(e));
    }
    let b = FakeBridge::new();
    b.join("p1", "ALICE", Role::Player);
    let mut h = ModHost::load(HostOptions::new(tmp.path(), Side::Server), b.clone());
    h.enable("World", "wid");
    for m in h.mods() {
        assert_eq!(m.state, ModState::Enabled, "{}", b.log_text());
    }
    let p = || CommandSender::Player("p1".into());
    for _ in 0..3 {
        assert_eq!(h.command(p(), "/buy sword"), Some(vec!["You bought a sword for 30 coins.".to_string()]));
    }
    assert_eq!(h.command(p(), "/buy sword"), Some(vec!["A sword costs 30 coins; you do not have enough.".to_string()]));
    assert_eq!(h.command(p(), "/balance"), Some(vec!["ALICE has 10 coins.".to_string()]));
    assert_eq!(h.command(p(), "/buy"), Some(vec!["Usage: /buy <apple|sword>".to_string()]));
}
