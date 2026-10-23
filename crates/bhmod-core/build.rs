//! With the `jvm` feature: compiles the Java SDK (`sdk/java/src`: the generated `blockheads:api` bindings and the
//! runtime glue) into `bhmod-sdk.jar`, which the JVM runtime embeds and puts on the JVM's class path.
//!
//! Needs `javac` and `jar` (a JDK 8 or newer; `JAVA_HOME` is honoured). Java 8 bytecode, so the same classes run on
//! any JRE and on Android (ART, through d8).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tool(name: &str) -> PathBuf {
    for var in ["BHMOD_JAVA_HOME", "JAVA_HOME"] {
        if let Some(home) = env::var_os(var) {
            let p = Path::new(&home).join("bin").join(name);
            if p.is_file() {
                return p;
            }
        }
    }
    PathBuf::from(name)
}

fn java_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read the Java SDK sources").flatten() {
        let p = e.path();
        if p.is_dir() {
            java_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "java") {
            out.push(p);
        }
    }
}

fn run(cmd: &mut Command, what: &str) {
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("{what} failed ({s}); the `jvm` feature needs a JDK 8+ (javac, jar)"),
        Err(e) => panic!("cannot run {what}: {e}; the `jvm` feature needs a JDK 8+ (javac, jar; set JAVA_HOME)"),
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");
    println!("cargo:rerun-if-env-changed=BHMOD_JAVA_HOME");
    if env::var_os("CARGO_FEATURE_JVM").is_none() {
        return;
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let src = manifest.join("../../sdk/java/src");
    println!("cargo:rerun-if-changed={}", src.display());
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let classes = out.join("java-classes");
    let _ = std::fs::remove_dir_all(&classes);
    std::fs::create_dir_all(&classes).expect("create the class directory");
    let mut files = Vec::new();
    java_files(&src, &mut files);
    files.sort();
    run(
        Command::new(tool("javac"))
            .args(["-nowarn", "-Xlint:-options", "--release", "8", "-encoding", "UTF-8", "-d"])
            .arg(&classes)
            .args(&files),
        "javac",
    );
    run(Command::new(tool("jar")).arg("cf").arg(out.join("bhmod-sdk.jar")).arg("-C").arg(&classes).arg("."), "jar");
}
