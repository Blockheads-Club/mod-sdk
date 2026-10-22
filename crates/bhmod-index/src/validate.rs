//! The pull request rules of the index repo.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Cursor};
use std::path::PathBuf;

use bhmod_manifest::manifest::Manifest;
use bhmod_manifest::pack::sha256_hex;
use bhmod_manifest::package::{check, CheckOptions, Source, ZipSource};
use semver::{Version, VersionReq};
use serde_json::Value;

use crate::entry::{permission_labels, VersionEntry};
use crate::repo::Repo;
use crate::{luau, wasm, Issue};

/// Label: the PR adds a mod id that is not in the index yet.
pub const LABEL_NEW_MOD: &str = "new-mod";
/// Label: a new mod, native code or new permissions; a maintainer must review the code before merge.
pub const LABEL_SECURITY: &str = "needs-security-review";
/// Label: the PR yanks a version.
pub const LABEL_YANK: &str = "yank";

/// Validation settings.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The index before the PR (the base branch). `None`: every entry and version is treated as new.
    pub base: Option<Repo>,
    /// GitHub login of the PR author. `None` skips the owner check (local runs).
    pub author: Option<String>,
    /// Folder holding the packages as `<id>-<version>.bhmod`.
    pub packages: Option<PathBuf>,
    /// `luau-compile` binary for the Luau syntax check.
    pub luau_compile: Option<PathBuf>,
    /// `blockheads:api` versions the hosts support; every new version's `api` must match one.
    pub api_versions: Vec<Version>,
    /// Allow more than one changed mod (maintainer bulk edits, local runs).
    pub allow_multiple: bool,
}

/// What a PR does to one mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// New id.
    New,
    /// Existing id changed.
    Updated,
    /// Id deleted.
    Removed,
}

/// Permission change of the newest added version against the previous one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionDiff {
    /// The previous (non-yanked) version, if any.
    pub from: Option<String>,
    /// The newest added version.
    pub to: String,
    /// All permissions of `to`.
    pub all: Vec<String>,
    /// Added since `from`.
    pub added: Vec<String>,
    /// Removed since `from`.
    pub removed: Vec<String>,
}

/// One changed mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Mod id.
    pub id: String,
    /// Kind of change.
    pub kind: ChangeKind,
    /// Versions added.
    pub added: Vec<String>,
    /// Versions newly yanked.
    pub yanked: Vec<String>,
    /// Owners list changed.
    pub owners_changed: bool,
    /// Permission diff when versions were added.
    pub permissions: Option<PermissionDiff>,
}

/// The validation result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Problems that fail the check.
    pub errors: Vec<Issue>,
    /// Notes that do not.
    pub warnings: Vec<Issue>,
    /// Labels to put on the PR.
    pub labels: BTreeSet<String>,
    /// Changed mods.
    pub changes: Vec<Change>,
}

impl Outcome {
    /// True when there are no errors.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Whether a maintainer security review is needed.
    pub fn needs_review(&self) -> bool {
        self.labels.contains(LABEL_SECURITY)
    }

    /// The PR comment (Markdown).
    pub fn markdown(&self) -> String {
        let mut s = String::from("## Mod index check\n\n");
        let verdict = if self.is_ok() { "**Passed.**" } else { "**Failed.** Fix the errors below and push again." };
        let _ = writeln!(s, "{verdict}\n");
        if self.changes.is_empty() {
            s.push_str("No mod entries changed.\n");
        }
        for c in &self.changes {
            let kind = match c.kind {
                ChangeKind::New => "new mod",
                ChangeKind::Updated => "update",
                ChangeKind::Removed => "removed",
            };
            let _ = writeln!(s, "### `{}` ({kind})\n", c.id);
            if !c.added.is_empty() {
                let _ = writeln!(s, "- New versions: {}", c.added.join(", "));
            }
            if !c.yanked.is_empty() {
                let _ = writeln!(s, "- Yanked: {}", c.yanked.join(", "));
            }
            if c.owners_changed {
                s.push_str("- The owners list changed.\n");
            }
            if let Some(p) = &c.permissions {
                let from = p.from.as_deref().map(|f| format!(" (compared with {f})")).unwrap_or_default();
                let _ = writeln!(s, "\n**Permissions of {}**{from}:\n", p.to);
                if p.all.is_empty() && p.removed.is_empty() {
                    s.push_str("- none\n");
                }
                for perm in &p.all {
                    let mark = if p.added.contains(perm) && p.from.is_some() { " **(new)**" } else { "" };
                    let warn = if perm == "native" { " - unsandboxed native code" } else { "" };
                    let _ = writeln!(s, "- `{perm}`{mark}{warn}");
                }
                for perm in &p.removed {
                    let _ = writeln!(s, "- ~~`{perm}`~~ (removed)");
                }
            }
            s.push('\n');
        }
        if !self.labels.is_empty() {
            let labels: Vec<String> = self.labels.iter().map(|l| format!("`{l}`")).collect();
            let _ = writeln!(s, "Labels: {}\n", labels.join(", "));
        }
        if self.needs_review() {
            s.push_str(
                "A maintainer must review this mod's code before it can be merged \
                 (every new mod, native code, and updates that add permissions).\n\n",
            );
        }
        for (title, list) in [("Errors", &self.errors), ("Warnings", &self.warnings)] {
            if !list.is_empty() {
                let _ = writeln!(s, "#### {title}\n");
                for i in list {
                    let _ = writeln!(s, "- {i}");
                }
                s.push('\n');
            }
        }
        s
    }
}

/// The canonical package file name, `<id>-<version>.bhmod`.
pub fn package_file_name(id: &str, version: &str) -> String {
    format!("{id}-{version}.bhmod")
}

/// Versions in `head` that are not in `base` (all versions when `base` is `None`), as `(id, version)`.
pub fn new_versions<'a>(head: &'a Repo, base: Option<&Repo>) -> Vec<(&'a str, &'a VersionEntry)> {
    let mut v = Vec::new();
    for (id, l) in &head.entries {
        let old = base.and_then(|b| b.entries.get(id));
        for ver in &l.entry.versions {
            if old.is_none_or(|o| o.entry.version(&ver.version).is_none()) {
                v.push((id.as_str(), ver));
            }
        }
    }
    v
}

/// What CI must download before [`validate`]: `(url, file name)` for each new version.
pub fn downloads(head: &Repo, base: Option<&Repo>) -> Vec<(String, String)> {
    new_versions(head, base).into_iter().map(|(id, v)| (v.url.clone(), package_file_name(id, &v.version))).collect()
}

fn changed_ids(head: &Repo, base: Option<&Repo>) -> Vec<(String, ChangeKind)> {
    let Some(base) = base else {
        return head.entries.keys().map(|k| (k.clone(), ChangeKind::New)).collect();
    };
    let ids: BTreeSet<&String> = head.entries.keys().chain(base.entries.keys()).collect();
    ids.into_iter()
        .filter_map(|id| match (base.entries.get(id), head.entries.get(id)) {
            (None, _) => Some((id.clone(), ChangeKind::New)),
            (Some(_), None) => Some((id.clone(), ChangeKind::Removed)),
            (Some(b), Some(h)) => (b.json != h.json).then(|| (id.clone(), ChangeKind::Updated)),
        })
        .collect()
}

fn strip(v: &Value) -> Value {
    let mut v = v.clone();
    let obj = v.as_object_mut().expect("version is an object");
    obj.remove("yanked");
    obj.remove("mirror");
    v
}

/// Runs every rule. `Err` only for I/O failures (unreadable package folder, `luau-compile` not runnable).
pub fn validate(head: &Repo, opts: &Options) -> io::Result<Outcome> {
    let mut out = Outcome { errors: head.issues.clone(), ..Outcome::default() };
    let base = opts.base.as_ref();
    let changed = changed_ids(head, base);
    if changed.len() > 1 && !opts.allow_multiple {
        let ids: Vec<&str> = changed.iter().map(|(id, _)| id.as_str()).collect();
        out.errors.push(Issue::new(
            "pull request",
            format!("change one mod per pull request (this one changes {})", ids.join(", ")),
        ));
    }
    if opts.author.is_none() {
        out.warnings.push(Issue::new("owners", "owner check skipped (no --author)"));
    }
    for (id, kind) in changed {
        let change = check_mod(head, opts, &id, kind, &mut out)?;
        out.changes.push(change);
    }
    Ok(out)
}

fn check_mod(head: &Repo, opts: &Options, id: &str, kind: ChangeKind, out: &mut Outcome) -> io::Result<Change> {
    let base = opts.base.as_ref().and_then(|b| b.entries.get(id));
    let mut change =
        Change { id: id.to_string(), kind, added: vec![], yanked: vec![], owners_changed: false, permissions: None };
    let Some(h) = head.entries.get(id) else {
        out.errors.push(Issue::new(id, "removing a mod is done by maintainers; yank its versions instead"));
        return Ok(change);
    };
    let path = h.path.as_str();
    let author = opts.author.as_deref().map(str::to_lowercase);
    let is_owner = |owners: &[String], a: &str| owners.iter().any(|o| o.to_lowercase() == a);
    if kind == ChangeKind::New {
        out.labels.insert(LABEL_NEW_MOD.into());
        out.labels.insert(LABEL_SECURITY.into());
        if let Some(a) = author.as_deref().filter(|a| !is_owner(&h.entry.owners, a)) {
            out.errors.push(Issue::new(path, format!("the PR author `{a}` must be listed in owners")));
        }
    }
    if let Some(b) = base {
        change.owners_changed = b.entry.owners != h.entry.owners;
        if let Some(a) = author.as_deref().filter(|a| !is_owner(&b.entry.owners, a)) {
            out.errors.push(Issue::new(
                path,
                format!("`{a}` is not an owner of `{id}`; only {} may change it", b.entry.owners.join(", ")),
            ));
        }
        check_append_only(path, b, h, &mut change, out);
    }
    check_order(path, &h.entry.versions, out);
    let added: Vec<&VersionEntry> =
        h.entry.versions.iter().filter(|v| base.is_none_or(|b| b.entry.version(&v.version).is_none())).collect();
    for v in &added {
        change.added.push(v.version.clone());
        check_version(head, opts, id, h.entry.draft, v, out)?;
    }
    if !change.yanked.is_empty() {
        out.labels.insert(LABEL_YANK.into());
    }
    if let Some(last) = added.last() {
        let prev = base.and_then(|b| b.entry.versions.iter().rev().find(|v| !v.yanked));
        let all = permission_labels(&last.permissions);
        let old = prev.map(|p| permission_labels(&p.permissions)).unwrap_or_default();
        let diff = PermissionDiff {
            from: prev.map(|p| p.version.clone()),
            to: last.version.clone(),
            added: all.iter().filter(|p| !old.contains(p)).cloned().collect(),
            removed: old.iter().filter(|p| !all.contains(p)).cloned().collect(),
            all,
        };
        // New mods are labelled above; here: native code, or permissions the last published version did not have.
        if added.iter().any(|v| v.permissions.native) || !diff.added.is_empty() {
            out.labels.insert(LABEL_SECURITY.into());
        }
        change.permissions = Some(diff);
    }
    Ok(change)
}

fn check_append_only(
    path: &str,
    b: &crate::repo::Loaded,
    h: &crate::repo::Loaded,
    change: &mut Change,
    out: &mut Outcome,
) {
    let (bj, hj) = (&b.json["versions"], &h.json["versions"]);
    for (i, old) in b.entry.versions.iter().enumerate() {
        let at = format!("{path} {}", old.version);
        let Some(new) = h.entry.versions.get(i).filter(|n| n.version == old.version) else {
            out.errors
                .push(Issue::new(at, "published versions cannot be removed or reordered; add yanked = true instead"));
            continue;
        };
        if strip(&bj[i]) != strip(&hj[i]) {
            out.errors.push(Issue::new(&at, "published versions cannot be edited (only yanked = true may be added)"));
        }
        if old.mirror.is_some() && old.mirror != new.mirror {
            out.errors.push(Issue::new(&at, "mirror is set by CI and cannot be changed"));
        }
        match (old.yanked, new.yanked) {
            (true, false) => {
                out.errors.push(Issue::new(&at, "a yanked version cannot be restored; publish a new version"))
            }
            (false, true) => change.yanked.push(old.version.clone()),
            _ => {}
        }
    }
}

fn check_order(path: &str, versions: &[VersionEntry], out: &mut Outcome) {
    let mut last: Option<Version> = None;
    for v in versions {
        let cur = Version::parse(&v.version).expect("the schema checks semver");
        if let Some(prev) = last.as_ref().filter(|p| cur <= **p) {
            out.errors.push(Issue::new(
                path,
                format!("version {cur} must be greater than {prev} (append newer versions at the end)"),
            ));
        }
        last = Some(cur);
    }
}

fn req(at: &str, field: &str, text: &str, out: &mut Outcome) -> Option<VersionReq> {
    let r = VersionReq::parse(text).ok();
    if r.is_none() {
        out.errors.push(Issue::new(at, format!("{field} `{text}` is not a semver requirement")));
    }
    r
}

fn check_version(
    head: &Repo,
    opts: &Options,
    id: &str,
    draft: bool,
    v: &VersionEntry,
    out: &mut Outcome,
) -> io::Result<()> {
    let at = format!("{id} {}", v.version);
    if v.size.is_none() {
        out.errors.push(Issue::new(&at, "size is required for new versions"));
    }
    if let Some(r) = req(&at, "api", &v.api, out).filter(|r| !opts.api_versions.iter().any(|a| r.matches(a))) {
        let supported: Vec<String> = opts.api_versions.iter().map(ToString::to_string).collect();
        out.errors
            .push(Issue::new(&at, format!("api `{r}` matches no supported API version ({})", supported.join(", "))));
    }
    if let Some(l) = &v.loader {
        req(&at, "loader", l, out);
    }
    for (soft, deps) in [(false, &v.depends), (true, &v.soft_depends)] {
        for (dep, text) in deps {
            let Some(r) = req(&at, "dependency", text, out) else { continue };
            let found = head.entries.get(dep).filter(|d| draft || !d.entry.draft).is_some_and(|d| {
                d.entry
                    .versions
                    .iter()
                    .any(|x| !x.yanked && Version::parse(&x.version).is_ok_and(|ver| r.matches(&ver)))
            });
            match (found, soft) {
                (true, _) => {}
                (false, false) => {
                    out.errors.push(Issue::new(&at, format!("dependency `{dep} {r}` is not in the index")))
                }
                (false, true) => {
                    out.warnings.push(Issue::new(&at, format!("soft dependency `{dep} {r}` is not in the index")))
                }
            }
        }
    }
    let Some(dir) = &opts.packages else {
        out.errors.push(Issue::new(&at, "package not checked (no --packages folder)"));
        return Ok(());
    };
    let file = dir.join(package_file_name(id, &v.version));
    if !file.is_file() {
        out.errors.push(Issue::new(&at, format!("package {} not found (download {})", file.display(), v.url)));
        return Ok(());
    }
    let bytes = fs::read(&file)?;
    check_package(&at, id, v, &bytes, opts, out)
}

fn check_package(
    at: &str,
    id: &str,
    v: &VersionEntry,
    bytes: &[u8],
    opts: &Options,
    out: &mut Outcome,
) -> io::Result<()> {
    let sha = sha256_hex(bytes);
    if sha != v.sha256 {
        out.errors.push(Issue::new(at, format!("sha256 is {sha}, the entry says {}", v.sha256)));
    }
    if v.size.is_some_and(|s| s != bytes.len() as u64) {
        out.errors
            .push(Issue::new(at, format!("size is {}, the entry says {}", bytes.len(), v.size.unwrap_or_default())));
    }
    let mut zip = match ZipSource::open(Cursor::new(bytes.to_vec()), bytes.len() as u64) {
        Ok(z) => z,
        Err(e) => {
            out.errors.push(Issue::new(at, format!("not a valid .bhmod zip: {e}")));
            return Ok(());
        }
    };
    let report = check(&mut zip, CheckOptions { index: true });
    out.errors.extend(report.errors.iter().map(|i| Issue::new(format!("{at} {}", i.path), &i.message)));
    out.warnings.extend(report.warnings.iter().map(|i| Issue::new(format!("{at} {}", i.path), &i.message)));
    if let Some(m) = &report.manifest {
        for problem in compare_manifest(m, id, v) {
            out.errors.push(Issue::new(format!("{at} mod.toml"), problem));
        }
    }
    let paths: Vec<String> = zip.files().iter().map(|f| f.path.clone()).collect();
    let mut lua = Vec::new();
    for p in &paths {
        if p.ends_with(".wasm") {
            let data = zip.read(p, u64::MAX)?;
            out.errors.extend(wasm::check(&data).into_iter().map(|m| Issue::new(format!("{at} {p}"), m)));
        } else if p.ends_with(".luau") {
            lua.push((p.clone(), zip.read(p, u64::MAX)?));
        }
    }
    let problems = luau::check(&lua, opts.luau_compile.as_deref())?;
    out.errors.extend(problems.into_iter().map(|(p, m)| Issue::new(format!("{at} {p}"), m)));
    Ok(())
}

/// Differences between a package's `mod.toml` and its index version entry.
pub fn compare_manifest(m: &Manifest, id: &str, v: &VersionEntry) -> Vec<String> {
    let mut p = Vec::new();
    let mut ne = |what: &str, got: String, want: String| {
        if got != want {
            p.push(format!("{what} is `{got}` in mod.toml but `{want}` in the entry"));
        }
    };
    ne("id", m.id.clone(), id.to_string());
    ne("version", m.version.clone(), v.version.clone());
    let mut sides = Vec::new();
    if m.sides.server {
        sides.push("server");
    }
    if m.sides.client {
        sides.push("client");
    }
    let mut entry_sides: Vec<&str> = v.sides.iter().map(String::as_str).collect();
    entry_sides.sort_by_key(|s| *s != "server");
    ne("sides", sides.join(", "), entry_sides.join(", "));
    ne("client-required", m.client_required().to_string(), v.client_required.unwrap_or(false).to_string());
    ne("api", m.api.clone(), v.api.clone());
    ne("loader", m.loader.clone().unwrap_or_default(), v.loader.clone().unwrap_or_default());
    let reqs = |d: &BTreeMap<String, bhmod_manifest::manifest::Dependency>| {
        d.iter().map(|(k, d)| format!("{k} {}", d.version())).collect::<Vec<_>>().join(", ")
    };
    let entry_reqs =
        |d: &BTreeMap<String, String>| d.iter().map(|(k, r)| format!("{k} {r}")).collect::<Vec<_>>().join(", ");
    ne("dependencies", reqs(&m.dependencies), entry_reqs(&v.depends));
    ne("soft-dependencies", reqs(&m.soft_dependencies), entry_reqs(&v.soft_depends));
    ne("permissions", permission_labels(&m.permissions).join(", "), permission_labels(&v.permissions).join(", "));
    p
}
