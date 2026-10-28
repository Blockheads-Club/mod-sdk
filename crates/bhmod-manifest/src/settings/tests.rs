use serde_json::json;

use super::*;

const BASE: &str = "schema = 1\nid = \"demo\"\nversion = \"1.0.0\"\nname = { en = \"Demo\" }\nauthors = [\"me\"]\n\
license = \"MIT\"\napi = \"^0.3\"\n[sides]\nserver = true\n[entry]\nserver = { runtime = \"lua\", path = \"server/main.luau\" }\n";

fn manifest(settings: &str) -> Result<Manifest, Vec<Issue>> {
    Manifest::parse(&format!("{BASE}{settings}"))
}

fn fields(settings: &str) -> Vec<Field> {
    match manifest(settings) {
        Ok(m) => m.settings.fields,
        Err(e) => panic!("rejected: {e:?}"),
    }
}

fn errors(settings: &str) -> Vec<String> {
    match manifest(settings) {
        Ok(m) => panic!("accepted: {:?}", m.settings),
        Err(e) => e.iter().map(ToString::to_string).collect(),
    }
}

#[track_caller]
fn has(settings: &str, needle: &str) {
    let errs = errors(settings);
    assert!(errs.iter().any(|e| e.contains(needle)), "no `{needle}` in {errs:?}");
}

const ALL: &str = r#"
[settings.zeta]
type = "bool"
default = true
label = "Zeta"

[settings.radius]
type = "int"
default = 32
min = 4
max = 128
step = 4
label = { en = "Radius", tr = "Yarıçap" }
help = "Blocks"
group = "proximity"
restart = true

[settings.gain]
type = "float"
default = 1
min = 0
max = 2.5
step = 0.1
label = "Gain"
scope = "client"

[settings.curve]
type = "enum"
values = ["lin", "log"]
default = "log"
label = "Curve"

[settings.motd]
type = "string"
default = "hi"
max_length = 10
label = "Motd"

[settings.who]
type = "roles"
default = ["admin", "mod"]
label = "Who"

[settings.talk]
type = "matrix"
rows = ["global", "local"]
cols = ["admin", "everyone"]
default = { global = { admin = true, everyone = false }, local = { admin = true, everyone = true } }
label = "Talk"

[settings.access]
type = "matrix"
rows = ["a"]
cols = ["b"]
cell = "roles"
default = { a = { b = ["owner"] } }
label = "Access"

[settings.channels]
type = "list"
max_items = 3
default = [{ name = "main" }]
label = "Channels"
item = { name = { type = "string", default = "", label = "Name", max_length = 8 }, open = { type = "bool", default = true, label = "Open", help = { en = "Anyone may join" } } }
"#;

#[test]
fn every_type_in_declaration_order() {
    let m = manifest(ALL).unwrap();
    let s = &m.settings;
    let keys: Vec<_> = s.fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["zeta", "radius", "gain", "curve", "motd", "who", "talk", "access", "channels"]);
    assert!(!s.is_empty() && s.issues().is_empty());
    assert!(Settings::default().is_empty());
    let r = s.get("radius").unwrap();
    assert_eq!((r.min, r.max, r.step, r.restart), (Some(4.0), Some(128.0), Some(4.0), true));
    assert_eq!(r.group.as_deref(), Some("proximity"));
    assert_eq!(r.label["tr"], "Yarıçap");
    assert_eq!(r.help["en"], "Blocks");
    let g = s.get("gain").unwrap();
    assert_eq!(g.default, json!(1.0), "an integer default of a float is a float");
    assert_eq!(g.scope, Scope::Client);
    assert_eq!(s.get("access").unwrap().cell, Cell::Roles);
    assert_eq!(s.get("talk").unwrap().cell, Cell::Bool);
    let ch = s.get("channels").unwrap();
    assert_eq!(ch.default, json!([{ "name": "main", "open": true }]), "missing item fields get their default");
    assert_eq!(ch.item.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), ["name", "open"]);
    assert!(s.get("nope").is_none());
    assert_eq!(s.defaults()["motd"], json!("hi"));
}

#[test]
fn canonical_json() {
    let m = manifest(ALL).unwrap();
    let mut values = Map::new();
    values.insert("zeta".into(), json!(false));
    let j = schema_json(&m, &values);
    assert_eq!(j["values"]["zeta"], json!(false));
    assert_eq!(j["values"]["radius"], json!(32));
    let f = &j["fields"];
    assert_eq!(f.as_array().unwrap().len(), 9);
    assert_eq!(
        f[0],
        json!({ "key": "zeta", "type": "bool", "default": true, "label": { "en": "Zeta" }, "scope": "server", "restart": false })
    );
    assert_eq!(
        f[1],
        json!({ "key": "radius", "type": "int", "default": 32, "label": { "en": "Radius", "tr": "Yarıçap" },
            "help": { "en": "Blocks" }, "group": "proximity", "scope": "server", "restart": true, "min": 4, "max": 128, "step": 4 })
    );
    assert_eq!(f[2]["min"], json!(0.0));
    assert_eq!(f[2]["max"], json!(2.5));
    assert_eq!(f[2]["scope"], json!("client"));
    assert_eq!(f[3]["values"], json!(["lin", "log"]));
    assert_eq!(f[4]["max_length"], json!(10));
    assert_eq!(f[6]["rows"], json!(["global", "local"]));
    assert_eq!(f[6]["cols"], json!(["admin", "everyone"]));
    assert_eq!(f[6]["cell"], json!("bool"));
    assert_eq!(f[8]["max_items"], json!(3));
    assert_eq!(
        f[8]["item"],
        json!([
            { "key": "name", "type": "string", "default": "", "label": { "en": "Name" }, "max_length": 8 },
            { "key": "open", "type": "bool", "default": true, "label": { "en": "Open" }, "help": { "en": "Anyone may join" } }
        ])
    );
    let none = manifest("").unwrap();
    assert_eq!(schema_json(&none, &Map::new()), json!({ "fields": [], "values": {} }));
}

#[test]
fn values_are_validated_and_normalised() {
    let fs = fields(ALL);
    let f = |k: &str| fs.iter().find(|f| f.key == k).unwrap();
    let ok = |k: &str, v: Json| validate_value(f(k), &v).unwrap();
    let err = |k: &str, v: Json| validate_value(f(k), &v).unwrap_err().to_string();
    assert_eq!(ok("zeta", json!(false)), json!(false));
    assert!(err("zeta", json!(1)).contains("expected true or false, got a number"));
    assert_eq!(ok("radius", json!(8)), json!(8));
    assert_eq!(ok("radius", json!(8.0)), json!(8), "a whole float is an integer");
    assert!(err("radius", json!(8.5)).contains("expected an integer"));
    assert!(err("radius", json!("8")).contains("got a text"));
    assert!(err("radius", json!(1)).contains("at least 4"));
    assert!(err("radius", json!(500)).contains("at most 128"));
    let mut big = f("radius").clone();
    big.min = None;
    big.max = None;
    assert!(validate_value(&big, &json!(MAX_SAFE_INT + 1)).is_err());
    assert!(validate_value(&big, &json!(1e300)).is_err());
    assert_eq!(validate_value(&big, &json!(-MAX_SAFE_INT)).unwrap(), json!(-MAX_SAFE_INT));
    assert_eq!(ok("gain", json!(2)), json!(2.0));
    assert!(err("gain", json!(3)).contains("at most 2.5"));
    assert!(err("gain", json!(-0.5)).contains("at least 0"));
    assert!(err("gain", json!(null)).contains("got null"));
    assert_eq!(ok("curve", json!("lin")), json!("lin"));
    assert!(err("curve", json!("cubic")).contains("one of lin, log"));
    assert!(err("curve", json!(1)).contains("one of"));
    assert_eq!(ok("motd", json!("ten chars!")), json!("ten chars!"));
    assert!(err("motd", json!("eleven char")).contains("at most 10 characters"));
    assert!(err("motd", json!(true)).contains("got a boolean"));
    assert_eq!(ok("who", json!([])), json!([]));
    assert!(err("who", json!("admin")).contains("list of role names"));
    assert!(err("who", json!(["Admin"])).contains("who[0]"));
    assert!(err("who", json!([1])).contains("who[0]: expected a role name"));
    assert!(err("who", json!(["a", "a"])).contains("listed twice"));
    let many: Vec<_> = (0..65).map(|i| format!("r{i}")).collect();
    assert!(err("who", json!(many)).contains("at most 64 roles"));
    let talk = json!({ "global": { "admin": false, "everyone": false }, "local": { "admin": true, "everyone": true } });
    assert_eq!(ok("talk", talk.clone()), talk);
    assert!(err("talk", json!([])).contains("expected a table, got a list"));
    assert!(err("talk", json!({ "global": {}, "local": {}, "x": {} })).contains("unknown entry `x`"));
    assert!(err("talk", json!({ "global": {} })).contains("missing entry `local`"));
    assert!(err(
        "talk",
        json!({ "global": { "admin": 1, "everyone": true }, "local": { "admin": true, "everyone": true } })
    )
    .contains("talk.global.admin"));
    assert!(err("access", json!({ "a": { "b": "owner" } })).contains("access.a.b"));
    assert_eq!(ok("access", json!({ "a": { "b": [] } })), json!({ "a": { "b": [] } }));
    assert_eq!(ok("channels", json!([])), json!([]));
    assert!(err("channels", json!({})).contains("expected a list"));
    assert!(err("channels", json!([{}, {}, {}, {}])).contains("at most 3 entries"));
    assert!(err("channels", json!([1])).contains("channels[0]: expected a table"));
    assert!(err("channels", json!([{ "x": 1 }])).contains("unknown field `x`"));
    assert!(err("channels", json!([{ "name": "much too long" }])).contains("channels[0].name"));
    let e = validate_value(f("channels"), &json!(5)).unwrap_err();
    assert_eq!(e.code(), "invalid-value");
    assert_eq!(SettingsError::UnknownKey("k".into()).code(), "unknown-key");
    assert_eq!(SettingsError::UnknownKey("k".into()).to_string(), "no setting `k`");
}

#[test]
fn resolve_falls_back_to_defaults() {
    let m = manifest(ALL).unwrap();
    let mut stored = Map::new();
    stored.insert("radius".into(), json!(8));
    stored.insert("motd".into(), json!(5));
    stored.insert("ghost".into(), json!(1));
    let (values, problems) = m.settings.resolve(&stored);
    assert_eq!(values.len(), 9);
    assert_eq!(values["radius"], json!(8));
    assert_eq!(values["motd"], json!("hi"));
    assert_eq!(problems.len(), 2);
    assert!(problems[0].to_string().contains("motd"));
    assert_eq!(problems[1], SettingsError::UnknownKey("ghost".into()));
}

#[test]
fn typed_input() {
    let fs = fields(ALL);
    let f = |k: &str| fs.iter().find(|f| f.key == k).unwrap();
    let ok = |k: &str, t: &str| parse_input(f(k), t).unwrap();
    let err = |k: &str, t: &str| parse_input(f(k), t).unwrap_err().to_string();
    for (t, v) in [
        ("true", true),
        ("ON", true),
        ("yes", true),
        ("1", true),
        ("false", false),
        ("off", false),
        ("no", false),
        ("0", false),
    ] {
        assert_eq!(ok("zeta", t), json!(v));
    }
    assert!(err("zeta", "maybe").contains("true or false"));
    assert_eq!(ok("radius", " 16 "), json!(16));
    assert!(err("radius", "x").contains("an integer"));
    assert!(err("radius", "2").contains("at least 4"));
    assert_eq!(ok("gain", "0.5"), json!(0.5));
    assert!(err("gain", "inf").contains("a number"));
    assert!(err("gain", "x").contains("a number"));
    assert_eq!(ok("curve", "lin"), json!("lin"));
    assert_eq!(ok("motd", "a b"), json!("a b"));
    assert_eq!(ok("who", "admin, mod,,"), json!(["admin", "mod"]));
    assert_eq!(ok("who", "[\"owner\"]"), json!(["owner"]));
    assert!(err("who", "[oops").contains("JSON list"));
    assert_eq!(ok("access", r#"{"a":{"b":["x"]}}"#), json!({ "a": { "b": ["x"] } }));
    assert!(err("access", "{").contains("expected JSON"));
    assert_eq!(ok("channels", r#"[{"name":"a"}]"#), json!([{ "name": "a", "open": true }]));
}

#[test]
fn toml_values() {
    let v: toml::Value = toml::from_str("a = [1, 2.5, \"x\", true, { b = 1 }]").unwrap();
    assert_eq!(toml_to_json(&v), Some(json!({ "a": [1, 2.5, "x", true, { "b": 1 }] })));
    let d: toml::Value = toml::from_str("a = 1979-05-27").unwrap();
    assert_eq!(toml_to_json(&d), None);
    assert_eq!(toml_to_json(&toml::Value::Float(f64::NAN)), None);
}

#[test]
fn names() {
    assert!(is_key("a_1") && !is_key("A") && !is_key("a-b") && !is_key("") && !is_key(&"a".repeat(49)));
    assert!(is_role("everyone") && is_role("vip-1") && !is_role("VIP") && !is_role(""));
    for t in SettingType::ALL {
        assert_eq!(SettingType::parse(t.name()), Some(t));
    }
    assert_eq!(SettingType::parse("map"), None);
    assert_eq!(Scope::default(), Scope::Server);
    assert_eq!(Scope::Client.name(), "client");
    assert_eq!(Cell::default(), Cell::Bool);
    assert_eq!(Cell::Roles.name(), "roles");
    let f = Field::new("k", SettingType::Bool, json!(true));
    assert_eq!(f.label["en"], "k");
    assert_eq!(field_json(&f, true), json!({ "key": "k", "type": "bool", "default": true, "label": { "en": "k" } }));
}

#[test]
fn bad_declarations() {
    let b = |extra: &str| format!("[settings.x]\ntype = \"bool\"\ndefault = true\nlabel = \"X\"\n{extra}");
    let i = |extra: &str| format!("[settings.x]\ntype = \"int\"\ndefault = 1\nlabel = \"X\"\n{extra}");
    has("[settings.Bad]\ntype = \"bool\"\ndefault = true\nlabel = \"X\"\n", "settings.Bad: key must match");
    has("[settings.x]\ntype = \"map\"\ndefault = 1\nlabel = \"X\"\n", "settings.x.type: must be one of bool, int");
    has("[settings.x]\ntype = \"bool\"\ndefault = true\n", "settings.x.label: missing");
    has(&b("help = { tr = \"Y\" }"), "settings.x.help: needs an `en` text");
    has(&b("help = { en = \"Y\", EN = \"Z\" }"), "`EN` is not a language code");
    has(&b("help = \"  \""), "`en` text must be 1-1024 characters");
    has(&b("group = \"G\""), "settings.x.group: must match");
    has(&b("scope = \"world\""), "settings.x.scope: must be server or client");
    for key in ["min = 1", "max = 1", "step = 1"] {
        has(&b(key), "is only for int and float settings");
    }
    has(&b("values = [\"a\"]"), "`values` is only for enum settings");
    has(&b("max_length = 3"), "`max_length` is only for string settings");
    for key in ["rows = [\"a\"]", "cols = [\"a\"]", "cell = \"bool\""] {
        has(&b(key), "is only for matrix settings");
    }
    has(&b("item = { a = { type = \"bool\", default = true, label = \"A\" } }"), "`item` is only for list settings");
    has(&b("max_items = 3"), "`max_items` is only for list settings");
    has(&b("bogus = 1"), "unknown field `bogus`");
    has(&i("min = 1.5"), "settings.x.min: must be an integer");
    has(&i("max = 9007199254740993"), "settings.x.max: must be an integer");
    has(&i("step = 0"), "settings.x.step: must be greater than 0");
    has(&i("min = 5\nmax = 1"), "min is greater than max");
    has("[settings.x]\ntype = \"float\"\ndefault = 1\nlabel = \"X\"\nmin = \"a\"\n", "must be a finite number");
    has("[settings.x]\ntype = \"float\"\ndefault = 1\nlabel = \"X\"\nmax = nan\n", "must be a finite number");
    let e =
        |values: &str| format!("[settings.x]\ntype = \"enum\"\ndefault = \"a\"\nlabel = \"X\"\nvalues = {values}\n");
    has(&e("[]"), "settings.x.values: must list");
    has(&e("[\"a\", \"a\"]"), "settings.x.values: must list");
    has(&e("[\"a\", \" \"]"), "settings.x.values: must list");
    has("[settings.x]\ntype = \"enum\"\ndefault = \"a\"\nlabel = \"X\"\n", "settings.x.values: must list");
    let s = |n: &str| format!("[settings.x]\ntype = \"string\"\ndefault = \"\"\nlabel = \"X\"\nmax_length = {n}\n");
    has(&s("0"), "settings.x.max_length: must be 1-65536");
    has(&s("65537"), "settings.x.max_length: must be 1-65536");
    let m = |rest: &str| format!("[settings.x]\ntype = \"matrix\"\ndefault = {{}}\nlabel = \"X\"\n{rest}");
    has(&m("cols = [\"a\"]"), "settings.x.rows: missing");
    has(&m("rows = [\"a\"]\ncols = []"), "settings.x.cols: must list");
    has(&m("rows = [\"a\", \"a\"]\ncols = [\"b\"]"), "settings.x.rows: must list");
    has(&m("rows = [\"A\"]\ncols = [\"b\"]"), "settings.x.rows: must list");
    has(&m("rows = [\"a\"]\ncols = [\"b\"]\ncell = \"int\""), "settings.x.cell: must be bool or roles");
    has(&m("rows = [\"a\"]\ncols = [\"b\"]"), "settings.x.default: x: missing entry `a`");
    let l = |rest: &str| format!("[settings.x]\ntype = \"list\"\ndefault = []\nlabel = \"X\"\n{rest}");
    has(&l(""), "settings.x.item: missing");
    has(&l("item = {}"), "settings.x.item: missing");
    has(
        &l("item = { a = { type = \"bool\", default = true, label = \"A\" } }\nmax_items = 0"),
        "max_items: must be 1-1000",
    );
    has(&l("item = { a = { type = \"list\", default = [], label = \"A\", item = { b = { type = \"bool\", default = true, label = \"B\" } } } }"),
        "settings.x.item.a.type: a list item cannot hold a list");
    has(&l("item = { a = { type = \"bool\", default = true, label = \"A\", group = \"g\", scope = \"server\", restart = true } }"),
        "`group` is only for top-level settings");
    has(&l("item = { a = { type = \"bool\", label = \"A\" } }"), "settings.x.item.a.default: missing");
    let many: String =
        (0..33).map(|i| format!("f{i} = {{ type = \"bool\", default = true, label = \"F\" }}, ")).collect();
    has(&l(&format!("item = {{ {} }}", many.trim_end_matches(", "))), "settings.x.item: at most 32 fields");
    has("[settings.x]\ntype = \"bool\"\nlabel = \"X\"\n", "settings.x.default: missing");
    has("[settings.x]\ntype = \"string\"\nlabel = \"X\"\ndefault = 1979-05-27\n", "dates and non-finite numbers");
    has(
        "[settings.x]\ntype = \"int\"\nlabel = \"X\"\ndefault = 1\nmin = 2\n",
        "settings.x.default: x: must be at least 2",
    );
    let lots: String =
        (0..129).map(|i| format!("[settings.s{i}]\ntype = \"bool\"\ndefault = true\nlabel = \"S\"\n")).collect();
    has(&lots, "settings: at most 128 settings");
}
