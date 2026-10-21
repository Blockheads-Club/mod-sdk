//! Schema tests: every fixture in `schema/tests/` must pass or fail the JSON Schemas as its directory says, and
//! the Rust validator must agree with `mod.schema.json` on every `mod.toml` fixture and example.

use std::fs;
use std::path::{Path, PathBuf};

use bhmod_manifest::Manifest;
use jsonschema::{Registry, Validator};
use serde_json::Value;

const BASE: &str = "https://blockheads-club.github.io/mod-sdk/schema/";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// TOML -> JSON, with dates as strings (how CI and other tools feed TOML into the schemas).
fn toml_to_json(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => Value::from(f),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect()),
    }
}

fn load_toml(path: &Path) -> Value {
    toml_to_json(toml::from_str(&fs::read_to_string(path).unwrap()).unwrap())
}

fn validator(name: &str) -> Validator {
    let dir = root().join("schema");
    let mut registry = Registry::new();
    for file in ["mod.schema.json", "index-entry.schema.json", "index.schema.json"] {
        registry = registry.add(format!("{BASE}{file}"), load_json(&dir.join(file))).unwrap();
    }
    let registry = registry.prepare().unwrap();
    jsonschema::options().with_registry(&registry).build(&load_json(&dir.join(name))).unwrap()
}

fn fixtures(dir: &str, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<_> = fs::read_dir(root().join("schema/tests").join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no fixtures in {dir}");
    v
}

#[test]
fn mod_schema_fixtures_agree_with_rust_validator() {
    let schema = validator("mod.schema.json");
    let mut valid = fixtures("mod/valid", "toml");
    valid.extend(fs::read_dir(root().join("examples")).unwrap().map(|e| e.unwrap().path().join("mod.toml")));
    for p in valid {
        let errors: Vec<String> = schema.iter_errors(&load_toml(&p)).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{}: schema errors {errors:?}", p.display());
        let parsed = Manifest::parse(&fs::read_to_string(&p).unwrap());
        assert!(parsed.is_ok(), "{}: {parsed:?}", p.display());
    }
    for p in fixtures("mod/invalid", "toml") {
        assert!(!schema.is_valid(&load_toml(&p)), "{}: schema accepted it", p.display());
        assert!(Manifest::parse(&fs::read_to_string(&p).unwrap()).is_err(), "{}: Rust accepted it", p.display());
    }
}

#[test]
fn index_entry_fixtures() {
    let schema = validator("index-entry.schema.json");
    for p in fixtures("index-entry/valid", "toml") {
        let errors: Vec<String> = schema.iter_errors(&load_toml(&p)).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{}: {errors:?}", p.display());
    }
    for p in fixtures("index-entry/invalid", "toml") {
        assert!(!schema.is_valid(&load_toml(&p)), "{}: accepted", p.display());
    }
}

#[test]
fn index_fixtures() {
    let schema = validator("index.schema.json");
    for p in fixtures("index/valid", "json") {
        let errors: Vec<String> = schema.iter_errors(&load_json(&p)).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{}: {errors:?}", p.display());
    }
    for p in fixtures("index/invalid", "json") {
        assert!(!schema.is_valid(&load_json(&p)), "{}: accepted", p.display());
    }
}
