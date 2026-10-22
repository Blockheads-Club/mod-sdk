//! Mod discovery: every `.bhmod` zip and every unpacked mod directory (a directory holding `mod.toml`) in the mods
//! directory, checked with the same rules as `bhmod check`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use bhmod_manifest::package::{self, CheckOptions, DirSource, Source, ZipSource};
use bhmod_manifest::Manifest;

use crate::runtime::ModFiles;
use crate::types::Side;

/// A valid package read into memory.
#[derive(Debug, Clone)]
pub struct Package {
    /// The validated manifest.
    pub manifest: Manifest,
    /// Where it was found.
    pub origin: PathBuf,
    /// SHA-256 of the zip (`None` for an unpacked directory).
    pub sha256: Option<String>,
    /// Every file under `server/`, `client/` and `data/`.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    /// The files a runtime gets for `side` (`None` when the mod has no entry on that side).
    pub fn side_files(&self, side: Side) -> Option<ModFiles> {
        let entry = match side {
            Side::Server => self.manifest.entry.server.as_ref()?,
            Side::Client => self.manifest.entry.client.as_ref()?,
        };
        let prefix = format!("{}/", side.name());
        let files = self
            .files
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix) || p.starts_with("data/"))
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect();
        Some(ModFiles { entry: entry.path.clone(), files })
    }
}

/// A package that could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The file or directory.
    pub origin: PathBuf,
    /// What is wrong.
    pub message: String,
}

fn load(src: &mut dyn Source, origin: &Path, sha256: Option<String>) -> Result<Package, String> {
    let report = package::check(src, CheckOptions::default());
    let manifest = match report.manifest {
        Some(m) if report.errors.is_empty() => m,
        _ => {
            let msgs: Vec<String> = report.errors.iter().map(|i| i.to_string()).collect();
            return Err(msgs.join("; "));
        }
    };
    let mut files = BTreeMap::new();
    for f in src.files().to_vec() {
        if ["server/", "client/", "data/"].iter().any(|p| f.path.starts_with(p)) {
            let data = src.read(&f.path, f.size).map_err(|e| format!("{}: {e}", f.path))?;
            files.insert(f.path, data);
        }
    }
    Ok(Package { manifest, origin: origin.to_path_buf(), sha256, files })
}

fn load_zip(path: &Path) -> Result<Package, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let sha = bhmod_manifest::pack::sha256_hex(&bytes);
    let len = bytes.len() as u64;
    let mut src = ZipSource::open(Cursor::new(bytes), len).map_err(|e| format!("not a valid zip: {e}"))?;
    load(&mut src, path, Some(sha))
}

fn load_dir(path: &Path) -> Result<Package, String> {
    let mut src = DirSource::open(path).map_err(|e| e.to_string())?;
    load(&mut src, path, None)
}

/// Reads every package in `dir`, in file name order. A missing directory has no mods.
pub fn discover(dir: &Path) -> (Vec<Package>, Vec<Problem>) {
    let mut packages = Vec::new();
    let mut problems = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return (packages, problems) };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let result = if path.is_dir() {
            if !path.join("mod.toml").is_file() {
                continue;
            }
            load_dir(&path)
        } else if name.ends_with(".bhmod") {
            load_zip(&path)
        } else {
            continue;
        };
        match result {
            Ok(p) => packages.push(p),
            Err(message) => problems.push(Problem { origin: path, message }),
        }
    }
    (packages, problems)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bhmod_manifest::pack::write_zip;

    pub const LUA_TOML: &str = "schema = 1\nid = \"ID\"\nversion = \"VER\"\nname = { en = \"N\" }\nauthors = [\"a\"]\nlicense = \"MIT\"\napi = \"^0.1\"\n[sides]\nserver = true\nclient = false\n[entry]\nserver = { runtime = \"lua\", path = \"server/main.luau\" }\n";

    pub fn write_mod(root: &Path, dir: &str, id: &str, extra: &str, main: &str) -> PathBuf {
        let d = root.join(dir);
        fs::create_dir_all(d.join("server")).unwrap();
        fs::write(d.join("mod.toml"), format!("{}{extra}", LUA_TOML.replace("ID", id).replace("VER", "1.0.0")))
            .unwrap();
        fs::write(d.join("server/main.luau"), main).unwrap();
        d
    }

    #[test]
    fn discovers_dirs_and_zips() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Source dir for the zip lives outside the mods dir.
        let src_root = tempfile::tempdir().unwrap();
        let zsrc = write_mod(src_root.path(), "z", "zipped", "", "return {}");
        fs::create_dir_all(zsrc.join("data")).unwrap();
        fs::write(zsrc.join("data/cfg.txt"), "x").unwrap();
        let mut ds = DirSource::open(&zsrc).unwrap();
        let out = write_zip(&mut ds, Cursor::new(Vec::new())).unwrap().into_inner();
        fs::write(root.join("zipped-1.0.0.bhmod"), &out).unwrap();
        write_mod(root, "b-dir", "unpacked", "", "return {}");
        fs::create_dir_all(root.join("c-not-a-mod")).unwrap();
        fs::create_dir_all(root.join(".cache")).unwrap();
        fs::write(root.join("mods.toml"), "").unwrap();
        fs::write(root.join("broken.bhmod"), b"not a zip").unwrap();
        let bad = write_mod(root, "d-bad", "bad", "", "");
        fs::remove_file(bad.join("server/main.luau")).unwrap();

        let (pkgs, problems) = discover(root);
        let ids: Vec<_> = pkgs.iter().map(|p| p.manifest.id.as_str()).collect();
        assert_eq!(ids, vec!["unpacked", "zipped"]);
        assert!(pkgs[0].sha256.is_none());
        assert_eq!(pkgs[1].sha256.as_deref(), Some(bhmod_manifest::pack::sha256_hex(&out).as_str()));
        let files = pkgs[1].side_files(Side::Server).unwrap();
        assert_eq!(files.entry, "server/main.luau");
        assert_eq!(files.files.keys().collect::<Vec<_>>(), vec!["data/cfg.txt", "server/main.luau"]);
        assert!(pkgs[1].side_files(Side::Client).is_none());
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].origin.ends_with("broken.bhmod"));
        assert!(problems[0].message.starts_with("not a valid zip"));
        assert!(problems[1].origin.ends_with("d-bad"));
    }

    #[test]
    fn missing_dir_and_unreadable() {
        let (p, q) = discover(Path::new("/nonexistent/bhmod-core-test"));
        assert!(p.is_empty() && q.is_empty());
        assert!(load_zip(Path::new("/nonexistent/x.bhmod")).is_err());
        assert!(load_dir(Path::new("/nonexistent/x")).is_err());
    }

    /// A source whose listing claims a file that cannot be read.
    struct Lying(DirSource);
    impl Source for Lying {
        fn files(&self) -> &[package::FileEntry] {
            self.0.files()
        }
        fn listing_issues(&self) -> &[bhmod_manifest::Issue] {
            self.0.listing_issues()
        }
        fn read(&mut self, path: &str, limit: u64) -> std::io::Result<Vec<u8>> {
            if path.starts_with("data/") {
                Err(std::io::Error::other("gone"))
            } else {
                self.0.read(path, limit)
            }
        }
        fn compressed_size(&self) -> Option<u64> {
            None
        }
    }

    #[test]
    fn read_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let d = write_mod(tmp.path(), "mm", "mm", "", "return {}");
        fs::create_dir_all(d.join("data")).unwrap();
        fs::write(d.join("data/x.txt"), "x").unwrap();
        let mut src = Lying(DirSource::open(&d).unwrap());
        assert_eq!(src.compressed_size(), None);
        let err = load(&mut src, &d, None).unwrap_err();
        assert!(err.contains("gone"), "{err}");
    }
}
