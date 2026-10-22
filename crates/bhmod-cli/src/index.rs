//! `bhmod index`: validate, build and sign the mod index (Blockheads-Club/mods).

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bhmod_index::build::{build, utc_now, BuildOptions};
use bhmod_index::repo::Repo;
use bhmod_index::sign::{keygen, sign, verify};
use bhmod_index::validate::{downloads, validate, Options};
use bhmod_manifest::API_VERSION;
use clap::{Args, Subcommand};
use semver::Version;

/// `bhmod index <command>`.
#[derive(Args)]
pub(crate) struct IndexArgs {
    #[command(subcommand)]
    pub(crate) command: IndexCommand,
}

#[derive(Subcommand)]
pub(crate) enum IndexCommand {
    /// Check the index against the pull request rules.
    Validate(ValidateArgs),
    /// Print `<url> <file>` for every package `validate` needs (new versions only).
    Downloads {
        /// Index repo checkout.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Checkout of the base branch.
        #[arg(long)]
        base: Option<PathBuf>,
    },
    /// Build index.json from every entry.
    Build(BuildArgs),
    /// Sign a file with a minisign secret key; writes <file>.minisig.
    Sign {
        /// File to sign.
        file: PathBuf,
        /// Secret key file.
        #[arg(long, conflicts_with = "key_env", required_unless_present = "key_env")]
        key: Option<PathBuf>,
        /// Environment variable holding the secret key text.
        #[arg(long)]
        key_env: Option<String>,
        /// Environment variable holding the key password (encrypted keys only).
        #[arg(long)]
        password_env: Option<String>,
    },
    /// Verify <file>.minisig with a public key.
    Verify {
        /// Signed file.
        file: PathBuf,
        /// Public key file (minisign .pub).
        #[arg(long)]
        pubkey: PathBuf,
    },
    /// Create a new signing key pair (index.pub, index.key) in a folder.
    Keygen {
        /// Output folder.
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
}

#[derive(Args)]
pub(crate) struct ValidateArgs {
    /// Index repo checkout (the pull request head).
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Checkout of the base branch; without it every entry counts as new.
    #[arg(long)]
    base: Option<PathBuf>,
    /// GitHub login of the pull request author (owner check).
    #[arg(long)]
    author: Option<String>,
    /// Folder with the packages as <id>-<version>.bhmod (CI downloads them; local test mode).
    #[arg(long)]
    packages: Option<PathBuf>,
    /// luau-compile binary for the Luau syntax check.
    #[arg(long)]
    luau_compile: Option<PathBuf>,
    /// Supported blockheads:api versions (repeatable).
    #[arg(long = "api", default_value = API_VERSION)]
    api: Vec<Version>,
    /// Allow changes to more than one mod.
    #[arg(long)]
    allow_multiple: bool,
    /// Write the PR comment (Markdown) here.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Write the labels (one per line) here.
    #[arg(long)]
    labels: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct BuildArgs {
    /// Index repo checkout.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Output file.
    #[arg(short, long, default_value = "index.json")]
    output: PathBuf,
    /// Keep draft entries (local testing only).
    #[arg(long)]
    include_drafts: bool,
    /// Release download base for mirrored packages.
    #[arg(long)]
    mirror_base: Option<String>,
    /// Build time (default: now, UTC).
    #[arg(long)]
    generated: Option<String>,
    /// blockheads:api versions to record (repeatable).
    #[arg(long = "api", default_value = API_VERSION)]
    api: Vec<String>,
}

pub(crate) fn run(cmd: IndexCommand, out: &mut dyn Write) -> io::Result<bool> {
    match cmd {
        IndexCommand::Validate(a) => cmd_validate(a, out),
        IndexCommand::Downloads { repo, base } => {
            let head = Repo::load(&repo)?;
            let base = base.as_deref().map(Repo::load).transpose()?;
            for (url, file) in downloads(&head, base.as_ref()) {
                writeln!(out, "{url} {file}")?;
            }
            Ok(true)
        }
        IndexCommand::Build(a) => cmd_build(a, out),
        IndexCommand::Sign { file, key, key_env, password_env } => {
            // clap makes sure one of --key and --key-env is given.
            let secret = match key {
                Some(k) => fs::read_to_string(k)?,
                None => env_var(&key_env.unwrap_or_default())?,
            };
            let password = password_env.as_deref().map(env_var).transpose()?;
            let data = fs::read(&file)?;
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let comment = format!("file:{name} signed:{}", utc_now());
            let sig = sign(&data, &secret, password, &comment).map_err(io::Error::other)?;
            let sig_path = minisig_path(&file);
            fs::write(&sig_path, sig)?;
            writeln!(out, "signed {} -> {}", file.display(), sig_path.display())?;
            Ok(true)
        }
        IndexCommand::Verify { file, pubkey } => {
            let data = fs::read(&file)?;
            let sig = fs::read_to_string(minisig_path(&file))?;
            match verify(&data, &fs::read_to_string(pubkey)?, &sig) {
                Ok(()) => writeln!(out, "ok: signature of {} is valid", file.display())?,
                Err(e) => {
                    writeln!(out, "error: {e}")?;
                    return Ok(false);
                }
            }
            Ok(true)
        }
        IndexCommand::Keygen { dir } => {
            let (pk, sk) = (dir.join("index.pub"), dir.join("index.key"));
            if pk.exists() || sk.exists() {
                writeln!(out, "error: {} or {} already exists", pk.display(), sk.display())?;
                return Ok(false);
            }
            let (pub_text, key_text) = keygen();
            fs::create_dir_all(&dir)?;
            fs::write(&pk, pub_text)?;
            fs::write(&sk, key_text)?;
            writeln!(out, "created {} and {} (keep the .key secret)", pk.display(), sk.display())?;
            Ok(true)
        }
    }
}

fn env_var(name: &str) -> io::Result<String> {
    env::var(name).map_err(|_| io::Error::other(format!("environment variable {name} is not set")))
}

fn minisig_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_owned();
    s.push(".minisig");
    PathBuf::from(s)
}

fn cmd_validate(a: ValidateArgs, out: &mut dyn Write) -> io::Result<bool> {
    let head = Repo::load(&a.repo)?;
    let opts = Options {
        base: a.base.as_deref().map(Repo::load).transpose()?,
        author: a.author,
        packages: a.packages,
        luau_compile: a.luau_compile,
        api_versions: a.api,
        allow_multiple: a.allow_multiple,
    };
    let outcome = validate(&head, &opts)?;
    for w in &outcome.warnings {
        writeln!(out, "warning: {w}")?;
    }
    for e in &outcome.errors {
        writeln!(out, "error: {e}")?;
    }
    for l in &outcome.labels {
        writeln!(out, "label: {l}")?;
    }
    if let Some(p) = &a.report {
        fs::write(p, outcome.markdown())?;
    }
    if let Some(p) = &a.labels {
        let text: String = outcome.labels.iter().map(|l| format!("{l}\n")).collect();
        fs::write(p, text)?;
    }
    let mods: Vec<&str> = outcome.changes.iter().map(|c| c.id.as_str()).collect();
    if outcome.is_ok() {
        let list = if mods.is_empty() { String::new() } else { format!(": {}", mods.join(", ")) };
        writeln!(out, "ok: {} changed mod(s){list}", mods.len())?;
    } else {
        writeln!(out, "failed: {} error(s)", outcome.errors.len())?;
    }
    Ok(outcome.is_ok())
}

fn cmd_build(a: BuildArgs, out: &mut dyn Write) -> io::Result<bool> {
    let repo = Repo::load(&a.repo)?;
    let opts = BuildOptions {
        include_drafts: a.include_drafts,
        mirror_base: a.mirror_base,
        generated: a.generated.unwrap_or_else(utc_now),
        api_versions: a.api,
    };
    match build(&repo, &opts) {
        Ok(text) => {
            fs::write(&a.output, &text)?;
            let n = serde_json::from_str::<serde_json::Value>(&text)
                .map(|v| v["mods"].as_array().map_or(0, Vec::len))
                .unwrap_or(0);
            writeln!(out, "built {} ({n} mods, {} bytes)", a.output.display(), text.len())?;
            Ok(true)
        }
        Err(errors) => {
            for e in &errors {
                writeln!(out, "error: {e}")?;
            }
            writeln!(out, "failed: {} error(s)", errors.len())?;
            Ok(false)
        }
    }
}
