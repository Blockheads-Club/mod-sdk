//! The package cache: complete packages as `<sha256 hex>.bhmod`, transfers in progress as `<sha256 hex>.part`.
//! Keyed by SHA-256 only, so one copy serves every server that offers the same package.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bhmod_proto::{hex, Hash, CHUNK_SIZE};
use sha2::{Digest, Sha256};

/// A cache directory.
#[derive(Debug, Clone)]
pub struct PackageCache {
    dir: PathBuf,
}

/// Parses 64 lowercase hex characters.
pub fn parse_hash(s: &str) -> Option<Hash> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut h = [0u8; 32];
    for (i, b) in h.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(h)
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> Hash {
    Sha256::digest(data).into()
}

impl PackageCache {
    /// A cache in `dir` (created on first write).
    pub fn new(dir: impl Into<PathBuf>) -> PackageCache {
        PackageCache { dir: dir.into() }
    }

    /// The cache directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, sha: &Hash, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", hex(sha)))
    }

    /// The complete package, when cached.
    pub fn get(&self, sha: &Hash) -> Option<PathBuf> {
        let p = self.path(sha, "bhmod");
        p.is_file().then_some(p)
    }

    /// Hashes of every complete package, sorted.
    pub fn cached(&self) -> Vec<Hash> {
        let mut out: Vec<Hash> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| parse_hash(e.file_name().to_str()?.strip_suffix(".bhmod")?))
            .collect();
        out.sort();
        out
    }

    /// Bytes of a partial transfer that can be resumed: its length rounded down to a whole chunk.
    pub fn resume_offset(&self, sha: &Hash) -> u64 {
        let len = fs::metadata(self.path(sha, "part")).map(|m| m.len()).unwrap_or(0);
        len / CHUNK_SIZE as u64 * CHUNK_SIZE as u64
    }

    /// Writes `data` at `offset` of the partial file (the file is cut to `offset` first, so a resumed transfer
    /// overwrites a torn last chunk).
    pub fn write_part(&self, sha: &Hash, offset: u64, data: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.path(sha, "part"))?;
        f.set_len(offset)?;
        io::Seek::seek(&mut f, io::SeekFrom::Start(offset))?;
        f.write_all(data)
    }

    /// Checks the SHA-256 of the complete partial file and moves it into the cache. A mismatch deletes the file.
    pub fn finish(&self, sha: &Hash) -> io::Result<bool> {
        let part = self.path(sha, "part");
        let data = fs::read(&part)?;
        if sha256(&data) != *sha {
            fs::remove_file(&part)?;
            return Ok(false);
        }
        fs::rename(&part, self.path(sha, "bhmod"))?;
        Ok(true)
    }

    /// Stores a complete package received some other way (a link or the index): its hash and path.
    pub fn put(&self, data: &[u8]) -> io::Result<(Hash, PathBuf)> {
        let sha = sha256(data);
        fs::create_dir_all(&self.dir)?;
        let p = self.path(&sha, "bhmod");
        let tmp = self.path(&sha, "tmp");
        fs::write(&tmp, data)?;
        fs::rename(&tmp, &p)?;
        Ok((sha, p))
    }

    /// Removes a package and its partial file.
    pub fn remove(&self, sha: &Hash) {
        let _ = fs::remove_file(self.path(sha, "bhmod"));
        let _ = fs::remove_file(self.path(sha, "part"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_resume_finish() {
        let t = tempfile::tempdir().unwrap();
        let c = PackageCache::new(t.path().join("c"));
        assert_eq!(c.dir(), t.path().join("c"));
        let data: Vec<u8> = (0..CHUNK_SIZE + 10).map(|i| i as u8).collect();
        let sha = sha256(&data);
        assert!(c.cached().is_empty());
        assert_eq!(c.resume_offset(&sha), 0);
        c.write_part(&sha, 0, &data[..100]).unwrap();
        assert_eq!(c.resume_offset(&sha), 0);
        c.write_part(&sha, 0, &data[..CHUNK_SIZE]).unwrap();
        c.write_part(&sha, CHUNK_SIZE as u64, &data[CHUNK_SIZE..CHUNK_SIZE + 5]).unwrap();
        assert_eq!(c.resume_offset(&sha), CHUNK_SIZE as u64);
        c.write_part(&sha, CHUNK_SIZE as u64, &data[CHUNK_SIZE..]).unwrap();
        assert!(c.get(&sha).is_none());
        assert!(c.finish(&sha).unwrap());
        assert!(c.get(&sha).is_some());
        assert_eq!(c.cached(), vec![sha]);
        // junk files are ignored
        fs::write(c.dir().join("zz.bhmod"), b"x").unwrap();
        fs::write(c.dir().join(format!("{}.bhmod", "g".repeat(64))), b"x").unwrap();
        assert_eq!(c.cached(), vec![sha]);
        c.remove(&sha);
        assert!(c.cached().is_empty());
    }

    #[test]
    fn mismatch_deletes_and_put() {
        let t = tempfile::tempdir().unwrap();
        let c = PackageCache::new(t.path());
        let sha = [7u8; 32];
        c.write_part(&sha, 0, b"abc").unwrap();
        assert!(!c.finish(&sha).unwrap());
        assert!(c.finish(&sha).is_err());
        let (h, p) = c.put(b"hello").unwrap();
        assert_eq!(h, sha256(b"hello"));
        assert_eq!(fs::read(p).unwrap(), b"hello");
    }

    #[test]
    fn hashes() {
        let h = sha256(b"x");
        assert_eq!(parse_hash(&hex(&h)), Some(h));
        assert_eq!(parse_hash("ab"), None);
        assert_eq!(parse_hash(&"A".repeat(64)), None);
    }
}
