//! Per-mod, per-world key-value storage (`storage` interface).
//!
//! [`Store`] enforces the API limits on top of a [`KvBackend`]. Hosts choose the backend through a
//! [`StorageProvider`]: [`FileStorage`] (one file per mod, rewritten atomically on flush), [`MemoryStorage`]
//! (tests, worlds without a save), or their own (the server uses a separate LMDB environment). Storage never
//! touches the vanilla world save.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::types::ApiError;

/// Longest key in bytes.
pub const MAX_KEY: usize = 256;
/// Largest value in bytes (1 MiB).
pub const MAX_VALUE: usize = 1024 * 1024;
/// Most bytes (keys + values) per mod per world (64 MiB).
pub const MAX_TOTAL: u64 = 64 * 1024 * 1024;

/// A storage backend for one mod in one world.
pub trait KvBackend {
    /// Reads every entry.
    fn load(&mut self) -> io::Result<Vec<(String, Vec<u8>)>>;
    /// Writes one entry (may be buffered until [`KvBackend::flush`]).
    fn put(&mut self, key: &str, value: &[u8]) -> io::Result<()>;
    /// Removes one entry (may be buffered).
    fn delete(&mut self, key: &str) -> io::Result<()>;
    /// Makes every earlier write durable.
    fn flush(&mut self) -> io::Result<()>;
}

/// Opens backends per mod.
pub trait StorageProvider {
    /// Opens the store of `mod_id`.
    fn open(&self, mod_id: &str) -> io::Result<Box<dyn KvBackend>>;
}

/// A mod's store with the API limits applied. Reads are served from memory.
pub struct Store {
    backend: Box<dyn KvBackend>,
    map: BTreeMap<String, Vec<u8>>,
    used: u64,
}

fn entry_size(key: &str, value: &[u8]) -> u64 {
    (key.len() + value.len()) as u64
}

impl Store {
    /// Loads a store.
    pub fn open(mut backend: Box<dyn KvBackend>) -> io::Result<Store> {
        let map: BTreeMap<String, Vec<u8>> = backend.load()?.into_iter().collect();
        let used = map.iter().map(|(k, v)| entry_size(k, v)).sum();
        Ok(Store { backend, map, used })
    }

    /// Bytes used (keys + values).
    pub fn used(&self) -> u64 {
        self.used
    }

    /// `storage.get`.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.map.get(key).cloned()
    }

    /// `storage.set`.
    pub fn set(&mut self, key: &str, value: &[u8]) -> Result<(), ApiError> {
        if key.is_empty() || key.len() > MAX_KEY {
            return Err(ApiError::InvalidArgument(format!("key must be 1-{MAX_KEY} bytes")));
        }
        if value.len() > MAX_VALUE {
            return Err(ApiError::LimitExceeded(format!("value larger than {MAX_VALUE} bytes")));
        }
        let old = self.map.get(key).map_or(0, |v| entry_size(key, v));
        let new_used = self.used - old + entry_size(key, value);
        if new_used > MAX_TOTAL {
            return Err(ApiError::LimitExceeded(format!("more than {MAX_TOTAL} bytes per mod per world")));
        }
        self.backend.put(key, value).map_err(|e| ApiError::Internal(e.to_string()))?;
        self.map.insert(key.to_string(), value.to_vec());
        self.used = new_used;
        Ok(())
    }

    /// `storage.delete`. A backend failure is reported as `Err`.
    pub fn delete(&mut self, key: &str) -> io::Result<bool> {
        let Some(v) = self.map.get(key) else { return Ok(false) };
        let size = entry_size(key, v);
        self.backend.delete(key)?;
        self.map.remove(key);
        self.used -= size;
        Ok(true)
    }

    /// `storage.keys`: keys starting with `prefix`, in byte order.
    pub fn keys(&self, prefix: &str) -> Vec<String> {
        self.map.range(prefix.to_string()..).map(|(k, _)| k).take_while(|k| k.starts_with(prefix)).cloned().collect()
    }

    /// Makes the store durable.
    pub fn flush(&mut self) -> io::Result<()> {
        self.backend.flush()
    }
}

/// Mod id -> key -> value.
type Shared = Rc<RefCell<BTreeMap<String, BTreeMap<String, Vec<u8>>>>>;

/// Stores in memory, shared between opens (so a "reopened" world sees earlier writes).
#[derive(Clone, Default)]
pub struct MemoryStorage {
    data: Shared,
}

impl MemoryStorage {
    /// An empty storage.
    pub fn new() -> MemoryStorage {
        MemoryStorage::default()
    }
}

struct MemoryKv {
    data: Shared,
    mod_id: String,
}

impl KvBackend for MemoryKv {
    fn load(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
        let d = self.data.borrow();
        Ok(d.get(&self.mod_id).map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default())
    }
    fn put(&mut self, key: &str, value: &[u8]) -> io::Result<()> {
        self.data.borrow_mut().entry(self.mod_id.clone()).or_default().insert(key.to_string(), value.to_vec());
        Ok(())
    }
    fn delete(&mut self, key: &str) -> io::Result<()> {
        if let Some(m) = self.data.borrow_mut().get_mut(&self.mod_id) {
            m.remove(key);
        }
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl StorageProvider for MemoryStorage {
    fn open(&self, mod_id: &str) -> io::Result<Box<dyn KvBackend>> {
        Ok(Box::new(MemoryKv { data: self.data.clone(), mod_id: mod_id.to_string() }))
    }
}

/// One file per mod (`<dir>/<mod-id>.kv`), rewritten atomically (temp file + rename) on flush.
///
/// Format: `BHKV` 0x01, then per entry `u32 LE key length`, key, `u32 LE value length`, value.
#[derive(Debug, Clone)]
pub struct FileStorage {
    dir: PathBuf,
}

impl FileStorage {
    /// Storage under `dir` (created on the first flush).
    pub fn new(dir: impl Into<PathBuf>) -> FileStorage {
        FileStorage { dir: dir.into() }
    }
}

const FILE_MAGIC: &[u8; 5] = b"BHKV\x01";

struct FileKv {
    path: PathBuf,
    map: BTreeMap<String, Vec<u8>>,
    dirty: bool,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> io::Result<&'a [u8]> {
    if b.len() < n {
        return Err(bad("truncated storage file"));
    }
    let (h, t) = b.split_at(n);
    *b = t;
    Ok(h)
}

fn take_len(b: &mut &[u8]) -> io::Result<usize> {
    let l = take(b, 4)?;
    Ok(u32::from_le_bytes([l[0], l[1], l[2], l[3]]) as usize)
}

/// Parses a storage file.
pub fn decode_file(mut b: &[u8]) -> io::Result<Vec<(String, Vec<u8>)>> {
    if take(&mut b, FILE_MAGIC.len())? != FILE_MAGIC {
        return Err(bad("not a storage file"));
    }
    let mut out = Vec::new();
    while !b.is_empty() {
        let kl = take_len(&mut b)?;
        let k = String::from_utf8(take(&mut b, kl)?.to_vec()).map_err(|_| bad("key is not UTF-8"))?;
        let vl = take_len(&mut b)?;
        out.push((k, take(&mut b, vl)?.to_vec()));
    }
    Ok(out)
}

/// Serializes entries in the storage file format.
pub fn encode_file<'a>(entries: impl IntoIterator<Item = (&'a String, &'a Vec<u8>)>) -> Vec<u8> {
    let mut out = FILE_MAGIC.to_vec();
    for (k, v) in entries {
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v);
    }
    out
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    fs::create_dir_all(path.parent().expect("storage files live in a directory"))?;
    let tmp = path.with_extension("kv.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

impl KvBackend for FileKv {
    fn load(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
        let entries = match fs::read(&self.path) {
            Ok(b) => decode_file(&b)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        self.map = entries.iter().cloned().collect();
        Ok(entries)
    }
    fn put(&mut self, key: &str, value: &[u8]) -> io::Result<()> {
        self.map.insert(key.to_string(), value.to_vec());
        self.dirty = true;
        Ok(())
    }
    fn delete(&mut self, key: &str) -> io::Result<()> {
        self.map.remove(key);
        self.dirty = true;
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.dirty {
            write_atomic(&self.path, &encode_file(&self.map))?;
            self.dirty = false;
        }
        Ok(())
    }
}

impl StorageProvider for FileStorage {
    fn open(&self, mod_id: &str) -> io::Result<Box<dyn KvBackend>> {
        Ok(Box::new(FileKv { path: self.dir.join(format!("{mod_id}.kv")), map: BTreeMap::new(), dirty: false }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Failing {
        load: bool,
    }
    impl KvBackend for Failing {
        fn load(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
            if self.load {
                Err(io::Error::other("load"))
            } else {
                Ok(vec![("a".into(), b"1".to_vec())])
            }
        }
        fn put(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
            Err(io::Error::other("put"))
        }
        fn delete(&mut self, _: &str) -> io::Result<()> {
            Err(io::Error::other("delete"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush"))
        }
    }

    #[test]
    fn limits() {
        let mut s = Store::open(MemoryStorage::new().open("m").unwrap()).unwrap();
        assert!(matches!(s.set("", b"x"), Err(ApiError::InvalidArgument(_))));
        assert!(matches!(s.set(&"k".repeat(MAX_KEY + 1), b"x"), Err(ApiError::InvalidArgument(_))));
        s.set(&"k".repeat(MAX_KEY), b"x").unwrap();
        assert!(matches!(s.set("v", &vec![0; MAX_VALUE + 1]), Err(ApiError::LimitExceeded(_))));
        s.set("v", &vec![0; MAX_VALUE]).unwrap();
        assert_eq!(s.used(), (MAX_KEY + 1 + 1 + MAX_VALUE) as u64);
        // Fill to the quota with 1 MiB values (64 of them would exceed it together with the others).
        let mut i = 0;
        let err = loop {
            match s.set(&format!("f{i}"), &vec![1; MAX_VALUE]) {
                Ok(()) => i += 1,
                Err(e) => break e,
            }
        };
        assert!(matches!(err, ApiError::LimitExceeded(_)));
        assert!(s.used() <= MAX_TOTAL);
        // Replacing an entry with a smaller one is always fine at the quota.
        s.set("f0", b"small").unwrap();
    }

    #[test]
    fn get_set_delete_keys() {
        let mem = MemoryStorage::new();
        let mut s = Store::open(mem.open("m").unwrap()).unwrap();
        assert_eq!(s.get("a"), None);
        s.set("a:1", b"x").unwrap();
        s.set("a:2", b"y").unwrap();
        s.set("b", b"z").unwrap();
        s.set("b", b"zz").unwrap();
        assert_eq!(s.get("b"), Some(b"zz".to_vec()));
        assert_eq!(s.keys("a:"), vec!["a:1", "a:2"]);
        assert_eq!(s.keys(""), vec!["a:1", "a:2", "b"]);
        assert!(s.keys("c").is_empty());
        assert!(s.delete("a:1").unwrap());
        assert!(!s.delete("a:1").unwrap());
        assert_eq!(s.used(), 3 + 1 + 1 + 2);
        s.flush().unwrap();
        // Another mod's store is separate; reopening sees the data.
        let other = Store::open(mem.open("n").unwrap()).unwrap();
        assert!(other.keys("").is_empty());
        let again = Store::open(mem.open("m").unwrap()).unwrap();
        assert_eq!(again.keys(""), vec!["a:2", "b"]);
        // Deleting from a mod that never wrote anything.
        let mut fresh = mem.open("never").unwrap();
        fresh.delete("x").unwrap();
    }

    #[test]
    fn backend_errors() {
        assert!(Store::open(Box::new(Failing { load: true })).is_err());
        let mut s = Store::open(Box::new(Failing { load: false })).unwrap();
        assert!(matches!(s.set("b", b"2"), Err(ApiError::Internal(_))));
        assert!(s.delete("a").is_err());
        assert_eq!(s.get("a"), Some(b"1".to_vec()));
        assert!(s.flush().is_err());
    }

    #[test]
    fn file_storage_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = FileStorage::new(dir.path().join("kv"));
        let mut s = Store::open(fs_.open("m").unwrap()).unwrap();
        s.flush().unwrap(); // nothing dirty: no file
        assert!(!dir.path().join("kv").exists());
        s.set("k", b"v").unwrap();
        s.set("gone", b"x").unwrap();
        assert!(s.delete("gone").unwrap());
        s.flush().unwrap();
        let s2 = Store::open(fs_.open("m").unwrap()).unwrap();
        assert_eq!(s2.get("k"), Some(b"v".to_vec()));
        assert_eq!(s2.keys(""), vec!["k"]);
        let bytes = fs::read(dir.path().join("kv/m.kv")).unwrap();
        assert_eq!(decode_file(&bytes).unwrap(), vec![("k".to_string(), b"v".to_vec())]);
    }

    #[test]
    fn file_storage_errors() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: read fails with something other than NotFound.
        fs::create_dir_all(dir.path().join("m.kv")).unwrap();
        let st = FileStorage::new(dir.path());
        assert!(Store::open(st.open("m").unwrap()).is_err());
        // Corrupt file.
        fs::write(dir.path().join("c.kv"), b"nope").unwrap();
        assert!(Store::open(st.open("c").unwrap()).is_err());
        // Flush into a directory that turned into a file after the store was opened.
        let blocked = FileStorage::new(dir.path().join("later"));
        let mut s = Store::open(blocked.open("m").unwrap()).unwrap();
        fs::write(dir.path().join("later"), b"").unwrap();
        s.set("k", b"v").unwrap();
        assert!(s.flush().is_err());
        // Temp file cannot be created (the temp path is a directory).
        let st2 = FileStorage::new(dir.path().join("t"));
        fs::create_dir_all(dir.path().join("t/m.kv.tmp")).unwrap();
        let mut s = Store::open(st2.open("m").unwrap()).unwrap();
        s.set("k", b"v").unwrap();
        assert!(s.flush().is_err());
    }

    #[test]
    fn decode_errors() {
        assert!(decode_file(b"BHK").is_err());
        assert!(decode_file(b"XXXX\x01").is_err());
        assert!(decode_file(b"BHKV\x01\x01\x00").is_err());
        assert!(decode_file(b"BHKV\x01\x05\x00\x00\x00ab").is_err());
        assert!(decode_file(b"BHKV\x01\x01\x00\x00\x00\xff\x00\x00\x00\x00").is_err());
        let ok = encode_file(&BTreeMap::from([("a".to_string(), vec![1u8, 2])]));
        assert_eq!(decode_file(&ok).unwrap(), vec![("a".to_string(), vec![1, 2])]);
    }
}
