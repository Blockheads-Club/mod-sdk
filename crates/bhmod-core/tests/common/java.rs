//! Compiles the Java fixtures (`tests/jvm-fixtures`) against the embedded SDK and packs them as mod jars.
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use bhmod_core::jvm::SDK_JAR;

fn tool(name: &str) -> PathBuf {
    for var in ["BHMOD_JAVA_HOME", "JAVA_HOME"] {
        if let Some(home) = std::env::var_os(var) {
            let p = Path::new(&home).join("bin").join(name);
            if p.is_file() {
                return p;
            }
        }
    }
    PathBuf::from(name)
}

fn java_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            java_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "java") {
            out.push(p);
        }
    }
}

fn class_files(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            class_files(&p, root, out);
        } else {
            let name = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((name, fs::read(&p).unwrap()));
        }
    }
}

/// The compiled fixture classes: (path in the jar, bytes). Built once per test process.
fn classes() -> &'static Vec<(String, Vec<u8>)> {
    static CLASSES: OnceLock<Vec<(String, Vec<u8>)>> = OnceLock::new();
    CLASSES.get_or_init(|| {
        let work = std::env::temp_dir().join(format!("bhmod-jvm-fixtures-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join("classes")).unwrap();
        let sdk = work.join("sdk.jar");
        fs::write(&sdk, SDK_JAR).unwrap();
        let mut sources = Vec::new();
        java_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/jvm-fixtures"), &mut sources);
        let out = Command::new(tool("javac"))
            .args(["-nowarn", "-Xlint:-options", "--release", "8", "-cp"])
            .arg(&sdk)
            .arg("-d")
            .arg(work.join("classes"))
            .args(&sources)
            .output()
            .expect("run javac");
        assert!(out.status.success(), "javac failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let mut files = Vec::new();
        class_files(&work.join("classes"), &work.join("classes"), &mut files);
        let _ = fs::remove_dir_all(&work);
        files
    })
}

/// A jar with every fixture class and `main` as `Bhmod-Main` (`None`: a manifest without a main class).
pub fn jar(main: Option<&str>) -> Vec<u8> {
    jar_of(classes(), main)
}

/// A jar of the given files.
pub fn jar_of(files: &[(String, Vec<u8>)], main: Option<&str>) -> Vec<u8> {
    let mf = match main {
        Some(m) => format!("Manifest-Version: 1.0\r\nBhmod-Main: {m}\r\n\r\n"),
        None => "Manifest-Version: 1.0\r\n\r\n".to_string(),
    };
    jar_with_manifest(files, &mf)
}

/// A jar of the given files with this manifest text.
pub fn jar_with_manifest(files: &[(String, Vec<u8>)], manifest: &str) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("META-INF/MANIFEST.MF", o).unwrap();
    z.write_all(manifest.as_bytes()).unwrap();
    for (name, bytes) in files {
        z.start_file(name, o).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// Writes an unpacked JVM mod `id` (main class `main`, permissions `native` and `channels`) into `mods`; `extra` is
/// appended to `mod.toml` (for example `[provides]`).
pub fn jvm_mod(mods: &Path, id: &str, main: &str, extra: &str) {
    let d = mods.join(id);
    fs::create_dir_all(d.join("server")).unwrap();
    let toml = format!(
        "schema = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = {{ en = \"N\" }}\nauthors = [\"a\"]\nlicense = \"MIT\"\napi = \"^0.2\"\n[sides]\nserver = true\nclient = false\n[entry]\nserver = {{ runtime = \"jvm\", path = \"server/mod.jar\" }}\n[permissions]\nnative = true\nchannels = true\n{extra}"
    );
    fs::write(d.join("mod.toml"), toml).unwrap();
    fs::write(d.join("server/mod.jar"), jar(Some(main))).unwrap();
    if extra.contains("wit/calc.wit") {
        fs::create_dir_all(d.join("wit")).unwrap();
        fs::write(d.join("wit/calc.wit"), "package test:calc@1.0.0;\ninterface calc {}\n").unwrap();
    }
}
