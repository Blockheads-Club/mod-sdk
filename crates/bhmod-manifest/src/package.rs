//! Package rules (research §5.1) applied to a source directory or a `.bhmod` zip.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};

use zip::CompressionMethod;

use crate::manifest::{Manifest, Runtime};
use crate::rules::{check_location, check_path, is_native, ALLOWED_DIRS, ALLOWED_ROOT_FILES};
use crate::Issue;

/// Largest `.bhmod` file (compressed): 32 MiB.
pub const MAX_COMPRESSED: u64 = 32 * 1024 * 1024;
/// Largest total uncompressed size: 128 MiB.
pub const MAX_UNCOMPRESSED: u64 = 128 * 1024 * 1024;
/// Most files in one package.
pub const MAX_FILES: usize = 4096;
/// Largest `mod.toml`: 64 KiB.
pub const MAX_MANIFEST: u64 = 64 * 1024;
/// Largest texture edge in pixels.
pub const MAX_TEXTURE_EDGE: u32 = 1024;

/// One file of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// `/`-separated path inside the package.
    pub path: String,
    /// Uncompressed size in bytes.
    pub size: u64,
}

/// A package being checked: either a source directory or a zip.
pub trait Source {
    /// Files in the package (no directories).
    fn files(&self) -> &[FileEntry];
    /// Problems found while listing (symlinks, bad zip entries, ...).
    fn listing_issues(&self) -> &[Issue];
    /// Reads one file, at most `limit` bytes.
    fn read(&mut self, path: &str, limit: u64) -> io::Result<Vec<u8>>;
    /// Compressed size when known (zip only).
    fn compressed_size(&self) -> Option<u64>;
}

/// A mod source directory. Only the package layout (root files and the allowed directories) is considered; other
/// top-level entries (`src/`, `Cargo.toml`, `target/`, ...) and hidden files are ignored.
#[derive(Debug)]
pub struct DirSource {
    root: PathBuf,
    files: Vec<FileEntry>,
    issues: Vec<Issue>,
    /// Top-level names that are not part of the package.
    pub ignored: Vec<String>,
}

impl DirSource {
    /// Lists a directory.
    pub fn open(root: &Path) -> io::Result<DirSource> {
        let mut s = DirSource { root: root.to_path_buf(), files: Vec::new(), issues: Vec::new(), ignored: Vec::new() };
        let mut top: Vec<_> = fs::read_dir(root)?.collect::<io::Result<_>>()?;
        top.sort_by_key(|e| e.file_name());
        for e in top {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = fs::symlink_metadata(e.path())?;
            let wanted = if meta.is_dir() {
                ALLOWED_DIRS.contains(&name.as_str())
            } else {
                ALLOWED_ROOT_FILES.contains(&name.as_str())
            };
            if wanted {
                s.walk(&e.path(), name)?;
            } else {
                s.ignored.push(name);
            }
        }
        Ok(s)
    }

    fn walk(&mut self, path: &Path, rel: String) -> io::Result<()> {
        let meta = fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            self.issues.push(Issue::new(rel, "symlinks are not allowed"));
        } else if meta.is_dir() {
            let mut children: Vec<_> = fs::read_dir(path)?.collect::<io::Result<_>>()?;
            children.sort_by_key(|e| e.file_name());
            for c in children {
                let name = c.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    self.ignored.push(format!("{rel}/{name}"));
                } else {
                    self.walk(&c.path(), format!("{rel}/{name}"))?;
                }
            }
        } else {
            self.files.push(FileEntry { path: rel, size: meta.len() });
        }
        Ok(())
    }
}

impl Source for DirSource {
    fn files(&self) -> &[FileEntry] {
        &self.files
    }
    fn listing_issues(&self) -> &[Issue] {
        &self.issues
    }
    fn read(&mut self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        fs::File::open(self.root.join(path))?.take(limit).read_to_end(&mut buf)?;
        Ok(buf)
    }
    fn compressed_size(&self) -> Option<u64> {
        None
    }
}

/// A `.bhmod` zip.
pub struct ZipSource<R: Read + Seek> {
    archive: zip::ZipArchive<R>,
    files: Vec<FileEntry>,
    issues: Vec<Issue>,
    compressed: u64,
}

impl<R: Read + Seek> ZipSource<R> {
    /// Opens a zip. `compressed` is the size of the whole zip file.
    pub fn open(reader: R, compressed: u64) -> io::Result<ZipSource<R>> {
        let mut archive = zip::ZipArchive::new(reader).map_err(io::Error::other)?;
        let mut files = Vec::new();
        let mut issues = Vec::new();
        for i in 0..archive.len() {
            let f = archive.by_index_raw(i).map_err(io::Error::other)?;
            let name = f.name().to_string();
            if f.is_symlink() {
                issues.push(Issue::new(&name, "symlinks are not allowed"));
            } else if f.encrypted() {
                issues.push(Issue::new(&name, "encrypted entries are not allowed"));
            } else if !matches!(f.compression(), CompressionMethod::Stored | CompressionMethod::Deflated) {
                issues.push(Issue::new(&name, "only stored and deflate compression are allowed"));
            } else if f.is_dir() {
                let dir = name.trim_end_matches('/');
                if let Err(msg) = check_path(dir) {
                    issues.push(Issue::new(&name, msg));
                }
            } else {
                files.push(FileEntry { path: name, size: f.size() });
            }
        }
        Ok(ZipSource { archive, files, issues, compressed })
    }
}

impl<R: Read + Seek> Source for ZipSource<R> {
    fn files(&self) -> &[FileEntry] {
        &self.files
    }
    fn listing_issues(&self) -> &[Issue] {
        &self.issues
    }
    fn read(&mut self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
        let f = self.archive.by_name(path).map_err(io::Error::other)?;
        let mut buf = Vec::new();
        f.take(limit).read_to_end(&mut buf)?;
        Ok(buf)
    }
    fn compressed_size(&self) -> Option<u64> {
        Some(self.compressed)
    }
}

/// Options for [`check`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckOptions {
    /// Apply the public index rules too (a `LICENSE` file is required).
    pub index: bool,
}

/// The result of checking a package.
#[derive(Debug, Default)]
pub struct Report {
    /// The manifest, if it parsed and validated.
    pub manifest: Option<Manifest>,
    /// Problems that make the package invalid.
    pub errors: Vec<Issue>,
    /// Problems that do not.
    pub warnings: Vec<Issue>,
    /// Number of files.
    pub file_count: usize,
    /// Total uncompressed size.
    pub total_size: u64,
}

impl Report {
    /// True when there are no errors.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Applies the package rules and validates `mod.toml` and the entry points.
pub fn check(src: &mut dyn Source, opts: CheckOptions) -> Report {
    let mut r = Report { errors: src.listing_issues().to_vec(), ..Report::default() };
    let files: Vec<FileEntry> = src.files().to_vec();
    r.file_count = files.len();
    r.total_size = files.iter().map(|f| f.size).sum();
    let mut seen = BTreeSet::new();
    for f in &files {
        if let Err(msg) = check_path(&f.path).and_then(|()| check_location(&f.path)) {
            r.errors.push(Issue::new(&f.path, msg));
        }
        if !seen.insert(f.path.to_ascii_lowercase()) {
            r.errors.push(Issue::new(&f.path, "duplicate name (names are compared case-insensitively)"));
        }
    }
    if r.file_count > MAX_FILES {
        r.errors.push(Issue::new("package", format!("{} files, at most {MAX_FILES} allowed", r.file_count)));
    }
    if r.total_size > MAX_UNCOMPRESSED {
        r.errors.push(Issue::new(
            "package",
            format!("{} bytes uncompressed, at most {MAX_UNCOMPRESSED} allowed", r.total_size),
        ));
    }
    if let Some(c) = src.compressed_size().filter(|c| *c > MAX_COMPRESSED) {
        r.errors.push(Issue::new("package", format!("{c} bytes compressed, at most {MAX_COMPRESSED} allowed")));
    }
    let sizes: BTreeMap<&str, u64> = files.iter().map(|f| (f.path.as_str(), f.size)).collect();
    if !sizes.keys().any(|p| p.starts_with("LICENSE")) {
        let issue = Issue::new("LICENSE", "no LICENSE file (required for the public index)");
        if opts.index {
            r.errors.push(issue)
        } else {
            r.warnings.push(issue)
        }
    }
    let Some(&manifest_size) = sizes.get("mod.toml") else {
        r.errors.push(Issue::new("mod.toml", "missing"));
        return r;
    };
    if manifest_size > MAX_MANIFEST {
        r.errors.push(Issue::new("mod.toml", format!("larger than {MAX_MANIFEST} bytes")));
        return r;
    }
    let manifest = match read_text(src, "mod.toml", MAX_MANIFEST).map(|t| Manifest::parse(&t)) {
        Err(issue) => {
            r.errors.push(issue);
            return r;
        }
        Ok(Err(issues)) => {
            r.errors.extend(issues);
            return r;
        }
        Ok(Ok(m)) => m,
    };
    check_against_manifest(src, &manifest, &sizes, &mut r);
    r.manifest = Some(manifest);
    r
}

fn check_against_manifest(src: &mut dyn Source, m: &Manifest, sizes: &BTreeMap<&str, u64>, r: &mut Report) {
    for (side, entry) in [("server", &m.entry.server), ("client", &m.entry.client)] {
        let Some(e) = entry else { continue };
        let key = format!("entry.{side}");
        if !sizes.contains_key(e.path.as_str()) {
            r.errors.push(Issue::new(&key, format!("entry point {} is not in the package", e.path)));
            continue;
        }
        let head = src.read(&e.path, 8).unwrap_or_default();
        let ok = match e.runtime {
            Runtime::Wasm => head.starts_with(b"\0asm") && (head[4..] == [1, 0, 0, 0] || head[4..] == [0x0d, 0, 1, 0]),
            Runtime::Lua => read_text(src, &e.path, MAX_UNCOMPRESSED).is_ok_and(|t| !t.trim().is_empty()),
            Runtime::Jvm => head.starts_with(b"PK\x03\x04"),
        };
        if !ok {
            r.errors.push(Issue::new(&key, format!("{} is not a valid {:?} entry point", e.path, e.runtime)));
        }
    }
    for side in ["server", "client"] {
        let enabled = if side == "server" { m.sides.server } else { m.sides.client };
        if !enabled && sizes.keys().any(|p| p.starts_with(&format!("{side}/"))) {
            r.warnings.push(Issue::new(side, format!("files in {side}/ are unused because sides.{side} is false")));
        }
    }
    if !m.permissions.native {
        for p in sizes.keys().filter(|p| is_native(p)) {
            r.errors.push(Issue::new(*p, "native libraries need permissions.native = true"));
        }
    }
    // The game is 32-bit ARM only: libraries for other ABIs are never loaded.
    let natives: Vec<&&str> = sizes.keys().filter(|p| is_native(p)).collect();
    if m.permissions.native
        && !natives.is_empty()
        && !natives.iter().any(|p| p.starts_with("client/native/armeabi-v7a/"))
    {
        r.warnings.push(Issue::new(
            "client/native",
            "the game runs armeabi-v7a code only: without client/native/armeabi-v7a/*.so no library of this mod loads",
        ));
    }
    for s in &m.provides.services {
        if !sizes.contains_key(s.wit.as_str()) {
            r.errors.push(Issue::new("provides", format!("{} is not in the package", s.wit)));
        }
    }
    for (key, texture) in m.content.textures() {
        if !sizes.contains_key(texture) {
            r.errors.push(Issue::new("content", format!("texture {texture} of `{key}` is not in the package")));
        }
    }
    for p in sizes.keys().filter(|p| p.starts_with("assets/") && p.ends_with(".png")) {
        let head = src.read(p, 24).unwrap_or_default();
        if let Err(msg) = check_png(&head) {
            r.errors.push(Issue::new(*p, msg));
        }
    }
}

fn read_text(src: &mut dyn Source, path: &str, limit: u64) -> Result<String, Issue> {
    let bytes = src.read(path, limit).map_err(|e| Issue::new(path, format!("cannot read: {e}")))?;
    String::from_utf8(bytes).map_err(|_| Issue::new(path, "not valid UTF-8"))
}

/// Checks a PNG header (first 24 bytes): signature, IHDR, and edge lengths 1..=1024.
pub fn check_png(head: &[u8]) -> Result<(u32, u32), String> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if head.len() < 24 || &head[..8] != SIG || &head[12..16] != b"IHDR" {
        return Err("not a PNG file".into());
    }
    let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    if w == 0 || h == 0 || w > MAX_TEXTURE_EDGE || h > MAX_TEXTURE_EDGE {
        return Err(format!("texture is {w}x{h}, edges must be 1-{MAX_TEXTURE_EDGE} px"));
    }
    Ok((w, h))
}
