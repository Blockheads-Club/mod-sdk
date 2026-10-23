//! The `bhmod` command line tool: check, pack and create Blockheads mods.
#![warn(missing_docs)]

use std::ffi::OsString;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};

use bhmod_manifest::pack::{sha256_hex, write_zip};
use bhmod_manifest::package::{check, CheckOptions, DirSource, Report, ZipSource};
use bhmod_manifest::rules::is_mod_id;
use clap::{Parser, Subcommand, ValueEnum};

mod index;
mod templates;

#[derive(Parser)]
#[command(name = "bhmod", version, about = "Check, pack and create Blockheads mods")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a mod source directory or a .bhmod package.
    Check {
        /// Directory or .bhmod file.
        path: PathBuf,
        /// Also apply the public index rules (LICENSE required).
        #[arg(long)]
        index: bool,
    },
    /// Build <id>-<version>.bhmod from a mod directory.
    Pack {
        /// Mod source directory.
        dir: PathBuf,
        /// Output file (default: ./<id>-<version>.bhmod).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Create a new mod from a template.
    New {
        /// Mod id, [a-z0-9_-]{2,40}.
        id: String,
        /// Language of the template.
        #[arg(long, value_enum, default_value = "lua")]
        lang: Lang,
        /// Parent directory (default: current directory).
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Validate and build the public mod index (Blockheads-Club/mods).
    Index(index::IndexArgs),
}

/// Template languages.
#[derive(Clone, Copy, ValueEnum)]
enum Lang {
    Lua,
    Rust,
    Java,
}

/// Runs the tool; returns the process exit code (0 ok, 1 failure, 2 usage error).
pub fn run(args: impl IntoIterator<Item = OsString>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = e.exit_code();
            let text = e.render().to_string();
            let _ = if code == 0 { out.write_all(text.as_bytes()) } else { err.write_all(text.as_bytes()) };
            return code;
        }
    };
    let result = match cli.command {
        Command::Check { path, index } => cmd_check(&path, index, out),
        Command::Pack { dir, output } => cmd_pack(&dir, output, out),
        Command::New { id, lang, dir } => cmd_new(&id, lang, &dir, out),
        Command::Index(args) => index::run(args.command, out),
    };
    match result {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            1
        }
    }
}

fn print_report(r: &Report, out: &mut dyn Write) -> io::Result<bool> {
    for w in &r.warnings {
        writeln!(out, "warning: {w}")?;
    }
    for e in &r.errors {
        writeln!(out, "error: {e}")?;
    }
    match (&r.manifest, r.is_ok()) {
        (Some(m), true) => {
            writeln!(out, "ok: {} {} ({} files, {} bytes)", m.id, m.version, r.file_count, r.total_size)?;
            Ok(true)
        }
        _ => {
            writeln!(out, "failed: {} error(s)", r.errors.len())?;
            Ok(false)
        }
    }
}

fn cmd_check(path: &Path, index: bool, out: &mut dyn Write) -> io::Result<bool> {
    let opts = CheckOptions { index };
    let report = if path.is_file() {
        let bytes = fs::read(path)?;
        let len = bytes.len() as u64;
        check(&mut ZipSource::open(Cursor::new(bytes), len)?, opts)
    } else {
        let mut src = DirSource::open(path)?;
        if !src.ignored.is_empty() {
            writeln!(out, "note: not packed: {}", src.ignored.join(", "))?;
        }
        check(&mut src, opts)
    };
    print_report(&report, out)
}

fn cmd_pack(dir: &Path, output: Option<PathBuf>, out: &mut dyn Write) -> io::Result<bool> {
    let mut src = DirSource::open(dir)?;
    let report = check(&mut src, CheckOptions::default());
    let Some(m) = report.manifest.as_ref().filter(|_| report.is_ok()) else {
        return print_report(&report, out);
    };
    let bytes = write_zip(&mut src, Cursor::new(Vec::new()))?.into_inner();
    let len = bytes.len() as u64;
    let zipped = check(&mut ZipSource::open(Cursor::new(bytes.clone()), len)?, CheckOptions::default());
    if !zipped.is_ok() {
        return print_report(&zipped, out);
    }
    let path = output.unwrap_or_else(|| PathBuf::from(format!("{}-{}.bhmod", m.id, m.version)));
    fs::write(&path, &bytes)?;
    for w in &report.warnings {
        writeln!(out, "warning: {w}")?;
    }
    writeln!(out, "packed {} ({} bytes, {} files)", path.display(), bytes.len(), zipped.file_count)?;
    writeln!(out, "sha256 {}", sha256_hex(&bytes))?;
    Ok(true)
}

fn cmd_new(id: &str, lang: Lang, parent: &Path, out: &mut dyn Write) -> io::Result<bool> {
    if !is_mod_id(id) {
        writeln!(out, "error: `{id}` is not a valid mod id ([a-z0-9_-]{{2,40}})")?;
        return Ok(false);
    }
    let root = parent.join(id);
    if root.exists() {
        writeln!(out, "error: {} already exists", root.display())?;
        return Ok(false);
    }
    let files = match lang {
        Lang::Lua => templates::lua(id),
        Lang::Rust => templates::rust(id),
        Lang::Java => templates::java(id),
    };
    for (path, content) in &files {
        let p = root.join(path);
        fs::create_dir_all(p.parent().expect("template paths have a parent"))?;
        fs::write(&p, content)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if root.join("build.sh").exists() {
            fs::set_permissions(root.join("build.sh"), fs::Permissions::from_mode(0o755))?;
        }
    }
    writeln!(out, "created {} ({} files)", root.display(), files.len())?;
    Ok(true)
}
