// SPDX-License-Identifier: GPL-3.0-or-later
//! Builds the vendored libopus (opus/) with `cc`: fixed point, no DNN extras (deep PLC, DRED, OSCE), NEON intrinsics
//! on ARM, SSE/SSE2 on x86-64. The source lists come from libopus' own `*_sources.mk` files so a version bump only
//! needs a new opus/ tree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("opus");
    let mk = parse_mk(&root, &["celt_sources.mk", "silk_sources.mk", "opus_sources.mk"]);
    let version = std::fs::read_to_string(root.join("package_version")).unwrap();
    let version = version.trim().trim_start_matches("PACKAGE_VERSION=").trim_matches('"').to_owned();

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    let mut groups = vec!["CELT_SOURCES", "SILK_SOURCES", "SILK_SOURCES_FIXED", "OPUS_SOURCES", "OPUS_SOURCES_FLOAT"];
    let mut b = cc::Build::new();
    b.warnings(false)
        .opt_level(2)
        .define("OPUS_BUILD", None)
        .define("FIXED_POINT", "1")
        .define("HAVE_LRINT", "1")
        .define("HAVE_LRINTF", "1")
        .define("PACKAGE_VERSION", format!("\"{version}\"").as_str());
    // MSVC has no C99 variable-length arrays.
    if env == "msvc" {
        b.define("USE_ALLOCA", None);
    } else {
        b.define("VAR_ARRAYS", "1");
    }
    match arch.as_str() {
        // armeabi-v7a: the NDK (r21+) assumes NEON, so take the intrinsics without run-time detection. The inline
        // ARMv5E/v6 asm helpers for the fixed-point multiplies are what libopus' configure enables on ARMv7 too.
        "arm" => {
            b.flag_if_supported("-mfpu=neon");
            for d in [
                "OPUS_ARM_MAY_HAVE_NEON_INTR",
                "OPUS_ARM_PRESUME_NEON_INTR",
                "OPUS_ARM_ASM",
                "OPUS_ARM_INLINE_ASM",
                "OPUS_ARM_INLINE_EDSP",
                "OPUS_ARM_INLINE_MEDIA",
                "OPUS_ARM_INLINE_NEON",
            ] {
                b.define(d, "1");
            }
            groups.extend([
                "CELT_SOURCES_ARM_NEON_INTR",
                "SILK_SOURCES_ARM_NEON_INTR",
                "SILK_SOURCES_FIXED_ARM_NEON_INTR",
            ]);
        }
        "aarch64" => {
            for d in ["OPUS_ARM_MAY_HAVE_NEON_INTR", "OPUS_ARM_PRESUME_NEON_INTR", "OPUS_ARM_PRESUME_AARCH64_NEON_INTR"]
            {
                b.define(d, "1");
            }
            groups.extend([
                "CELT_SOURCES_ARM_NEON_INTR",
                "SILK_SOURCES_ARM_NEON_INTR",
                "SILK_SOURCES_FIXED_ARM_NEON_INTR",
            ]);
        }
        // SSE and SSE2 are part of the x86-64 baseline. SSE4.1/AVX2 would need run-time CPU detection (not built).
        "x86_64" => {
            for d in
                ["OPUS_X86_MAY_HAVE_SSE", "OPUS_X86_PRESUME_SSE", "OPUS_X86_MAY_HAVE_SSE2", "OPUS_X86_PRESUME_SSE2"]
            {
                b.define(d, "1");
            }
            groups.extend(["CELT_SOURCES_SSE", "CELT_SOURCES_SSE2"]);
        }
        _ => {}
    }
    for dir in ["", "include", "celt", "silk", "silk/fixed"] {
        b.include(root.join(dir));
    }
    for g in groups {
        for f in mk.get(g).unwrap_or_else(|| panic!("{g} missing from the libopus .mk files")) {
            b.file(root.join(f));
        }
    }
    b.compile("opus");

    println!("cargo:include={}", root.join("include").display());
    println!("cargo:rerun-if-changed=opus");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Reads `NAME = a.c \ b.c` variable blocks from automake fragments.
fn parse_mk(root: &Path, files: &[&str]) -> HashMap<String, Vec<String>> {
    let mut vars = HashMap::new();
    for file in files {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        let mut current: Option<String> = None;
        for line in text.lines() {
            let (body, more) = match line.trim_end().strip_suffix('\\') {
                Some(b) => (b, true),
                None => (line, false),
            };
            let body = match (&current, body.split_once('=')) {
                (None, Some((name, rest))) => {
                    current = Some(name.trim().to_owned());
                    rest
                }
                _ => body,
            };
            if let Some(name) = &current {
                let list: &mut Vec<String> = vars.entry(name.clone()).or_default();
                list.extend(body.split_whitespace().map(str::to_owned));
            }
            if !more {
                current = None;
            }
        }
    }
    vars
}
