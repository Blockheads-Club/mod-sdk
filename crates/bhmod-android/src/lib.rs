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
pub mod weapons;

#[cfg(test)]
mod testutil;

#[cfg(target_os = "android")]
pub mod android;

use bhmod_client::{Caps, Options};
use bhmod_core::runtime::Runtime;
use bhmod_manifest::manifest::Runtime as Kind;

/// Loader version (`HELLO.loader`, shown in the Mods screen).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// `HELLO.platform`.
pub const PLATFORM: &str = "android-armv7";

/// Makes the JVM runtime: on Android it adopts ART (see `android::jvm_factory`). Logs its own failures.
#[cfg(feature = "jvm")]
pub type JvmFactory = fn() -> Result<Box<dyn Runtime>, String>;

#[cfg(feature = "jvm")]
static JVM: std::sync::Mutex<Option<JvmFactory>> = std::sync::Mutex::new(None);

/// Turns JVM mods on (`Some`) or off (`None`). Until it is called with a factory this loader neither runs nor
/// advertises `jvm`: Java and Kotlin mods are unsandboxed code, so the owner has to ask for them.
#[cfg(feature = "jvm")]
pub fn set_jvm(factory: Option<JvmFactory>) {
    *JVM.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = factory;
}

#[cfg(feature = "jvm")]
fn jvm_factory() -> Option<JvmFactory> {
    *JVM.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The runtime kinds compiled in (and, for the JVM, switched on).
pub fn kinds() -> Vec<Kind> {
    #[allow(unused_mut)]
    let mut v = vec![Kind::Lua];
    if cfg!(feature = "wasm") {
        v.push(Kind::Wasm);
    }
    #[cfg(feature = "jvm")]
    if jvm_factory().is_some() {
        v.push(Kind::Jvm);
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
    let names: Vec<&str> = kinds()
        .iter()
        .map(|k| match k {
            Kind::Lua => "lua",
            Kind::Wasm => "wasm",
            Kind::Jvm => "jvm",
        })
        .collect();
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
    #[cfg(feature = "jvm")]
    if let Some(make) = jvm_factory() {
        // The factory logs why it failed; the mods that need the JVM then report "runtime not available".
        v.extend(make());
    }
    v
}

#[cfg(test)]
mod tests {
    /// The JVM switch is process-wide: tests that read or change it take turns.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn identity() {
        let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let o = super::options();
        assert_eq!(o.platform, "android-armv7");
        assert!(o.runtimes.contains(&"lua".to_string()));
        assert_eq!(super::caps().runtimes.len(), o.runtimes.len());
        assert!(super::budgets().hard > bhmod_core::budget::Budgets::default().hard);
        let t = tempfile::tempdir().unwrap();
        assert_eq!(super::runtimes(t.path()).len(), o.runtimes.len());
    }

    /// The JVM is off until the glue switches it on, then it is advertised and created like the others.
    #[cfg(feature = "jvm")]
    #[test]
    fn jvm_is_opt_in() {
        let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        fn desktop_jvm() -> Result<Box<dyn bhmod_core::runtime::Runtime>, String> {
            bhmod_core::jvm::JvmRuntime::new(Default::default()).map(|r| Box::new(r) as _)
        }
        fn broken() -> Result<Box<dyn bhmod_core::runtime::Runtime>, String> {
            Err("no JVM".into())
        }
        let t = tempfile::tempdir().unwrap();
        super::set_jvm(None);
        assert!(!super::options().runtimes.contains(&"jvm".to_string()));
        let without = super::runtimes(t.path()).len();
        super::set_jvm(Some(desktop_jvm));
        assert!(super::options().runtimes.contains(&"jvm".to_string()));
        assert!(super::kinds().contains(&super::Kind::Jvm));
        assert_eq!(super::caps().runtimes.len(), super::options().runtimes.len());
        assert_eq!(super::runtimes(t.path()).len(), without + 1);
        super::set_jvm(Some(broken));
        assert_eq!(super::runtimes(t.path()).len(), without);
        super::set_jvm(None);
    }
}
