//! Adopting a JVM whose class loader cannot see the SDK is an error, not a crash.
#![cfg(feature = "jvm")]

use bhmod_core::jvm::{attach_existing, JvmConfig, JvmRuntime};
use jni::objects::JValue;
use jni::{InitArgsBuilder, JNIVersion, JavaVM};

#[test]
fn a_loader_without_the_sdk_is_refused() {
    let lib = JvmRuntime::find_jvm(&JvmConfig::default()).unwrap();
    let args = InitArgsBuilder::new().version(JNIVersion::V8).option("-Xrs").build().unwrap();
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
    // SAFETY: the pointer comes from the live JavaVM above.
    let err =
        unsafe { attach_existing(jvm.get_java_vm_pointer(), loader, &JvmConfig::default()) }.err().expect("must fail");
    assert!(err.contains("not visible to the class loader"), "{err}");
}
