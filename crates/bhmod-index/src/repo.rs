//! Loading an index repository checkout (`mods/<letter>/<id>.toml`).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use crate::entry::Entry;
use crate::Issue;

/// One loaded entry file.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// Path relative to the repo root, `mods/<letter>/<id>.toml`.
    pub path: String,
    /// JSON form of the file.
    pub json: Value,
    /// Typed form.
    pub entry: Entry,
}

/// Every entry of an index checkout plus the problems found while reading it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Repo {
    /// Entries by id.
    pub entries: BTreeMap<String, Loaded>,
    /// Bad file names, unreadable or schema-invalid files.
    pub issues: Vec<Issue>,
}

impl Repo {
    /// Reads `<root>/mods`. A missing `mods/` folder is an empty index.
    pub fn load(root: &Path) -> io::Result<Repo> {
        let mut repo = Repo::default();
        let mods = root.join("mods");
        if !mods.is_dir() {
            return Ok(repo);
        }
        for letter in sorted_dir(&mods)? {
            let name = letter.file_name().to_string_lossy().into_owned();
            let rel = format!("mods/{name}");
            if !letter.file_type()?.is_dir() {
                repo.issues.push(Issue::new(rel, "only folders named after a letter belong in mods/"));
                continue;
            }
            for file in sorted_dir(&letter.path())? {
                let fname = file.file_name().to_string_lossy().into_owned();
                let rel = format!("{rel}/{fname}");
                let id = fname.strip_suffix(".toml").unwrap_or_default();
                if !file.file_type()?.is_file() || id.is_empty() {
                    repo.issues.push(Issue::new(rel, "expected mods/<first letter>/<id>.toml"));
                    continue;
                }
                repo.add(&rel, &name, id, &fs::read(file.path())?);
            }
        }
        Ok(repo)
    }

    fn add(&mut self, rel: &str, letter: &str, id: &str, bytes: &[u8]) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            self.issues.push(Issue::new(rel, "not valid UTF-8"));
            return;
        };
        match Entry::parse(rel, text) {
            Err(issues) => self.issues.extend(issues),
            Ok((json, entry)) => {
                if entry.id != id {
                    self.issues.push(Issue::new(rel, format!("file name says `{id}` but id is `{}`", entry.id)));
                } else if !id.starts_with(letter) || letter.chars().count() != 1 {
                    self.issues.push(Issue::new(rel, format!("`{id}` belongs in mods/{}/", &id[..1])));
                } else {
                    self.entries.insert(entry.id.clone(), Loaded { path: rel.to_string(), json, entry });
                }
            }
        }
    }
}

fn sorted_dir(p: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut v = fs::read_dir(p)?.collect::<io::Result<Vec<_>>>()?;
    v.sort_by_key(|e| e.file_name());
    Ok(v)
}
