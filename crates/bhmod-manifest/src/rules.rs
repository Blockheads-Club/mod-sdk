//! Small validation helpers shared by the manifest and package checks.

/// Top-level directories a package may contain.
pub const ALLOWED_DIRS: [&str; 5] = ["server", "client", "assets", "data", "wit"];

/// Files a package may contain at its root.
pub const ALLOWED_ROOT_FILES: [&str; 6] =
    ["mod.toml", "LICENSE", "LICENSE.md", "LICENSE.txt", "README.md", "CHANGELOG.md"];

/// ABIs accepted under `client/native/<abi>/`.
pub const NATIVE_ABIS: [&str; 4] = ["arm64-v8a", "armeabi-v7a", "x86_64", "x86"];

/// Longest allowed package path in bytes.
pub const MAX_PATH_LEN: usize = 240;

/// True for `[a-z0-9_-]` strings whose length is within `min..=max`.
pub fn is_slug(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// A mod id: `[a-z0-9_-]{2,40}`.
pub fn is_mod_id(s: &str) -> bool {
    is_slug(s, 2, 40)
}

/// A language code such as `en`, `tr`, `pt-BR`, `zh-Hans`.
pub fn is_lang(s: &str) -> bool {
    let (base, region) = match s.split_once('-') {
        Some((b, r)) => (b, Some(r)),
        None => (s, None),
    };
    let base_ok = (2..=3).contains(&base.len()) && base.bytes().all(|b| b.is_ascii_lowercase());
    let region_ok = region.is_none_or(|r| (2..=8).contains(&r.len()) && r.bytes().all(|b| b.is_ascii_alphanumeric()));
    base_ok && region_ok
}

/// A host name for the `net-http` allowlist: lowercase DNS labels, optionally starting with `*.`.
pub fn is_host(s: &str) -> bool {
    let rest = s.strip_prefix("*.").unwrap_or(s);
    !rest.is_empty()
        && rest.len() <= 253
        && rest.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Checks the shape of a path inside a package (`/` separated, relative, no `.`/`..`, no hidden components).
pub fn check_path(p: &str) -> Result<(), String> {
    if p.is_empty() {
        return Err("empty path".into());
    }
    if p.len() > MAX_PATH_LEN {
        return Err(format!("path longer than {MAX_PATH_LEN} bytes"));
    }
    if p.starts_with('/') || p.contains('\\') || p.contains(':') {
        return Err("absolute path, `\\` or `:` not allowed".into());
    }
    if p.chars().any(char::is_control) {
        return Err("control character in path".into());
    }
    for comp in p.split('/') {
        if comp.is_empty() {
            return Err("empty path component".into());
        }
        if comp == "." || comp == ".." {
            return Err("`.` or `..` component not allowed".into());
        }
        if comp.starts_with('.') {
            return Err("hidden file or directory not allowed".into());
        }
    }
    Ok(())
}

/// Checks that a (shape-valid) file path sits in an allowed place of the package layout.
pub fn check_location(p: &str) -> Result<(), String> {
    match p.split_once('/') {
        None if ALLOWED_ROOT_FILES.contains(&p) => Ok(()),
        None => Err(format!("file not allowed at package root (allowed: {})", ALLOWED_ROOT_FILES.join(", "))),
        Some((top, rest)) if ALLOWED_DIRS.contains(&top) => check_native(top, rest),
        Some(_) => Err(format!("top-level directory not allowed (allowed: {})", ALLOWED_DIRS.join(", "))),
    }
}

fn check_native(top: &str, rest: &str) -> Result<(), String> {
    match (top, rest.strip_prefix("native/")) {
        ("client", Some(native)) => match native.split_once('/') {
            Some((abi, file)) if NATIVE_ABIS.contains(&abi) && !file.contains('/') && file.ends_with(".so") => Ok(()),
            _ => Err(format!(
                "native files must be client/native/<abi>/<name>.so with abi one of {}",
                NATIVE_ABIS.join(", ")
            )),
        },
        ("server", Some(_)) => Err("server native code is not supported in package schema 1".into()),
        _ => Ok(()),
    }
}

/// True if the path is a native library (`client/native/...`).
pub fn is_native(p: &str) -> bool {
    p.starts_with("client/native/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_ids() {
        assert!(is_mod_id("hello-lua"));
        assert!(is_mod_id("a_1"));
        assert!(!is_mod_id("a"));
        assert!(!is_mod_id("Hello"));
        assert!(!is_mod_id(&"a".repeat(41)));
        assert!(is_slug("x", 1, 2));
    }

    #[test]
    fn langs() {
        for ok in ["en", "tr", "pt-BR", "zh-Hans", "fil"] {
            assert!(is_lang(ok), "{ok}");
        }
        for bad in ["e", "EN", "english", "pt-", "pt-B", "pt-BR_x"] {
            assert!(!is_lang(bad), "{bad}");
        }
    }

    #[test]
    fn hosts() {
        for ok in ["api.example.com", "*.example.com", "localhost", "a-b.c1"] {
            assert!(is_host(ok), "{ok}");
        }
        for bad in
            ["", "*.", "Example.com", "-a.com", "a-.com", "a..b", &"a".repeat(64), &format!("{}.com", "a.".repeat(130))]
        {
            assert!(!is_host(bad), "{bad}");
        }
    }

    #[test]
    fn paths() {
        assert!(check_path("server/main.luau").is_ok());
        for bad in ["", "/etc/passwd", "a\\b", "C:/x", "a\u{1}b", "a//b", "a/./b", "../x", "a/.hidden", "a/"] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        assert!(check_path(&"a".repeat(MAX_PATH_LEN + 1)).is_err());
    }

    #[test]
    fn locations() {
        for ok in [
            "mod.toml",
            "LICENSE",
            "server/x.luau",
            "client/native/arm64-v8a/libx.so",
            "client/main.luau",
            "assets/a.png",
        ] {
            assert!(check_location(ok).is_ok(), "{ok}");
        }
        for bad in [
            "Cargo.toml",
            "src/lib.rs",
            "client/native/mips/libx.so",
            "client/native/x86/sub/libx.so",
            "client/native/x86/libx.dll",
            "client/native/libx.so",
            "server/native/x86/libx.so",
        ] {
            assert!(check_location(bad).is_err(), "{bad}");
        }
        assert!(is_native("client/native/x86/libx.so"));
        assert!(!is_native("client/main.luau"));
    }
}
