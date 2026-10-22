//! Luau checks: UTF-8, not empty, and (when a `luau-compile` binary is given) a syntax check with
//! `luau-compile --null`.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

/// Checks Luau sources `(path, bytes)`; returns `(path, problem)` pairs.
pub fn check(files: &[(String, Vec<u8>)], luau_compile: Option<&Path>) -> io::Result<Vec<(String, String)>> {
    let mut problems = Vec::new();
    let mut good = Vec::new();
    for (path, bytes) in files {
        match std::str::from_utf8(bytes) {
            Err(_) => problems.push((path.clone(), "not valid UTF-8".to_string())),
            Ok(t) if t.trim().is_empty() => problems.push((path.clone(), "empty Luau file".to_string())),
            Ok(_) => good.push((path, bytes)),
        }
    }
    let Some(bin) = luau_compile else { return Ok(problems) };
    let dir = tempfile::tempdir()?;
    for (path, bytes) in good {
        let file = dir.path().join("check.luau");
        fs::write(&file, bytes)?;
        let out = Command::new(bin).arg("--null").arg(&file).output()?;
        if !out.status.success() {
            let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
            let text = text.replace(&file.display().to_string(), path);
            problems.push((path.clone(), format!("luau-compile: {}", text.trim())));
        }
    }
    Ok(problems)
}
