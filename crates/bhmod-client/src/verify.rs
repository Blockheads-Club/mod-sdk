//! Checks a package before a client loads it: the package rules of `bhmod-manifest`, a client half, a runtime this
//! loader has, the `api` and `loader` requirements, and never native code (milestone M10 adds index-listed native
//! mods behind the red warning).

use std::fmt;
use std::io::Cursor;

use bhmod_manifest::manifest::Runtime;
use bhmod_manifest::package::{check, CheckOptions, Source, ZipSource};
use bhmod_manifest::Manifest;
use semver::{Version, VersionReq};

/// What this loader can run.
#[derive(Debug, Clone)]
pub struct Caps {
    /// The loader version (checked against `mod.toml` `loader`).
    pub loader: Version,
    /// Runtimes compiled into the loader.
    pub runtimes: Vec<Runtime>,
}

/// Why a package is not loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not a readable zip.
    Unreadable(String),
    /// The package rules failed.
    Invalid(Vec<String>),
    /// The package is another mod or version than the server listed.
    WrongMod {
        /// `id@version` listed.
        expected: String,
        /// `id@version` in the package.
        found: String,
    },
    /// No client half.
    NoClientHalf,
    /// Asks for native access.
    Native,
    /// Native code that was not installed from the public index with the player's confirmation.
    NativeUnconfirmed,
    /// Needs a newer `blockheads:api` (the requirement).
    NeedsNewerApi(String),
    /// Needs another loader version (the requirement).
    NeedsNewerLoader(String),
    /// Uses a runtime this loader lacks (`wasm`, `lua`, `jvm`).
    MissingRuntime(String),
}

impl Refusal {
    /// A stable code for translated UI texts.
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::Unreadable(_) => "unreadable",
            Refusal::Invalid(_) => "invalid",
            Refusal::WrongMod { .. } => "wrong-mod",
            Refusal::NoClientHalf => "no-client-half",
            Refusal::Native => "native",
            Refusal::NativeUnconfirmed => "native-unconfirmed",
            Refusal::NeedsNewerApi(_) => "needs-newer-api",
            Refusal::NeedsNewerLoader(_) => "needs-newer-loader",
            Refusal::MissingRuntime(_) => "missing-runtime",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Unreadable(e) => write!(f, "the file is not a mod package ({e})"),
            Refusal::Invalid(v) => write!(f, "the file check failed: {}", v.join("; ")),
            Refusal::WrongMod { expected, found } => write!(f, "expected {expected}, the file is {found}"),
            Refusal::NoClientHalf => write!(f, "the mod has no client part"),
            Refusal::Native => write!(f, "native mods are not loaded from servers or links"),
            Refusal::NativeUnconfirmed => {
                write!(f, "native mods come from the public mod list and need the player's confirmation")
            }
            Refusal::NeedsNewerApi(r) => write!(f, "the mod needs API {r}; update the mod loader"),
            Refusal::NeedsNewerLoader(r) => write!(f, "the mod needs mod loader {r}; update the mod loader"),
            Refusal::MissingRuntime(r) => write!(f, "this mod loader cannot run {r} mods"),
        }
    }
}

fn runtime_name(r: Runtime) -> &'static str {
    match r {
        Runtime::Wasm => "wasm",
        Runtime::Lua => "lua",
        Runtime::Jvm => "jvm",
    }
}

/// The `api` value of the package's `mod.toml` (best effort, for the message).
fn api_of(src: &mut dyn Source) -> String {
    let text = src.read("mod.toml", 1 << 20).unwrap_or_default();
    let v: toml::Table = String::from_utf8_lossy(&text).parse().unwrap_or_default();
    v.get("api").and_then(|a| a.as_str()).unwrap_or("?").to_string()
}

/// Checks a client package. Native code is refused.
pub fn check_package(bytes: &[u8], caps: &Caps) -> Result<Manifest, Refusal> {
    check_package_with(bytes, caps, false)
}

/// Like [`check_package`]; `allow_native` lets a package with native code through (the host did the extra checks).
pub fn check_package_with(bytes: &[u8], caps: &Caps, allow_native: bool) -> Result<Manifest, Refusal> {
    let mut src =
        ZipSource::open(Cursor::new(bytes), bytes.len() as u64).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    let report = check(&mut src, CheckOptions::default());
    let m = match report.manifest {
        Some(m) if report.errors.is_empty() => m,
        _ => {
            // The SDK validates `api` already; a requirement it cannot serve means the loader is too old.
            if report.errors.iter().any(|i| i.path == "api" && i.message.starts_with("matches none")) {
                return Err(Refusal::NeedsNewerApi(api_of(&mut src)));
            }
            return Err(Refusal::Invalid(report.errors.iter().map(|i| i.to_string()).collect()));
        }
    };
    if m.permissions.native && !allow_native {
        return Err(Refusal::Native);
    }
    let entry = m.entry.client.as_ref().ok_or(Refusal::NoClientHalf)?;
    if !caps.runtimes.contains(&entry.runtime) {
        return Err(Refusal::MissingRuntime(runtime_name(entry.runtime).into()));
    }
    if let Some(l) = &m.loader {
        if !VersionReq::parse(l).is_ok_and(|r| r.matches(&caps.loader)) {
            return Err(Refusal::NeedsNewerLoader(l.clone()));
        }
    }
    Ok(m)
}

/// Checks a package the server listed as `id@version`.
pub fn check_offered(bytes: &[u8], id: &str, version: &str, caps: &Caps) -> Result<Manifest, Refusal> {
    check_offered_with(bytes, id, version, caps, false)
}

/// Like [`check_offered`] with the `allow_native` switch of [`check_package_with`].
pub fn check_offered_with(
    bytes: &[u8],
    id: &str,
    version: &str,
    caps: &Caps,
    allow_native: bool,
) -> Result<Manifest, Refusal> {
    let m = check_package_with(bytes, caps, allow_native)?;
    if m.id != id || m.version != version {
        return Err(Refusal::WrongMod {
            expected: format!("{id}@{version}"),
            found: format!("{}@{}", m.id, m.version),
        });
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{caps, package};

    #[test]
    fn accepts_a_lua_client_mod() {
        let p = package("demo", "1.0.0", "", "");
        let m = check_offered(&p, "demo", "1.0.0", &caps()).unwrap();
        assert_eq!(m.id, "demo");
    }

    #[test]
    fn refusals() {
        let c = caps();
        let r = |e: Refusal| (e.code(), e.to_string());
        assert_eq!(check_package(b"nope", &c).unwrap_err().code(), "unreadable");
        let e = check_offered(&package("demo", "1.0.0", "", ""), "demo", "2.0.0", &c).unwrap_err();
        assert_eq!(r(e).0, "wrong-mod");
        let e = check_package(&package("demo", "1.0.0", "native = true\n", ""), &c).unwrap_err();
        assert_eq!(e, Refusal::Native, "{e}");
        let e = check_package(&package("demo", "1.0.0", "", "server-only"), &c).unwrap_err();
        assert_eq!(e, Refusal::NoClientHalf);
        let e = check_package(&package("demo", "1.0.0", "", "api9"), &c).unwrap_err();
        assert_eq!(e.code(), "needs-newer-api");
        let e = check_package(&package("demo", "1.0.0", "", "loader9"), &c).unwrap_err();
        assert_eq!(e.code(), "needs-newer-loader");
        assert!(check_package(&package("demo", "1.0.0", "", "loader0"), &c).is_ok());
        let e = check_package(&package("demo", "1.0.0", "", "wasm"), &c).unwrap_err();
        assert_eq!(e, Refusal::MissingRuntime("wasm".into()));
        let e = check_package(&package("demo", "1.0.0", "", "invalid"), &c).unwrap_err();
        assert_eq!(e.code(), "invalid");
        for e in [
            Refusal::Unreadable("x".into()),
            Refusal::Invalid(vec!["a".into()]),
            Refusal::WrongMod { expected: "a".into(), found: "b".into() },
            Refusal::NoClientHalf,
            Refusal::Native,
            Refusal::NativeUnconfirmed,
            Refusal::NeedsNewerApi("^9".into()),
            Refusal::NeedsNewerLoader("^9".into()),
            Refusal::MissingRuntime("jvm".into()),
        ] {
            assert!(!e.to_string().is_empty());
            assert!(!e.code().is_empty());
        }
        let nat = package("demo", "1.0.0", "native = true\n", "");
        assert!(check_package_with(&nat, &c, true).unwrap().permissions.native);
        assert!(check_offered_with(&nat, "demo", "1.0.0", &c, true).is_ok());
        assert_eq!(runtime_name(Runtime::Jvm), "jvm");
        assert_eq!(runtime_name(Runtime::Lua), "lua");
    }
}
