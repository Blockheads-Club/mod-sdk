//! Calls from the Java UI (on its worker threads) about the player's installed mods. They only touch the file
//! system, never the running host: changes apply when the next world opens. Results are JSON strings.

use std::fs;
use std::path::Path;

use bhmod_client::cache::sha256;
use bhmod_client::installed::{InstallError, Installed};
use bhmod_client::verify::check_package;
use bhmod_client::Caps;
use bhmod_manifest::Manifest;
use bhmod_proto::hex;
use serde_json::{json, Value};

fn manifest_json(m: &Manifest) -> Value {
    let p = &m.permissions;
    json!({
        "id": m.id,
        "version": m.version,
        "name": m.name,
        "description": m.description,
        "authors": m.authors,
        "client": m.sides.client,
        "server": m.sides.server,
        "permissions": {
            "world": bhmod_core::config::required_permissions(p).iter().find(|x| x.starts_with("world:")).map(|x| &x[6..]).unwrap_or("none"),
            "players": bhmod_core::config::required_permissions(p).iter().filter_map(|x| x.strip_prefix("players:")).collect::<Vec<_>>(),
            "netHttp": p.net_http,
            "channels": p.channels,
            "native": p.native,
        },
    })
}

fn error(code: &str, text: &str) -> String {
    json!({"ok": false, "code": code, "text": text}).to_string()
}

/// Every installed mod.
pub fn list(dir: &Path, caps: &Caps) -> String {
    let mods: Vec<Value> = Installed::new(dir, caps.clone())
        .list()
        .into_iter()
        .map(|m| {
            let mut v = manifest_json(&m.manifest);
            v["enabled"] = json!(m.enabled);
            v["sha256"] = json!(m.sha256);
            v["source"] = json!(m.source);
            v["size"] = json!(fs::metadata(&m.path).map(|x| x.len()).unwrap_or(0));
            v
        })
        .collect();
    json!({"ok": true, "mods": mods}).to_string()
}

fn read_checked(file: &Path, expected_sha: Option<&str>) -> Result<Vec<u8>, String> {
    let bytes = fs::read(file).map_err(|e| error("io", &e.to_string()))?;
    if let Some(want) = expected_sha {
        let got = hex(&sha256(&bytes));
        if !got.eq_ignore_ascii_case(want) {
            return Err(error("hash-mismatch", &format!("expected {want}, got {got}")));
        }
    }
    Ok(bytes)
}

/// Checks a downloaded package without installing it (for the confirm dialog).
pub fn inspect(file: &Path, expected_sha: Option<&str>, caps: &Caps) -> String {
    let bytes = match read_checked(file, expected_sha) {
        Ok(b) => b,
        Err(e) => return e,
    };
    match check_package(&bytes, caps) {
        Ok(m) => json!({"ok": true, "mod": manifest_json(&m), "size": bytes.len(), "sha256": hex(&sha256(&bytes))})
            .to_string(),
        Err(r) => error(r.code(), &r.to_string()),
    }
}

/// Installs a downloaded package (checked again; `expected_sha` from the index).
pub fn install(dir: &Path, file: &Path, expected_sha: Option<&str>, source: Option<&str>, caps: &Caps) -> String {
    let bytes = match read_checked(file, expected_sha) {
        Ok(b) => b,
        Err(e) => return e,
    };
    match Installed::new(dir, caps.clone()).install(&bytes, source) {
        Ok(m) => json!({"ok": true, "mod": manifest_json(&m.manifest)}).to_string(),
        Err(InstallError::Refused(r)) => error(r.code(), &r.to_string()),
        Err(e @ InstallError::Io(_)) => error("io", &e.to_string()),
    }
}

/// Turns an installed mod on or off.
pub fn set_enabled(dir: &Path, id: &str, on: bool, caps: &Caps) -> bool {
    Installed::new(dir, caps.clone()).set_enabled(id, on).unwrap_or(false)
}

/// Removes an installed mod.
pub fn remove(dir: &Path, id: &str, caps: &Caps) -> bool {
    Installed::new(dir, caps.clone()).remove(id).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::package;

    fn v(s: String) -> Value {
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn install_flow() {
        let t = tempfile::tempdir().unwrap();
        let caps = crate::caps();
        let dir = t.path().join("installed");
        let f = t.path().join("dl.bhmod");
        fs::write(
            &f,
            package("alpha", "1.0.0", "world = \"read\"\nplayers = [\"chat\"]\n", "client/main.luau", "return {}"),
        )
        .unwrap();
        let sha = hex(&sha256(&fs::read(&f).unwrap()));
        let i = v(inspect(&f, Some(&sha), &caps));
        assert_eq!(i["mod"]["permissions"]["world"], "read");
        assert_eq!(i["mod"]["permissions"]["players"][0], "chat");
        assert_eq!(v(inspect(&f, Some("00"), &caps))["code"], "hash-mismatch");
        assert_eq!(v(inspect(&t.path().join("none"), None, &caps))["code"], "io");
        assert_eq!(v(install(&dir, &t.path().join("none"), None, None, &caps))["code"], "io");
        assert_eq!(v(install(&dir, &f, Some(&sha), Some("https://x"), &caps))["ok"], true);
        let l = v(list(&dir, &caps));
        assert_eq!(l["mods"][0]["source"], "https://x");
        assert_eq!(l["mods"][0]["enabled"], true);
        assert!(set_enabled(&dir, "alpha", false, &caps));
        assert!(!set_enabled(&dir, "nope", false, &caps));
        assert!(remove(&dir, "alpha", &caps));
        assert!(!remove(&dir, "alpha", &caps));
        // refused packages
        fs::write(&f, b"junk").unwrap();
        assert_eq!(v(inspect(&f, None, &caps))["code"], "unreadable");
        assert_eq!(v(install(&dir, &f, None, None, &caps))["code"], "unreadable");
        // the installed dir is a file
        fs::write(&f, package("alpha", "1.0.0", "", "client/main.luau", "return {}")).unwrap();
        let file_dir = t.path().join("filedir");
        fs::write(&file_dir, b"x").unwrap();
        assert_eq!(v(install(&file_dir, &f, None, None, &caps))["code"], "io");
    }
}
