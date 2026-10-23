//! JVM start-up options: heap limit, extra options, the work directory and an explicit Java home. They apply to the
//! first runtime of a process, so this file is its own test binary and every test uses the same configuration.
#![cfg(feature = "jvm")]

mod common;
#[path = "common/java.rs"]
mod java;

use bhmod_core::jvm::{JvmConfig, JvmRuntime};
use bhmod_core::types::Side;
use bhmod_core::{HostOptions, ModHost, ModState};
use common::*;
use java::*;

#[test]
fn options_reach_the_jvm_and_the_bridge_reports_bad_calls() {
    let work = tempfile::tempdir().unwrap();
    let home = JvmRuntime::find_jvm(&JvmConfig::default()).unwrap();
    // <home>/lib/server/libjvm.so
    let java_home = home.ancestors().nth(3).unwrap().to_path_buf();
    let cfg = JvmConfig {
        java_home: Some(java_home),
        max_heap_mb: 96,
        extra_args: vec!["-Dbhmod.test=yes".to_string()],
        work_dir: Some(work.path().to_path_buf()),
        ..JvmConfig::default()
    };
    let tmp = tempfile::tempdir().unwrap();
    jvm_mod(tmp.path(), "props", "fx.Props", "");
    std::fs::write(tmp.path().join("mods.toml"), "[props]\ngrant = [\"native\", \"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.runtimes.push(Box::new(JvmRuntime::new(cfg).unwrap()));
    let mut h = ModHost::load(opts, b.clone());
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    let log = b.log_text();
    assert!(log.contains("prop yes"), "{log}");
    assert!(log.contains("heap 96"), "{log}");
    assert!(log.contains(&work.path().display().to_string()), "{log}");
    assert!(log.contains("raw props"), "{log}");
    assert!(log.contains("bridge unknown host function \"nope.nothing\""), "{log}");
    assert!(log.contains("bridge argument 0 must be text"), "{log}");
    // The SDK jar was written into the work directory.
    assert!(std::fs::read_dir(work.path())
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("bhmod-sdk-")));
}
