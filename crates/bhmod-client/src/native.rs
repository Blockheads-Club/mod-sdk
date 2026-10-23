//! Native libraries of client packages: which files a loader takes out of a `.bhmod` and where it puts them.
//!
//! A package keeps its libraries under `client/native/<abi>/<name>.so`. The Android loader needs real files to
//! `dlopen`; the PC runner reads the same files into its guest. Both extract with [`extract`] into a folder named
//! after the package's SHA-256, so a replaced package never reuses an old file and a leftover folder is harmless.

use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};

use bhmod_manifest::package::{Source, ZipSource};

/// The ABI of the game's native code (32-bit ARM).
pub const ABI: &str = "armeabi-v7a";
/// Libraries are small; this bounds what a package can make the loader write.
pub const MAX_LIB_BYTES: u64 = 32 << 20;

/// The library paths inside the package for `abi`, sorted.
pub fn libraries(src: &dyn Source, abi: &str) -> Vec<String> {
    let prefix = format!("client/native/{abi}/");
    let mut v: Vec<String> = src.files().iter().map(|f| f.path.clone()).filter(|p| p.starts_with(&prefix)).collect();
    v.sort();
    v
}

/// Writes the `abi` libraries of `package` to `<root>/<sha256>/<abi>/` and returns their paths (sorted). Files that
/// are already there with the right size are kept.
pub fn extract(package: &Path, sha256: &str, abi: &str, root: &Path) -> io::Result<Vec<PathBuf>> {
    let bytes = fs::read(package)?;
    let mut zip = ZipSource::open(Cursor::new(&bytes[..]), bytes.len() as u64)?;
    let dir = root.join(sha256).join(abi);
    let mut out = Vec::new();
    for path in libraries(&zip, abi) {
        let name = path.rsplit('/').next().expect("a path has a last part");
        let data = zip.read(&path, MAX_LIB_BYTES)?;
        let to = dir.join(name);
        if fs::metadata(&to).ok().map(|m| m.len()) != Some(data.len() as u64) {
            fs::create_dir_all(&dir)?;
            fs::write(&to, &data)?;
        }
        out.push(to);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::package_files;

    #[test]
    fn extracts_the_libraries_of_one_abi() {
        let t = tempfile::tempdir().unwrap();
        let bytes = package_files(
            "nat",
            "native = true\n",
            &[
                ("client/native/armeabi-v7a/libb.so", b"BBBB"),
                ("client/native/armeabi-v7a/liba.so", b"AA"),
                ("client/native/arm64-v8a/liba.so", b"64"),
            ],
        );
        let pkg = t.path().join("nat.bhmod");
        fs::write(&pkg, &bytes).unwrap();
        let root = t.path().join("native");
        let out = extract(&pkg, "abc", ABI, &root).unwrap();
        assert_eq!(out, [root.join("abc/armeabi-v7a/liba.so"), root.join("abc/armeabi-v7a/libb.so")]);
        assert_eq!(fs::read(&out[1]).unwrap(), b"BBBB");
        // a second run keeps files of the right size and repairs the others
        fs::write(&out[0], b"x").unwrap();
        assert_eq!(extract(&pkg, "abc", ABI, &root).unwrap(), out);
        assert_eq!(fs::read(&out[0]).unwrap(), b"AA");
        // no libraries for another ABI
        assert!(extract(&pkg, "abc", "x86", &root).unwrap().is_empty());
        // errors: missing package, not a zip
        assert!(extract(&t.path().join("none"), "abc", ABI, &root).is_err());
        fs::write(&pkg, b"junk").unwrap();
        assert!(extract(&pkg, "abc", ABI, &root).is_err());
    }
}
