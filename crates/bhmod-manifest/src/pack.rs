//! Deterministic `.bhmod` writer.

use std::io::{self, Seek, Write};

use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime};

use crate::package::{FileEntry, Source};

/// Writes the files of `src` into a zip in path order with fixed timestamps (1980-01-01) and permissions (0644),
/// so the same input always gives the same bytes.
pub fn write_zip<W: Write + Seek>(src: &mut dyn Source, out: W) -> io::Result<W> {
    let mut files: Vec<FileEntry> = src.files().to_vec();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let mut zip = zip::ZipWriter::new(out);
    for f in files {
        let data = src.read(&f.path, f.size)?;
        zip.start_file(f.path.as_str(), opts).map_err(io::Error::other)?;
        zip.write_all(&data)?;
    }
    zip.finish().map_err(io::Error::other)
}

/// Lowercase hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}
