//! Building `index.json` from the entry files.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::entry::{schema_errors, Schema};
use crate::repo::Repo;
use crate::validate::package_file_name;
use crate::Issue;

/// Build settings.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Keep `draft = true` entries (local testing and the website's sample data only).
    pub include_drafts: bool,
    /// Release download base of the index repo; versions without `mirror` get
    /// `<base>/<id>-<version>/<id>-<version>.bhmod`.
    pub mirror_base: Option<String>,
    /// UTC build time, `YYYY-MM-DDTHH:MM:SSZ`.
    pub generated: String,
    /// `blockheads:api` versions the index was checked against.
    pub api_versions: Vec<String>,
}

/// Builds the `index.json` text (pretty JSON with a final newline), or every problem found.
pub fn build(repo: &Repo, opts: &BuildOptions) -> Result<String, Vec<Issue>> {
    let mut errors = repo.issues.clone();
    let mut mods = Vec::new();
    for (id, l) in &repo.entries {
        if l.entry.draft && !opts.include_drafts {
            continue;
        }
        let mut entry = l.json.clone();
        let obj = entry.as_object_mut().expect("entry is an object");
        obj.remove("draft");
        for v in obj.get_mut("versions").and_then(Value::as_array_mut).expect("versions is an array") {
            let ver = v["version"].as_str().expect("version is a string").to_string();
            if v.get("size").is_none() {
                errors.push(Issue::new(&l.path, format!("{ver}: size is required in index.json")));
            }
            if let (Some(base), None) = (&opts.mirror_base, v.get("mirror")) {
                let name = package_file_name(id, &ver);
                v["mirror"] = Value::String(format!("{}/{id}-{ver}/{name}", base.trim_end_matches('/')));
            }
        }
        mods.push(entry);
    }
    let index = json!({ "schema": 1, "generated": opts.generated, "api": opts.api_versions, "mods": mods });
    errors.extend(schema_errors(Schema::Index, &index).into_iter().map(|e| Issue::new("index.json", e)));
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(serde_json::to_string_pretty(&index).expect("JSON serializes") + "\n")
}

/// `YYYY-MM-DDTHH:MM:SSZ` for seconds since the Unix epoch.
pub fn format_utc(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil-from-days (Howard Hinnant), valid for every date after 1970.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

/// The current UTC time as [`format_utc`].
pub fn utc_now() -> String {
    format_utc(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default())
}
