//! Which discovered packages are loaded, and in which order.
//!
//! A package is dropped when it is disabled in `mods.toml`, has no entry on this side, needs an API version the host
//! does not implement, uses a runtime the host lacks, asks for permissions the admin has not granted, misses a
//! required dependency (on this side, with a matching version), or is part of a dependency cycle. Dropping a
//! package can drop its dependents. The rest are ordered topologically (dependencies and present soft
//! dependencies first), ties by mod id.

use std::collections::{BTreeMap, BTreeSet};

use bhmod_manifest::manifest::{DepSide, Runtime};
use semver::{Version, VersionReq};

use crate::config::{missing_permissions, required_permissions, ModsConfig};
use crate::discover::{Package, Problem};
use crate::types::Side;
use crate::API_VERSION;

fn dep_on_side(side: DepSide, host: Side) -> bool {
    match side {
        DepSide::Both => true,
        DepSide::Server => host == Side::Server,
        DepSide::Client => host == Side::Client,
    }
}

fn dep_side(d: &bhmod_manifest::manifest::Dependency) -> DepSide {
    match d {
        bhmod_manifest::manifest::Dependency::Req(_) => DepSide::Both,
        bhmod_manifest::manifest::Dependency::Detailed { side, .. } => *side,
    }
}

/// Filters and orders packages. `runtimes` are the runtimes the host can run. Skipped packages are reported as
/// problems, except those that are disabled or have no entry for this side (reported in `skipped`).
pub fn resolve(
    packages: Vec<Package>,
    config: &ModsConfig,
    side: Side,
    runtimes: &[Runtime],
) -> (Vec<Package>, Vec<Problem>, Vec<String>) {
    let mut problems = Vec::new();
    let mut skipped = Vec::new();
    let api = Version::parse(API_VERSION).expect("API_VERSION is semver");
    let mut by_id: BTreeMap<String, Package> = BTreeMap::new();
    for p in packages {
        let id = p.manifest.id.clone();
        let mut fail = |msg: String| problems.push(Problem { origin: p.origin.clone(), message: msg });
        if by_id.contains_key(&id) {
            fail(format!("{id}: another package with the same id is already loaded"));
            continue;
        }
        let cfg = config.get(&id);
        if !cfg.enabled {
            skipped.push(format!("{id}: disabled in mods.toml"));
            continue;
        }
        let entry = match side {
            Side::Server => p.manifest.entry.server.as_ref(),
            Side::Client => p.manifest.entry.client.as_ref(),
        };
        let Some(entry) = entry else {
            skipped.push(format!("{id}: no {} entry", side.name()));
            continue;
        };
        // The manifest crate validated `api` as a requirement already.
        if !VersionReq::parse(&p.manifest.api).is_ok_and(|r| r.matches(&api)) {
            fail(format!("{id}: needs API {}, this host implements {API_VERSION}", p.manifest.api));
            continue;
        }
        if !runtimes.contains(&entry.runtime) {
            fail(format!("{id}: runtime {:?} is not available in this host", entry.runtime));
            continue;
        }
        let missing = missing_permissions(&required_permissions(&p.manifest.permissions), &cfg.grant);
        if !missing.is_empty() {
            fail(format!("{id}: permissions not granted in mods.toml: {}", missing.join(", ")));
            continue;
        }
        by_id.insert(id, p);
    }

    // Drop packages with missing / mismatched required dependencies until nothing changes.
    loop {
        let mut dropped = None;
        'outer: for (id, p) in &by_id {
            for (dep, d) in &p.manifest.dependencies {
                if !dep_on_side(dep_side(d), side) {
                    continue;
                }
                let req = VersionReq::parse(d.version()).expect("validated by bhmod-manifest");
                let ok = by_id.get(dep).is_some_and(|q| req.matches(&q.manifest.semver()));
                if !ok {
                    dropped = Some((id.clone(), format!("{id}: needs {dep} {} which is not loaded", d.version())));
                    break 'outer;
                }
            }
        }
        match dropped {
            Some((id, msg)) => {
                let p = by_id.remove(&id).expect("present");
                problems.push(Problem { origin: p.origin, message: msg });
            }
            None => break,
        }
    }

    // Kahn's algorithm; edges dep -> dependent.
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (id, p) in &by_id {
        let set = p
            .manifest
            .dependencies
            .iter()
            .chain(p.manifest.soft_dependencies.iter())
            .filter(|(dep, _)| by_id.contains_key(*dep))
            .map(|(dep, _)| dep.clone())
            .collect();
        deps.insert(id.clone(), set);
    }
    let mut order = Vec::new();
    loop {
        let ready = deps.iter().find(|(_, d)| d.is_empty()).map(|(id, _)| id.clone());
        let Some(id) = ready else { break };
        deps.remove(&id);
        for d in deps.values_mut() {
            d.remove(&id);
        }
        order.push(by_id.remove(&id).expect("present"));
    }
    for (id, p) in by_id {
        problems.push(Problem { origin: p.origin, message: format!("{id}: dependency cycle") });
    }
    (order, problems, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::discover;
    use crate::discover::tests::write_mod;

    fn pkgs(mods: &[(&str, &str)]) -> Vec<Package> {
        let tmp = tempfile::tempdir().unwrap();
        for (id, extra) in mods {
            write_mod(tmp.path(), id, id, extra, "return {}");
        }
        let (p, problems) = discover(tmp.path());
        assert!(problems.is_empty(), "{problems:?}");
        p
    }

    fn ids(p: &[Package]) -> Vec<&str> {
        p.iter().map(|p| p.manifest.id.as_str()).collect()
    }

    #[test]
    fn orders_by_dependencies() {
        let p = pkgs(&[
            ("aa", "[dependencies]\ncc = \"^1\"\n"),
            ("bb", "[soft-dependencies]\naa = \"*\"\nzz = \"*\"\n"),
            ("cc", ""),
            ("dd", "[dependencies]\nee = { version = \"^1\", side = \"client\" }\n"),
        ]);
        let (order, problems, skipped) = resolve(p, &ModsConfig::default(), Side::Server, &[Runtime::Lua]);
        assert_eq!(ids(&order), vec!["cc", "aa", "bb", "dd"]);
        assert!(problems.is_empty() && skipped.is_empty());
    }

    #[test]
    fn drops_and_reports() {
        let p = pkgs(&[
            ("aa", "[dependencies]\nmissing = \"^1\"\n"),
            ("bb", "[dependencies]\naa = \"^1\"\n"),
            ("cc", "[dependencies]\ndd = \"^1\"\n"),
            ("dd", "[dependencies]\ncc = \"^1\"\n"),
            ("ee", "[dependencies]\nff = \"^2\"\n"),
            ("ff", ""),
            ("gg", "[permissions]\nplayers = [\"kick\"]\n"),
            ("hh", ""),
            ("ii", "[dependencies]\nff = { version = \"^1\", side = \"server\" }\n"),
        ]);
        let cfg = ModsConfig::parse("[hh]\nenabled = false\n").unwrap();
        let (order, problems, skipped) = resolve(p, &cfg, Side::Server, &[Runtime::Lua]);
        assert_eq!(ids(&order), vec!["ff", "ii"]);
        let msgs: Vec<_> = problems.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(
            msgs,
            vec![
                "gg: permissions not granted in mods.toml: players:kick",
                "aa: needs missing ^1 which is not loaded",
                "bb: needs aa ^1 which is not loaded",
                "ee: needs ff ^2 which is not loaded",
                "cc: dependency cycle",
                "dd: dependency cycle",
            ]
        );
        assert_eq!(skipped, vec!["hh: disabled in mods.toml"]);
    }

    #[test]
    fn side_api_runtime_and_duplicates() {
        let mut p = pkgs(&[("aa", ""), ("bb", "")]);
        let dup = p[0].clone();
        p.push(dup);
        p[1].manifest.api = "^0.2".into();
        let (order, problems, _) = resolve(p.clone(), &ModsConfig::default(), Side::Server, &[Runtime::Lua]);
        assert_eq!(ids(&order), vec!["aa"]);
        assert!(problems[0].message.contains("needs API ^0.2"));
        assert!(problems[1].message.contains("same id"));
        let (order, problems, skipped) = resolve(p.clone(), &ModsConfig::default(), Side::Server, &[Runtime::Wasm]);
        assert!(order.is_empty());
        assert!(problems[0].message.contains("runtime Lua is not available"));
        assert!(skipped.is_empty());
        let (order, _, skipped) = resolve(p, &ModsConfig::default(), Side::Client, &[Runtime::Lua]);
        assert!(order.is_empty());
        assert_eq!(skipped[0], "aa: no client entry");
        // Client-side dependency checks.
        assert!(dep_on_side(DepSide::Client, Side::Client));
        assert!(!dep_on_side(DepSide::Server, Side::Client));
    }
}
