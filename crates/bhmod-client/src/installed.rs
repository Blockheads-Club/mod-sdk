//! Mods the player installed (from the public index, a link or a zip): `<id>.bhmod` files in one directory plus
//! `installed.toml` with the on/off switch and where each came from. Packages are checked on install like a
//! server's, so the same rules apply (a client half, a runtime this loader has). Native code is the exception that
//! needs two things: the package came from the public index (`source` starts with [`INDEX_SOURCE`]) and the player
//! confirmed the red warning; the SHA-256 of the confirmed file is stored, so a replaced file is not loaded.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bhmod_manifest::Manifest;

use crate::cache::sha256;
use crate::verify::{check_package_with, Caps, Refusal};

/// The state file.
pub const STATE_FILE: &str = "installed.toml";
/// Prefix of `source` for packages downloaded from the public index (the only way to install native code).
pub const INDEX_SOURCE: &str = "index:";

/// State key: the SHA-256 of the native package the player confirmed.
const NATIVE_KEY: &str = "native-sha256";

/// One installed mod.
#[derive(Debug, Clone)]
pub struct InstalledMod {
    /// The package.
    pub path: PathBuf,
    /// Its manifest.
    pub manifest: Manifest,
    /// SHA-256 (hex) of the package.
    pub sha256: String,
    /// Runs when a world opens.
    pub enabled: bool,
    /// Where it came from (index URL or link), if known.
    pub source: Option<String>,
}

/// Why an install failed.
#[derive(Debug)]
pub enum InstallError {
    /// The package was refused.
    Refused(Refusal),
    /// The file system failed.
    Io(io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Refused(r) => r.fmt(f),
            InstallError::Io(e) => write!(f, "could not save the mod: {e}"),
        }
    }
}

/// The installed-mods directory.
#[derive(Debug, Clone)]
pub struct Installed {
    dir: PathBuf,
    caps: Caps,
}

impl Installed {
    /// Mods in `dir`, checked against `caps`.
    pub fn new(dir: impl Into<PathBuf>, caps: Caps) -> Installed {
        Installed { dir: dir.into(), caps }
    }

    /// The directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn state(&self) -> toml::Table {
        fs::read_to_string(self.dir.join(STATE_FILE)).ok().and_then(|t| t.parse().ok()).unwrap_or_default()
    }

    fn save(&self, t: &toml::Table) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.dir.join(STATE_FILE), t.to_string())
    }

    /// Checks and installs a package (replacing another version of the same mod). New mods start enabled. Native
    /// packages are refused: see [`Installed::install_with`].
    pub fn install(&self, bytes: &[u8], source: Option<&str>) -> Result<InstalledMod, InstallError> {
        self.install_with(bytes, source, false)
    }

    /// Like [`Installed::install`]; `native_confirmed` says the player confirmed the native-code warning for this
    /// file. A native package is installed only when it is confirmed and `source` is an [`INDEX_SOURCE`].
    pub fn install_with(
        &self,
        bytes: &[u8],
        source: Option<&str>,
        native_confirmed: bool,
    ) -> Result<InstalledMod, InstallError> {
        let manifest = check_package_with(bytes, &self.caps, true).map_err(InstallError::Refused)?;
        let native = manifest.permissions.native;
        if native && !source.is_some_and(|s| s.starts_with(INDEX_SOURCE)) {
            return Err(InstallError::Refused(Refusal::Native));
        }
        if native && !native_confirmed {
            return Err(InstallError::Refused(Refusal::NativeUnconfirmed));
        }
        fs::create_dir_all(&self.dir).map_err(InstallError::Io)?;
        let path = self.dir.join(format!("{}.bhmod", manifest.id));
        fs::write(&path, bytes).map_err(InstallError::Io)?;
        let mut t = self.state();
        let mut entry = t.get(&manifest.id).and_then(|v| v.as_table()).cloned().unwrap_or_default();
        entry.entry("enabled").or_insert(toml::Value::Boolean(true));
        match source {
            Some(s) => entry.insert("source".into(), toml::Value::String(s.into())),
            None => entry.remove("source"),
        };
        let enabled = entry["enabled"].as_bool().unwrap_or(true);
        let sha = bhmod_proto::hex(&sha256(bytes));
        if native {
            entry.insert(NATIVE_KEY.into(), toml::Value::String(sha.clone()));
        } else {
            entry.remove(NATIVE_KEY);
        }
        t.insert(manifest.id.clone(), toml::Value::Table(entry));
        self.save(&t).map_err(InstallError::Io)?;
        Ok(InstalledMod { path, manifest, sha256: sha, enabled, source: source.map(str::to_string) })
    }

    /// Every installed mod that still passes the checks, sorted by id.
    pub fn list(&self) -> Vec<InstalledMod> {
        let t = self.state();
        let mut out: Vec<InstalledMod> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "bhmod"))
            .filter_map(|path| {
                let bytes = fs::read(&path).ok()?;
                let manifest = check_package_with(&bytes, &self.caps, true).ok()?;
                let entry = t.get(&manifest.id).and_then(|v| v.as_table());
                let sha256 = bhmod_proto::hex(&sha256(&bytes));
                let confirmed = entry.and_then(|e| e.get(NATIVE_KEY)).and_then(|v| v.as_str()) == Some(sha256.as_str());
                if manifest.permissions.native && !confirmed {
                    return None;
                }
                let enabled = entry.and_then(|e| e.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(true);
                let source = entry.and_then(|e| e.get("source")).and_then(|v| v.as_str()).map(str::to_string);
                Some(InstalledMod { path, manifest, sha256, enabled, source })
            })
            .collect();
        out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        out
    }

    /// Turns a mod on or off. False when it is not installed.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> io::Result<bool> {
        if !self.dir.join(format!("{id}.bhmod")).is_file() {
            return Ok(false);
        }
        let mut t = self.state();
        let mut entry = t.get(id).and_then(|v| v.as_table()).cloned().unwrap_or_default();
        entry.insert("enabled".into(), toml::Value::Boolean(enabled));
        t.insert(id.into(), toml::Value::Table(entry));
        self.save(&t)?;
        Ok(true)
    }

    /// Uninstalls a mod. False when it is not installed.
    pub fn remove(&self, id: &str) -> io::Result<bool> {
        let p = self.dir.join(format!("{id}.bhmod"));
        if !p.is_file() {
            return Ok(false);
        }
        fs::remove_file(p)?;
        let mut t = self.state();
        t.remove(id);
        self.save(&t)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{caps, package};

    #[test]
    fn install_toggle_remove() {
        let t = tempfile::tempdir().unwrap();
        let i = Installed::new(t.path().join("m"), caps());
        assert_eq!(i.dir(), t.path().join("m"));
        assert!(i.list().is_empty());
        let m = i.install(&package("alpha", "1.0.0", "", ""), Some("https://x/a.bhmod")).unwrap();
        assert!(m.enabled);
        assert_eq!(m.source.as_deref(), Some("https://x/a.bhmod"));
        i.install(&package("beta", "1.0.0", "", ""), None).unwrap();
        assert!(i.set_enabled("alpha", false).unwrap());
        assert!(!i.set_enabled("nope", false).unwrap());
        // an update keeps the switch and drops the old source
        let m = i.install(&package("alpha", "1.1.0", "", ""), None).unwrap();
        assert!(!m.enabled);
        assert_eq!(m.source, None);
        // junk in the directory is skipped
        fs::write(i.dir().join("junk.bhmod"), b"no").unwrap();
        fs::write(i.dir().join("notes.txt"), b"no").unwrap();
        let l = i.list();
        assert_eq!(
            l.iter().map(|m| (m.manifest.id.as_str(), m.enabled)).collect::<Vec<_>>(),
            [("alpha", false), ("beta", true)]
        );
        assert_eq!(l[0].manifest.version, "1.1.0");
        assert_eq!(l[0].sha256.len(), 64);
        assert!(i.remove("alpha").unwrap());
        assert!(!i.remove("alpha").unwrap());
        assert_eq!(i.list().len(), 1);
    }

    #[test]
    fn native_code_needs_the_index_and_a_confirmation() {
        use crate::testutil::package_files;
        let t = tempfile::tempdir().unwrap();
        let i = Installed::new(t.path(), caps());
        let nat = package_files("nat", "native = true\n", &[("client/native/armeabi-v7a/libn.so", b"ELF")]);
        let index = Some("index:https://x/nat.bhmod");
        // not from the index, or not confirmed: refused, nothing written
        let e = i.install_with(&nat, Some("https://x/nat.bhmod"), true).unwrap_err();
        assert!(matches!(e, InstallError::Refused(Refusal::Native)), "{e}");
        let e = i.install_with(&nat, index, false).unwrap_err();
        assert!(matches!(e, InstallError::Refused(Refusal::NativeUnconfirmed)));
        assert!(e.to_string().contains("confirmation"));
        assert!(i.list().is_empty());
        // confirmed from the index: installed and listed
        let m = i.install_with(&nat, index, true).unwrap();
        assert!(m.manifest.permissions.native);
        assert_eq!(i.list().len(), 1);
        // a replaced file is not the confirmed one: skipped
        let other = package_files("nat", "native = true\n", &[("client/native/armeabi-v7a/libn.so", b"EVIL")]);
        fs::write(&m.path, other).unwrap();
        assert!(i.list().is_empty());
        // an update without native code drops the confirmation
        i.install(&package("nat", "1.1.0", "", ""), None).unwrap();
        assert_eq!(i.list().len(), 1);
        fs::write(&m.path, &nat).unwrap();
        assert!(i.list().is_empty());
    }

    #[test]
    fn refusals_and_io_errors() {
        let t = tempfile::tempdir().unwrap();
        let i = Installed::new(t.path(), caps());
        let e = i.install(&package("natmod", "1.0.0", "native = true\n", ""), None).unwrap_err();
        assert!(matches!(e, InstallError::Refused(Refusal::Native)));
        assert!(!e.to_string().is_empty());
        // the directory is a file
        let f = t.path().join("file");
        fs::write(&f, b"x").unwrap();
        let i = Installed::new(&f, caps());
        let e = i.install(&package("alpha", "1.0.0", "", ""), None).unwrap_err();
        assert!(e.to_string().starts_with("could not save"));
        // the package file is a directory
        let i = Installed::new(t.path().join("d"), caps());
        fs::create_dir_all(i.dir().join("alpha.bhmod")).unwrap();
        assert!(matches!(i.install(&package("alpha", "1.0.0", "", ""), None), Err(InstallError::Io(_))));
        // the state file cannot be written
        let i = Installed::new(t.path().join("s"), caps());
        fs::create_dir_all(i.dir().join(STATE_FILE)).unwrap();
        assert!(matches!(i.install(&package("alpha", "1.0.0", "", ""), None), Err(InstallError::Io(_))));
        assert!(i.set_enabled("alpha", true).is_err());
        assert!(i.remove("alpha").is_err());
    }
}
