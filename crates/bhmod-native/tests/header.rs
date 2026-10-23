//! `include/bhmod.h` and `abi.rs` describe the same table: compile the header for armv7 with static assertions
//! generated from the Rust description.

use bhmod_native::abi::{func_offset, Code, ABI_VERSION, DATA_FIELDS, FUNCS, TABLE_SIZE};
use std::path::PathBuf;
use std::process::Command;

fn clang() -> String {
    if let Ok(ndk) = std::env::var("ANDROID_NDK_HOME") {
        let p = PathBuf::from(ndk).join("toolchains/llvm/prebuilt/linux-x86_64/bin/clang");
        if p.exists() {
            return p.display().to_string();
        }
    }
    "clang".into()
}

#[test]
fn header_matches_the_rust_layout() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = String::from("#include \"bhmod.h\"\n#include <stddef.h>\n");
    for (name, off) in DATA_FIELDS {
        c += &format!("_Static_assert(offsetof(bhmod_api_t, {name}) == {off}, \"{name}\");\n");
    }
    for f in FUNCS {
        c += &format!("_Static_assert(offsetof(bhmod_api_t, {f}) == {}, \"{f}\");\n", func_offset(f).unwrap());
    }
    c += &format!("_Static_assert(sizeof(bhmod_api_t) == {TABLE_SIZE}, \"size\");\n");
    c += &format!("_Static_assert(BHMOD_ABI_VERSION == {ABI_VERSION}, \"abi\");\n");
    for (name, code) in [
        ("ARG", Code::Arg),
        ("NOT_FOUND", Code::NotFound),
        ("THUMB", Code::Thumb),
        ("RELOC", Code::Reloc),
        ("EXISTS", Code::Exists),
        ("MEMORY", Code::Memory),
        ("HANDLE", Code::Handle),
    ] {
        c += &format!("_Static_assert(BHMOD_E_{name} == {}, \"{name}\");\n", code.ret());
    }
    let src = dir.path().join("check.c");
    std::fs::write(&src, c).unwrap();
    let include = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
    let out = Command::new(clang())
        .args(["--target=armv7a-linux-androideabi24", "-ffreestanding", "-fsyntax-only", "-Wall", "-Werror", "-I"])
        .arg(&include)
        .arg(&src)
        .output()
        .expect("clang is needed for the header check");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(func_offset("nope"), None);
}
