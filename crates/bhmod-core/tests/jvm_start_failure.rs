//! A Java runtime that cannot be started is reported once and stays reported.
#![cfg(feature = "jvm")]

use bhmod_core::jvm::{JvmConfig, JvmRuntime};

#[test]
fn a_broken_libjvm_is_an_error_not_a_crash() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("lib/server");
    std::fs::create_dir_all(&dir).unwrap();
    let lib = if cfg!(windows) {
        "jvm.dll"
    } else if cfg!(target_os = "macos") {
        "libjvm.dylib"
    } else {
        "libjvm.so"
    };
    std::fs::write(dir.join(lib), b"this is not a shared library").unwrap();
    let cfg = JvmConfig {
        java_home: Some(home.path().to_path_buf()),
        work_dir: Some(home.path().join("work")),
        ..JvmConfig::default()
    };
    let first = JvmRuntime::new(cfg.clone()).err().expect("must fail");
    assert!(first.contains("cannot start the JVM from"), "{first}");
    // The failure is cached for the process; a later runtime with another configuration sees the same.
    let second = JvmRuntime::new(JvmConfig::default()).err().expect("must fail");
    assert_eq!(first, second);
}
