//! A poll-based watcher of a mods directory (the developer option that reloads mods when their files change).
//!
//! No file-system notification API is used, so it works the same everywhere: [`Watcher::changed`] looks at the
//! names, sizes and modification times of every file below the directory (hidden entries such as `.cache` are
//! ignored) at most every [`INTERVAL`], and reports a change once the stamp has been the same on two checks in a row,
//! so a package that is still being copied is not read half-written.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often the directory is looked at.
pub const INTERVAL: Duration = Duration::from_millis(500);

fn visit(dir: &Path, h: &mut DefaultHasher) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for e in entries {
        e.file_name().hash(h);
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            visit(&e.path(), h);
        } else {
            meta.len().hash(h);
            meta.modified().ok().hash(h);
        }
    }
}

/// A hash of the names, sizes and modification times below `dir`.
pub fn stamp(dir: &Path) -> u64 {
    let mut h = DefaultHasher::new();
    visit(dir, &mut h);
    h.finish()
}

/// Watches one directory.
#[derive(Debug)]
pub struct Watcher {
    dir: PathBuf,
    applied: u64,
    pending: Option<u64>,
    next: Option<Instant>,
}

impl Watcher {
    /// Starts watching: the directory as it is now counts as seen.
    pub fn new(dir: impl Into<PathBuf>) -> Watcher {
        let dir = dir.into();
        let applied = stamp(&dir);
        Watcher { dir, applied, pending: None, next: None }
    }

    /// True once when the directory changed and is quiet again. Cheap to call on every poll.
    pub fn changed(&mut self, now: Instant) -> bool {
        if self.next.is_some_and(|n| now < n) {
            return false;
        }
        self.next = Some(now + INTERVAL);
        let s = stamp(&self.dir);
        if self.pending == Some(s) {
            self.pending = None;
            self.applied = s;
            return true;
        }
        self.pending = (s != self.applied).then_some(s);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_change_once_it_is_quiet() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        fs::create_dir_all(dir.join("m/server")).unwrap();
        fs::write(dir.join("m/server/main.luau"), "a").unwrap();
        fs::create_dir_all(dir.join(".cache")).unwrap();
        let mut w = Watcher::new(dir);
        let mut t = Instant::now();
        assert!(!w.changed(t));
        // Rate limited: no look before the interval passed.
        fs::write(dir.join("m/server/main.luau"), "bb").unwrap();
        assert!(!w.changed(t + INTERVAL / 2));
        t += INTERVAL;
        // First sight of the change: wait for a quiet check.
        assert!(!w.changed(t));
        fs::write(dir.join("m/server/main.luau"), "ccc").unwrap();
        t += INTERVAL;
        assert!(!w.changed(t), "still changing");
        t += INTERVAL;
        assert!(w.changed(t));
        t += INTERVAL;
        assert!(!w.changed(t));
        // Hidden entries are ignored; a new file counts.
        fs::write(dir.join(".cache/x"), "x").unwrap();
        t += INTERVAL;
        assert!(!w.changed(t));
        fs::write(dir.join("new.bhmod"), "z").unwrap();
        t += INTERVAL;
        assert!(!w.changed(t));
        t += INTERVAL;
        assert!(w.changed(t));
    }

    #[test]
    fn missing_directory_is_quiet() {
        let mut w = Watcher::new("/nonexistent/mods-dir");
        let t = Instant::now();
        assert!(!w.changed(t));
        assert!(!w.changed(t + INTERVAL));
        assert_eq!(stamp(Path::new("/nonexistent/mods-dir")), stamp(Path::new("/nonexistent/other")));
    }
}
