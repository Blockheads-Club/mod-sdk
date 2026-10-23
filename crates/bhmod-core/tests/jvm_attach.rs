//! Adopting a JVM that already runs in the process, the way the Android loader adopts ART: the host owns the
//! `JavaVM` and hands over a class loader that sees the SDK classes.
#![cfg(feature = "jvm")]

mod common;

use bhmod_core::jvm::{attach_existing, JvmConfig, JvmRuntime, SDK_JAR};
use bhmod_core::types::{Event, Side};
use bhmod_core::{HostOptions, ModHost, ModState};
use common::java::*;
use common::*;
use jni::objects::JValue;
use jni::{InitArgsBuilder, JNIVersion, JavaVM};

#[test]
fn a_running_jvm_can_be_adopted() {
    let lib = JvmRuntime::find_jvm(&JvmConfig::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("sdk.jar");
    std::fs::write(&jar, SDK_JAR).unwrap();
    let args = InitArgsBuilder::new()
        .version(JNIVersion::V8)
        .option("-Xrs")
        .option(format!("-Djava.class.path={}", jar.display()))
        .build()
        .unwrap();
    let jvm = JavaVM::with_libjvm(args, || Ok::<_, jni::errors::StartJvmError>(lib)).unwrap();
    let mut env = jvm.attach_current_thread_permanently().unwrap();
    let loader = env
        .call_static_method(
            "java/lang/ClassLoader",
            "getSystemClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[] as &[JValue],
        )
        .unwrap()
        .l()
        .unwrap();
    let loader = env.new_global_ref(loader).unwrap();
    // SAFETY: the pointer comes from the live JavaVM above, which stays alive for the whole test process.
    let rt = unsafe { attach_existing(jvm.get_java_vm_pointer(), loader.clone(), &JvmConfig::default()) }.unwrap();
    // A second adoption (or `JvmRuntime::new`) shares the same VM.
    let again = unsafe { attach_existing(jvm.get_java_vm_pointer(), loader, &JvmConfig::default()) }.unwrap();
    drop(again);

    let tmp = tempfile::tempdir().unwrap();
    jvm_mod(tmp.path(), "sink", "fx.Sink", "");
    std::fs::write(tmp.path().join("mods.toml"), "[sink]\ngrant = [\"native\", \"channels\"]\n").unwrap();
    let b = FakeBridge::new();
    let mut opts = HostOptions::new(tmp.path(), Side::Server);
    opts.runtimes.push(Box::new(rt));
    let mut h = ModHost::load(opts, b.clone());
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    h.fire(Event::Chat { player: "p1".into(), text: "adopted".into(), cancelled: false });
    assert!(b.has_log("chat p1 adopted false"), "{}", b.log_text());
}
