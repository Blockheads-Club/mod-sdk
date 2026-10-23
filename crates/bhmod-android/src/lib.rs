//! The Blockheads Club mod loader for the Android game (`libbhmodloader.so`).
//!
//! Blockheads Patcher adds this library to the user's own game APK (manifest meta-data `android.app.libs`, after
//! `libApplication`) together with the Java UI (`com.blockheadsclub.modloader`, a lifecycle listener). The game
//! loads it on its main thread; `JNI_OnLoad` swizzles a few Objective-C methods ([`android`]) and from then on the
//! [`loader::Loader`] runs on the game thread: the 0xBC client (`bhmod-client`), the mod host (`bhmod-core`: Luau,
//! WASM through `bhmod-wasm`) and the player's installed mods. Everything but the `android` module is plain Rust
//! and tested on the desktop with a fake [`loader::Platform`].
#![warn(missing_docs)]

pub mod api;
pub mod fonts;
pub mod inbox;
pub mod loader;
pub mod menus;
pub mod ui;

#[cfg(test)]
mod testutil;

#[cfg(target_os = "android")]
pub mod android;

use bhmod_client::{Caps, Options};
use bhmod_core::runtime::Runtime;
use bhmod_manifest::manifest::Runtime as Kind;

/// Loader version (`HELLO.loader`, the title mark).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// `HELLO.platform`.
pub const PLATFORM: &str = "android-armv7";

/// The runtime kinds compiled in.
pub fn kinds() -> Vec<Kind> {
    let mut v = vec![Kind::Lua];
    if cfg!(feature = "wasm") {
        v.push(Kind::Wasm);
    }
    v
}

/// Mod budgets on 32-bit phones: interpreted Luau and Pulley run several times slower than on a server, and the first
/// call of a mod also compiles it, so the hard limit is wider than the server's 25 ms. Measured on a Galaxy Z Fold4
/// (a 20k-step workload took Luau 14.7 ms median / 20.5 ms worst, Pulley 2 ms; `on-enable` 10 to 20 ms for Luau),
/// so 100 ms leaves room for a slower phone without letting a mod stall a frame for a quarter of a second.
pub fn budgets() -> bhmod_core::budget::Budgets {
    use std::time::Duration;
    bhmod_core::budget::Budgets {
        soft: Duration::from_millis(10),
        hard: Duration::from_millis(100),
        step: Duration::from_millis(40),
        ..Default::default()
    }
}

/// What this build can run.
pub fn caps() -> Caps {
    Caps { loader: semver::Version::parse(VERSION).expect("crate version"), runtimes: kinds() }
}

/// The 0xBC client options of this build.
pub fn options() -> Options {
    let names: Vec<&str> = kinds().iter().map(|k| if *k == Kind::Lua { "lua" } else { "wasm" }).collect();
    Options::new(VERSION, PLATFORM, bhmod_core::API_VERSION, &names)
}

/// The runtimes of a new host. WASM components are compiled once per package and cached in `wasm_cache`.
pub fn runtimes(wasm_cache: &std::path::Path) -> Vec<Box<dyn Runtime>> {
    #[allow(unused_mut)]
    let mut v = bhmod_core::default_runtimes();
    #[cfg(feature = "wasm")]
    {
        let cfg = bhmod_wasm::WasmConfig {
            backend: bhmod_wasm::Backend::Auto,
            cache_dir: Some(wasm_cache.to_path_buf()),
            trust_precompiled: false,
        };
        if let Ok(rt) = bhmod_wasm::WasmRuntime::new(cfg) {
            v.push(Box::new(rt));
        }
    }
    #[cfg(not(feature = "wasm"))]
    let _ = wasm_cache;
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn identity() {
        let o = super::options();
        assert_eq!(o.platform, "android-armv7");
        assert!(o.runtimes.contains(&"lua".to_string()));
        assert_eq!(super::caps().runtimes.len(), o.runtimes.len());
        assert!(super::budgets().hard > bhmod_core::budget::Budgets::default().hard);
        let t = tempfile::tempdir().unwrap();
        assert_eq!(super::runtimes(t.path()).len(), o.runtimes.len());
    }
}
