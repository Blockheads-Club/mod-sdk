//! `[settings.<key>]` in `mod.toml` (API 0.3.3): the typed settings a mod offers its admin (and players, for
//! `scope = "client"`), the validator for their values and the canonical JSON export the host app and the 0xBC
//! `SETTINGS_SCHEMA` op use.
//!
//! ```toml
//! [settings.radius]
//! type = "int"
//! default = 32
//! min = 4
//! max = 128
//! label = { en = "Hearing radius (blocks)", tr = "Duyma yarıçapı (blok)" }
//! group = "proximity"
//!
//! [settings.channels]
//! type = "list"
//! default = []
//! label = "Channels"
//! item = { name = { type = "string", default = "", label = "Name", max_length = 32 }, password = { type = "string", default = "", label = "Password" } }
//! ```
//!
//! Values are JSON ([`serde_json::Value`]): `bool`, numbers, strings, `roles` = list of role names, `matrix` =
//! `{row: {col: cell}}` with every row and column, `list` = list of records. [`validate_value`] checks a value
//! against its field and returns it normalised (an integer given for a `float` becomes a float, a list record gets
//! the defaults of the fields it leaves out). [`schema_json`] builds `{ "fields": [...], "values": {...} }` with the
//! fields in declaration order.

use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;
use serde_json::{Map, Value as Json};
use toml::Spanned;

use crate::manifest::{Localized, Manifest};
use crate::rules::is_lang;
use crate::Issue;

/// Longest setting key.
pub const MAX_KEY: usize = 48;
/// Most settings per mod.
pub const MAX_FIELDS: usize = 128;
/// Most fields of one `list` item.
pub const MAX_ITEM_FIELDS: usize = 32;
/// `max_length` of a `string` setting that gives none.
pub const DEFAULT_MAX_LENGTH: u64 = 256;
/// Largest allowed `max_length`.
pub const MAX_STRING: u64 = 65_536;
/// `max_items` of a `list` setting that gives none.
pub const DEFAULT_MAX_ITEMS: u64 = 100;
/// Largest allowed `max_items`.
pub const MAX_ITEMS: u64 = 1000;
/// Most roles in one `roles` value, most entries of `values`, `rows` and `cols`.
pub const MAX_ENTRIES: usize = 64;
/// Longest label in characters.
pub const MAX_LABEL: usize = 128;
/// Longest help text in characters.
pub const MAX_HELP: usize = 1024;
/// Largest integer magnitude (2^53): what a Luau number and a JSON consumer keep exact.
pub const MAX_SAFE_INT: i64 = 9_007_199_254_740_992;

/// A setting key: `[a-z0-9_]{1,48}` (also used for groups, matrix rows and columns, list item fields).
pub fn is_key(s: &str) -> bool {
    (1..=MAX_KEY).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A role name in a `roles` value: `[a-z0-9_-]{1,40}` (`owner`, `admin`, `mod`, `everyone` or a group of a
/// permissions mod).
pub fn is_role(s: &str) -> bool {
    crate::rules::is_slug(s, 1, 40)
}

/// The type of a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingType {
    /// `true` / `false`.
    Bool,
    /// An integer (within ±2^53).
    Int,
    /// A number.
    Float,
    /// One of `values`.
    Enum,
    /// Text of at most `max_length` characters.
    String,
    /// A list of role names.
    Roles,
    /// `{row: {col: cell}}`.
    Matrix,
    /// A list of records with the fields of `item`.
    List,
}

impl SettingType {
    /// Every type, in the order of the docs.
    pub const ALL: [SettingType; 8] = [
        SettingType::Bool,
        SettingType::Int,
        SettingType::Float,
        SettingType::Enum,
        SettingType::String,
        SettingType::Roles,
        SettingType::Matrix,
        SettingType::List,
    ];

    /// The name in `mod.toml` and JSON.
    pub fn name(self) -> &'static str {
        match self {
            SettingType::Bool => "bool",
            SettingType::Int => "int",
            SettingType::Float => "float",
            SettingType::Enum => "enum",
            SettingType::String => "string",
            SettingType::Roles => "roles",
            SettingType::Matrix => "matrix",
            SettingType::List => "list",
        }
    }

    /// Parses a type name.
    pub fn parse(s: &str) -> Option<SettingType> {
        SettingType::ALL.into_iter().find(|t| t.name() == s)
    }
}

/// Where a setting belongs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    /// Set by the server admin (host app, `/mod set`, the in-game admin screen). The default.
    #[default]
    Server,
    /// Set by each player for themselves (the loader's settings screen), stored on the client.
    Client,
}

impl Scope {
    /// The name in `mod.toml` and JSON.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Server => "server",
            Scope::Client => "client",
        }
    }
}

/// The cell type of a `matrix`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Cell {
    /// Every cell is `true` / `false` (the default).
    #[default]
    Bool,
    /// Every cell is a list of role names.
    Roles,
}

impl Cell {
    /// The name in `mod.toml` and JSON.
    pub fn name(self) -> &'static str {
        match self {
            Cell::Bool => "bool",
            Cell::Roles => "roles",
        }
    }
}

/// One validated setting (or one field of a `list` item).
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// `[a-z0-9_]{1,48}`.
    pub key: String,
    /// The type.
    pub ty: SettingType,
    /// The default value, valid and normalised.
    pub default: Json,
    /// Label per language (`en` always present).
    pub label: Localized,
    /// Help text per language; empty when there is none.
    pub help: Localized,
    /// Group key for the forms (top-level settings only).
    pub group: Option<String>,
    /// Where the setting belongs (top-level settings only; list item fields inherit).
    pub scope: Scope,
    /// True: a change takes effect only after a restart (the forms say so). Top-level settings only.
    pub restart: bool,
    /// `int` / `float`: smallest value.
    pub min: Option<f64>,
    /// `int` / `float`: largest value.
    pub max: Option<f64>,
    /// `int` / `float`: step of the form's slider or spinner (a hint, not enforced).
    pub step: Option<f64>,
    /// `enum`: the allowed values.
    pub values: Vec<String>,
    /// `string`: most characters.
    pub max_length: u64,
    /// `matrix`: row keys.
    pub rows: Vec<String>,
    /// `matrix`: column keys.
    pub cols: Vec<String>,
    /// `matrix`: cell type.
    pub cell: Cell,
    /// `list`: the fields of one record, in declaration order.
    pub item: Vec<Field>,
    /// `list`: most records.
    pub max_items: u64,
}

impl Field {
    /// A field of type `ty` with every optional property at its default (handy for tests and hosts).
    pub fn new(key: &str, ty: SettingType, default: Json) -> Field {
        Field {
            key: key.to_string(),
            ty,
            default,
            label: Localized::from([("en".to_string(), key.to_string())]),
            help: Localized::new(),
            group: None,
            scope: Scope::Server,
            restart: false,
            min: None,
            max: None,
            step: None,
            values: Vec::new(),
            max_length: DEFAULT_MAX_LENGTH,
            rows: Vec::new(),
            cols: Vec::new(),
            cell: Cell::Bool,
            item: Vec::new(),
            max_items: DEFAULT_MAX_ITEMS,
        }
    }
}

/// `[settings]` of a manifest: the valid fields in declaration order. Problems found while reading are reported
/// by [`Manifest::validate`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// The settings in declaration order.
    pub fields: Vec<Field>,
    issues: Vec<Issue>,
}

impl Settings {
    /// True when the mod declares no settings.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.issues.is_empty()
    }

    /// The setting `key`.
    pub fn get(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// The problems found while reading `[settings]`.
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// Every default, by key.
    pub fn defaults(&self) -> Map<String, Json> {
        self.fields.iter().map(|f| (f.key.clone(), f.default.clone())).collect()
    }

    /// The values a host runs with, from the `stored` ones (`mods.toml`): every key of the schema, a stored valid
    /// value or else the default. The second list names what was ignored: unknown keys and invalid values (these
    /// fall back to the default).
    pub fn resolve(&self, stored: &Map<String, Json>) -> (Map<String, Json>, Vec<SettingsError>) {
        let mut problems = Vec::new();
        let mut out = Map::new();
        for f in &self.fields {
            let v = match stored.get(&f.key) {
                Some(v) => validate_value(f, v).unwrap_or_else(|e| {
                    problems.push(e);
                    f.default.clone()
                }),
                None => f.default.clone(),
            };
            out.insert(f.key.clone(), v);
        }
        problems.extend(stored.keys().filter(|k| self.get(k).is_none()).map(|k| SettingsError::UnknownKey(k.clone())));
        (out, problems)
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Settings, D::Error> {
        let raw = RawFields::deserialize(d)?;
        let mut issues = Vec::new();
        let fields = read_fields(&mut issues, "settings", raw.0, false);
        if fields.len() > MAX_FIELDS {
            issues.push(Issue::new("settings", format!("at most {MAX_FIELDS} settings")));
        }
        Ok(Settings { fields, issues })
    }
}

/// A label or help text: one English string or a table per language.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Text {
    Plain(String),
    Localized(Localized),
}

/// The settings (or item fields) as written, keyed by spanned names so the declaration order survives.
#[derive(Debug, Deserialize)]
struct RawFields(std::collections::BTreeMap<Spanned<String>, RawField>);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    #[serde(rename = "type")]
    ty: String,
    default: Option<toml::Value>,
    label: Option<Text>,
    help: Option<Text>,
    group: Option<String>,
    scope: Option<String>,
    restart: Option<bool>,
    min: Option<toml::Value>,
    max: Option<toml::Value>,
    step: Option<toml::Value>,
    values: Option<Vec<String>>,
    max_length: Option<i64>,
    rows: Option<Vec<String>>,
    cols: Option<Vec<String>>,
    cell: Option<String>,
    item: Option<RawFields>,
    max_items: Option<i64>,
}

fn read_fields(
    issues: &mut Vec<Issue>,
    at: &str,
    raw: std::collections::BTreeMap<Spanned<String>, RawField>,
    nested: bool,
) -> Vec<Field> {
    let mut raw: Vec<_> = raw.into_iter().collect();
    raw.sort_by_key(|(k, _)| k.span().start);
    raw.into_iter()
        .filter_map(|(k, r)| read_field(issues, &format!("{at}.{}", k.get_ref()), k.get_ref(), r, nested))
        .collect()
}

/// A label or help text as a language map, checked.
fn text(issues: &mut Vec<Issue>, at: &str, t: Option<Text>, required: bool, max: usize) -> Localized {
    let map = match t {
        None if required => {
            issues.push(Issue::new(at, "missing (a text, or a table with an `en` text)"));
            return Localized::new();
        }
        None => return Localized::new(),
        Some(Text::Plain(s)) => Localized::from([("en".to_string(), s)]),
        Some(Text::Localized(m)) => m,
    };
    if !map.contains_key("en") {
        issues.push(Issue::new(at, "needs an `en` text"));
    }
    for (lang, s) in &map {
        if !is_lang(lang) {
            issues.push(Issue::new(at, format!("`{lang}` is not a language code")));
        }
        if s.trim().is_empty() || s.chars().count() > max {
            issues.push(Issue::new(at, format!("`{lang}` text must be 1-{max} characters")));
        }
    }
    map
}

/// A number property (`min`, `max`, `step`) for an `int` (integers only) or `float` setting.
fn number(issues: &mut Vec<Issue>, at: &str, v: Option<toml::Value>, int: bool) -> Option<f64> {
    match v? {
        toml::Value::Integer(i) if i.abs() <= MAX_SAFE_INT => Some(i as f64),
        toml::Value::Float(f) if !int && f.is_finite() => Some(f),
        _ => {
            let want = if int { "an integer within ±2^53" } else { "a finite number" };
            issues.push(Issue::new(at, format!("must be {want}")));
            None
        }
    }
}

/// A list of distinct keys (`rows`, `cols`).
fn keys(issues: &mut Vec<Issue>, at: &str, v: Option<Vec<String>>) -> Vec<String> {
    let Some(v) = v else {
        issues.push(Issue::new(at, "missing"));
        return Vec::new();
    };
    let unique: BTreeSet<_> = v.iter().collect();
    if v.is_empty() || v.len() > MAX_ENTRIES || unique.len() != v.len() || !v.iter().all(|k| is_key(k)) {
        issues.push(Issue::new(at, format!("must list 1-{MAX_ENTRIES} distinct keys matching [a-z0-9_]{{1,48}}")));
    }
    v
}

fn read_field(issues: &mut Vec<Issue>, at: &str, key: &str, r: RawField, nested: bool) -> Option<Field> {
    let before = issues.len();
    if !is_key(key) {
        issues.push(Issue::new(at, "key must match [a-z0-9_]{1,48}"));
    }
    let Some(ty) = SettingType::parse(&r.ty) else {
        let names: Vec<_> = SettingType::ALL.iter().map(|t| t.name()).collect();
        issues.push(Issue::new(format!("{at}.type"), format!("must be one of {}", names.join(", "))));
        return None;
    };
    let mut f = Field::new(key, ty, Json::Null);
    f.label = text(issues, &format!("{at}.label"), r.label, true, MAX_LABEL);
    f.help = text(issues, &format!("{at}.help"), r.help, false, MAX_HELP);
    if nested {
        if ty == SettingType::List {
            issues.push(Issue::new(format!("{at}.type"), "a list item cannot hold a list"));
        }
        for (name, given) in
            [("group", r.group.is_some()), ("scope", r.scope.is_some()), ("restart", r.restart.is_some())]
        {
            if given {
                issues.push(Issue::new(at, format!("`{name}` is only for top-level settings")));
            }
        }
    }
    if let Some(g) = r.group {
        if !is_key(&g) {
            issues.push(Issue::new(format!("{at}.group"), "must match [a-z0-9_]{1,48}"));
        }
        f.group = Some(g);
    }
    match r.scope.as_deref() {
        None | Some("server") => {}
        Some("client") => f.scope = Scope::Client,
        Some(_) => issues.push(Issue::new(format!("{at}.scope"), "must be server or client")),
    }
    f.restart = r.restart.unwrap_or(false);
    let numeric = matches!(ty, SettingType::Int | SettingType::Float);
    let only = |issues: &mut Vec<Issue>, name: &str, given: bool, ok: bool, types: &str| {
        if given && !ok {
            issues.push(Issue::new(at, format!("`{name}` is only for {types} settings")));
        }
    };
    only(issues, "min", r.min.is_some(), numeric, "int and float");
    only(issues, "max", r.max.is_some(), numeric, "int and float");
    only(issues, "step", r.step.is_some(), numeric, "int and float");
    only(issues, "values", r.values.is_some(), ty == SettingType::Enum, "enum");
    only(issues, "max_length", r.max_length.is_some(), ty == SettingType::String, "string");
    let matrix = ty == SettingType::Matrix;
    only(issues, "rows", r.rows.is_some(), matrix, "matrix");
    only(issues, "cols", r.cols.is_some(), matrix, "matrix");
    only(issues, "cell", r.cell.is_some(), matrix, "matrix");
    only(issues, "item", r.item.is_some(), ty == SettingType::List, "list");
    only(issues, "max_items", r.max_items.is_some(), ty == SettingType::List, "list");
    match ty {
        SettingType::Int | SettingType::Float => {
            let int = ty == SettingType::Int;
            f.min = number(issues, &format!("{at}.min"), r.min, int);
            f.max = number(issues, &format!("{at}.max"), r.max, int);
            f.step = number(issues, &format!("{at}.step"), r.step, int);
            if f.step.is_some_and(|s| s <= 0.0) {
                issues.push(Issue::new(format!("{at}.step"), "must be greater than 0"));
            }
            if let (Some(a), Some(b)) = (f.min, f.max) {
                if a > b {
                    issues.push(Issue::new(at, "min is greater than max"));
                }
            }
        }
        SettingType::Enum => {
            f.values = r.values.unwrap_or_default();
            let unique: BTreeSet<_> = f.values.iter().collect();
            let bad = f
                .values
                .iter()
                .any(|v| v.trim().is_empty() || v.chars().count() > 64 || v.chars().any(char::is_control));
            if f.values.is_empty() || f.values.len() > MAX_ENTRIES || unique.len() != f.values.len() || bad {
                issues.push(Issue::new(
                    format!("{at}.values"),
                    format!("must list 1-{MAX_ENTRIES} distinct texts of 1-64 characters"),
                ));
            }
        }
        SettingType::String => match r.max_length {
            None => {}
            Some(n) if (1..=MAX_STRING as i64).contains(&n) => f.max_length = n as u64,
            Some(_) => issues.push(Issue::new(format!("{at}.max_length"), format!("must be 1-{MAX_STRING}"))),
        },
        SettingType::Matrix => {
            f.rows = keys(issues, &format!("{at}.rows"), r.rows);
            f.cols = keys(issues, &format!("{at}.cols"), r.cols);
            match r.cell.as_deref() {
                None | Some("bool") => {}
                Some("roles") => f.cell = Cell::Roles,
                Some(_) => issues.push(Issue::new(format!("{at}.cell"), "must be bool or roles")),
            }
        }
        SettingType::List => {
            match r.item {
                Some(item) if !item.0.is_empty() => {
                    f.item = read_fields(issues, &format!("{at}.item"), item.0, true);
                    if f.item.len() > MAX_ITEM_FIELDS {
                        issues.push(Issue::new(format!("{at}.item"), format!("at most {MAX_ITEM_FIELDS} fields")));
                    }
                }
                _ => issues.push(Issue::new(format!("{at}.item"), "missing (a table of at least one field)")),
            }
            match r.max_items {
                None => {}
                Some(n) if (1..=MAX_ITEMS as i64).contains(&n) => f.max_items = n as u64,
                Some(_) => issues.push(Issue::new(format!("{at}.max_items"), format!("must be 1-{MAX_ITEMS}"))),
            }
        }
        SettingType::Bool | SettingType::Roles => {}
    }
    if issues.len() > before {
        return None;
    }
    let Some(default) = r.default else {
        issues.push(Issue::new(format!("{at}.default"), "missing"));
        return None;
    };
    let Some(default) = toml_to_json(&default) else {
        issues.push(Issue::new(format!("{at}.default"), "dates and non-finite numbers are not setting values"));
        return None;
    };
    match validate_value(&f, &default) {
        Ok(v) => f.default = v,
        Err(e) => {
            issues.push(Issue::new(format!("{at}.default"), e.to_string()));
            return None;
        }
    }
    Some(f)
}

/// TOML to JSON; `None` for dates and non-finite floats (they are never setting values).
pub fn toml_to_json(v: &toml::Value) -> Option<Json> {
    Some(match v {
        toml::Value::String(s) => Json::String(s.clone()),
        toml::Value::Integer(i) => Json::from(*i),
        toml::Value::Float(f) => Json::Number(serde_json::Number::from_f64(*f)?),
        toml::Value::Boolean(b) => Json::Bool(*b),
        toml::Value::Datetime(_) => return None,
        toml::Value::Array(a) => Json::Array(a.iter().map(toml_to_json).collect::<Option<_>>()?),
        toml::Value::Table(t) => {
            Json::Object(t.iter().map(|(k, v)| Some((k.clone(), toml_to_json(v)?))).collect::<Option<_>>()?)
        }
    })
}

/// Why a value does not fit its setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// The mod has no setting with this key.
    UnknownKey(String),
    /// The value does not fit: where (`key`, `key.row.col`, `key[2].name`) and why.
    Invalid {
        /// Path of the bad part of the value.
        path: String,
        /// What is wrong.
        message: String,
    },
}

impl SettingsError {
    fn invalid(path: &str, message: impl Into<String>) -> SettingsError {
        SettingsError::Invalid { path: path.to_string(), message: message.into() }
    }

    /// A stable code: `unknown-key` or `invalid-value` (what `SETTINGS_RESULT` and the host app show).
    pub fn code(&self) -> &'static str {
        match self {
            SettingsError::UnknownKey(_) => "unknown-key",
            SettingsError::Invalid { .. } => "invalid-value",
        }
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::UnknownKey(k) => write!(f, "no setting `{k}`"),
            SettingsError::Invalid { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for SettingsError {}

/// Checks `value` against `field` and returns it normalised (see the module docs).
pub fn validate_value(field: &Field, value: &Json) -> Result<Json, SettingsError> {
    check(field, value, &field.key)
}

fn type_name(v: &Json) -> &'static str {
    match v {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::String(_) => "a text",
        Json::Array(_) => "a list",
        Json::Object(_) => "a table",
    }
}

fn expected(path: &str, what: &str, v: &Json) -> SettingsError {
    SettingsError::invalid(path, format!("expected {what}, got {}", type_name(v)))
}

fn in_range(f: &Field, path: &str, n: f64) -> Result<(), SettingsError> {
    let fmt = |x: f64| if f.ty == SettingType::Int { format!("{}", x as i64) } else { format!("{x}") };
    if let Some(min) = f.min.filter(|&m| n < m) {
        return Err(SettingsError::invalid(path, format!("must be at least {}", fmt(min))));
    }
    if let Some(max) = f.max.filter(|&m| n > m) {
        return Err(SettingsError::invalid(path, format!("must be at most {}", fmt(max))));
    }
    Ok(())
}

fn roles(path: &str, v: &Json) -> Result<Json, SettingsError> {
    let Json::Array(items) = v else { return Err(expected(path, "a list of role names", v)) };
    if items.len() > MAX_ENTRIES {
        return Err(SettingsError::invalid(path, format!("at most {MAX_ENTRIES} roles")));
    }
    let mut seen = BTreeSet::new();
    for (i, r) in items.iter().enumerate() {
        let at = format!("{path}[{i}]");
        let Json::String(s) = r else { return Err(expected(&at, "a role name", r)) };
        if !is_role(s) {
            return Err(SettingsError::invalid(&at, "a role name matches [a-z0-9_-]{1,40}"));
        }
        if !seen.insert(s) {
            return Err(SettingsError::invalid(&at, format!("`{s}` is listed twice")));
        }
    }
    Ok(v.clone())
}

/// The entries of a table that must have exactly `keys`.
fn exact<'a>(path: &str, v: &'a Json, keys: &[String]) -> Result<&'a Map<String, Json>, SettingsError> {
    let Json::Object(m) = v else { return Err(expected(path, "a table", v)) };
    if let Some(k) = m.keys().find(|k| !keys.contains(k)) {
        return Err(SettingsError::invalid(path, format!("unknown entry `{k}`")));
    }
    if let Some(k) = keys.iter().find(|k| !m.contains_key(*k)) {
        return Err(SettingsError::invalid(path, format!("missing entry `{k}`")));
    }
    Ok(m)
}

fn check(f: &Field, v: &Json, path: &str) -> Result<Json, SettingsError> {
    match f.ty {
        SettingType::Bool => v.as_bool().map(Json::Bool).ok_or_else(|| expected(path, "true or false", v)),
        SettingType::Int => {
            let n = match v {
                Json::Number(n) => n.as_i64().or_else(|| {
                    n.as_f64().filter(|x| x.fract() == 0.0 && x.abs() <= MAX_SAFE_INT as f64).map(|x| x as i64)
                }),
                _ => None,
            }
            .filter(|n| n.abs() <= MAX_SAFE_INT)
            .ok_or_else(|| expected(path, "an integer", v))?;
            in_range(f, path, n as f64)?;
            Ok(Json::from(n))
        }
        SettingType::Float => {
            let n = v.as_f64().ok_or_else(|| expected(path, "a number", v))?;
            in_range(f, path, n)?;
            Ok(Json::from(n))
        }
        SettingType::Enum => match v {
            Json::String(s) if f.values.contains(s) => Ok(v.clone()),
            _ => Err(SettingsError::invalid(path, format!("must be one of {}", f.values.join(", ")))),
        },
        SettingType::String => match v {
            Json::String(s) if s.chars().count() as u64 <= f.max_length => Ok(v.clone()),
            Json::String(_) => Err(SettingsError::invalid(path, format!("at most {} characters", f.max_length))),
            _ => Err(expected(path, "a text", v)),
        },
        SettingType::Roles => roles(path, v),
        SettingType::Matrix => {
            let m = exact(path, v, &f.rows)?;
            let mut out = Map::new();
            for row in &f.rows {
                let at = format!("{path}.{row}");
                let cells = exact(&at, &m[row], &f.cols)?;
                let mut r = Map::new();
                for col in &f.cols {
                    let at = format!("{at}.{col}");
                    let c = &cells[col];
                    let c = match f.cell {
                        Cell::Bool => c.as_bool().map(Json::Bool).ok_or_else(|| expected(&at, "true or false", c))?,
                        Cell::Roles => roles(&at, c)?,
                    };
                    r.insert(col.clone(), c);
                }
                out.insert(row.clone(), Json::Object(r));
            }
            Ok(Json::Object(out))
        }
        SettingType::List => {
            let Json::Array(items) = v else { return Err(expected(path, "a list", v)) };
            if items.len() as u64 > f.max_items {
                return Err(SettingsError::invalid(path, format!("at most {} entries", f.max_items)));
            }
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let at = format!("{path}[{i}]");
                let Json::Object(m) = item else { return Err(expected(&at, "a table", item)) };
                if let Some(k) = m.keys().find(|k| !f.item.iter().any(|x| &x.key == *k)) {
                    return Err(SettingsError::invalid(&at, format!("unknown field `{k}`")));
                }
                let mut rec = Map::new();
                for sub in &f.item {
                    let val = match m.get(&sub.key) {
                        Some(x) => check(sub, x, &format!("{at}.{}", sub.key))?,
                        None => sub.default.clone(),
                    };
                    rec.insert(sub.key.clone(), val);
                }
                out.push(Json::Object(rec));
            }
            Ok(Json::Array(out))
        }
    }
}

/// Parses what an admin typed (`/mod set <id> <key> <value>`) for `field` and validates it: `bool` takes
/// `true/false/on/off/yes/no/1/0`, `int` and `float` a number, `enum` and `string` the text as it is, `roles` a
/// comma-separated list or a JSON list, `matrix` and `list` JSON.
pub fn parse_input(field: &Field, text: &str) -> Result<Json, SettingsError> {
    let t = text.trim();
    let bad = |what: &str| SettingsError::invalid(&field.key, format!("expected {what}"));
    let value = match field.ty {
        SettingType::Bool => match t.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => Json::Bool(true),
            "false" | "off" | "no" | "0" => Json::Bool(false),
            _ => return Err(bad("true or false")),
        },
        SettingType::Int => Json::from(t.parse::<i64>().map_err(|_| bad("an integer"))?),
        SettingType::Float => {
            let n = t.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| bad("a number"))?;
            Json::from(n)
        }
        SettingType::Enum | SettingType::String => Json::String(text.to_string()),
        SettingType::Roles if t.starts_with('[') => {
            serde_json::from_str(t).map_err(|_| bad("a JSON list of role names"))?
        }
        SettingType::Roles => Json::Array(
            t.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| Json::String(s.to_string())).collect(),
        ),
        SettingType::Matrix | SettingType::List => serde_json::from_str(t).map_err(|e| bad(&format!("JSON ({e})")))?,
    };
    validate_value(field, &value)
}

fn localized_json(m: &Localized) -> Json {
    Json::Object(m.iter().map(|(k, v)| (k.clone(), Json::String(v.clone()))).collect())
}

fn number_json(f: &Field, n: f64) -> Json {
    if f.ty == SettingType::Int {
        Json::from(n as i64)
    } else {
        Json::from(n)
    }
}

/// One field as canonical JSON. Item fields (`nested`) leave out `group`, `scope` and `restart`.
pub fn field_json(f: &Field, nested: bool) -> Json {
    let mut o = Map::new();
    o.insert("key".into(), Json::String(f.key.clone()));
    o.insert("type".into(), Json::String(f.ty.name().into()));
    o.insert("default".into(), f.default.clone());
    o.insert("label".into(), localized_json(&f.label));
    if !f.help.is_empty() {
        o.insert("help".into(), localized_json(&f.help));
    }
    if !nested {
        if let Some(g) = &f.group {
            o.insert("group".into(), Json::String(g.clone()));
        }
        o.insert("scope".into(), Json::String(f.scope.name().into()));
        o.insert("restart".into(), Json::Bool(f.restart));
    }
    for (name, n) in [("min", f.min), ("max", f.max), ("step", f.step)] {
        if let Some(n) = n {
            o.insert(name.into(), number_json(f, n));
        }
    }
    let strings = |v: &[String]| Json::Array(v.iter().cloned().map(Json::String).collect());
    match f.ty {
        SettingType::Enum => {
            o.insert("values".into(), strings(&f.values));
        }
        SettingType::String => {
            o.insert("max_length".into(), Json::from(f.max_length));
        }
        SettingType::Matrix => {
            o.insert("rows".into(), strings(&f.rows));
            o.insert("cols".into(), strings(&f.cols));
            o.insert("cell".into(), Json::String(f.cell.name().into()));
        }
        SettingType::List => {
            o.insert("item".into(), Json::Array(f.item.iter().map(|x| field_json(x, true)).collect()));
            o.insert("max_items".into(), Json::from(f.max_items));
        }
        _ => {}
    }
    Json::Object(o)
}

/// The canonical JSON of a mod's settings: `{ "fields": [...], "values": {...} }`, fields in declaration order,
/// `values` with every key (the given value, else the default; values are not validated here).
pub fn schema_json(m: &Manifest, values: &Map<String, Json>) -> Json {
    let fields = m.settings.fields.iter().map(|f| field_json(f, false)).collect();
    let values =
        m.settings.fields.iter().map(|f| (f.key.clone(), values.get(&f.key).unwrap_or(&f.default).clone())).collect();
    let mut o = Map::new();
    o.insert("fields".into(), Json::Array(fields));
    o.insert("values".into(), Json::Object(values));
    Json::Object(o)
}

#[cfg(test)]
mod tests;
