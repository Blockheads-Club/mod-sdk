//! Project templates for `bhmod new`.

const LUA_MOD: &str = include_str!("../templates/lua/mod.toml");
const LUA_MAIN: &str = include_str!("../templates/lua/main.luau");
const LUA_README: &str = include_str!("../templates/lua/README.md");
const RUST_MOD: &str = include_str!("../templates/rust/mod.toml");
const RUST_CARGO: &str = include_str!("../templates/rust/Cargo.toml.tmpl");
const RUST_LIB: &str = include_str!("../templates/rust/lib.rs");
const RUST_BUILD: &str = include_str!("../templates/rust/build.sh");
const RUST_GITIGNORE: &str = include_str!("../templates/rust/gitignore");
const RUST_README: &str = include_str!("../templates/rust/README.md");

const JAVA_MOD: &str = include_str!("../templates/java/mod.toml");
const JAVA_MAIN: &str = include_str!("../templates/java/Main.java");
const JAVA_BUILD: &str = include_str!("../templates/java/build.sh");
const JAVA_GRADLE: &str = include_str!("../templates/java/build.gradle.kts");
const JAVA_GITIGNORE: &str = include_str!("../templates/java/gitignore");
const JAVA_README: &str = include_str!("../templates/java/README.md");

/// The `blockheads:api` WIT files, copied into Rust projects.
pub const WIT: [(&str, &str); 15] = [
    ("channels.wit", include_str!("../../../wit/blockheads-api/channels.wit")),
    ("commands.wit", include_str!("../../../wit/blockheads-api/commands.wit")),
    ("content.wit", include_str!("../../../wit/blockheads-api/content.wit")),
    ("events.wit", include_str!("../../../wit/blockheads-api/events.wit")),
    ("guest.wit", include_str!("../../../wit/blockheads-api/guest.wit")),
    ("host.wit", include_str!("../../../wit/blockheads-api/host.wit")),
    ("log.wit", include_str!("../../../wit/blockheads-api/log.wit")),
    ("players.wit", include_str!("../../../wit/blockheads-api/players.wit")),
    ("scheduler.wit", include_str!("../../../wit/blockheads-api/scheduler.wit")),
    ("services.wit", include_str!("../../../wit/blockheads-api/services.wit")),
    ("settings.wit", include_str!("../../../wit/blockheads-api/settings.wit")),
    ("storage.wit", include_str!("../../../wit/blockheads-api/storage.wit")),
    ("types.wit", include_str!("../../../wit/blockheads-api/types.wit")),
    ("ui.wit", include_str!("../../../wit/blockheads-api/ui.wit")),
    ("worlds.wit", include_str!("../../../wit/blockheads-api/worlds.wit")),
];

fn fill(template: &str, id: &str) -> String {
    // Cargo package names may not start with a digit.
    let package = if id.starts_with(|c: char| c.is_ascii_digit()) { format!("mod-{id}") } else { id.to_string() };
    // Java package names are lower-case identifiers: `-` becomes `_`, and a leading digit gets a prefix.
    let javapkg = format!("mod_{}", id.replace('-', "_"));
    template
        .replace("{{id}}", id)
        .replace("{{package}}", &package)
        .replace("{{crate}}", &package.replace('-', "_"))
        .replace("{{javapkg}}", &javapkg)
}

/// Files of a new Luau mod.
pub fn lua(id: &str) -> Vec<(String, String)> {
    vec![
        ("mod.toml".into(), fill(LUA_MOD, id)),
        ("server/main.luau".into(), fill(LUA_MAIN, id)),
        ("README.md".into(), fill(LUA_README, id)),
    ]
}

/// Files of a new Rust (WASM component) mod.
pub fn rust(id: &str) -> Vec<(String, String)> {
    let mut files = vec![
        ("mod.toml".into(), fill(RUST_MOD, id)),
        ("Cargo.toml".into(), fill(RUST_CARGO, id)),
        ("src/lib.rs".into(), fill(RUST_LIB, id)),
        ("build.sh".into(), fill(RUST_BUILD, id)),
        (".gitignore".into(), RUST_GITIGNORE.to_string()),
        ("README.md".into(), fill(RUST_README, id)),
    ];
    files.extend(WIT.iter().map(|(name, text)| (format!("bindings/blockheads-api/{name}"), text.to_string())));
    files
}

/// Files of a new Java (JVM) mod.
pub fn java(id: &str) -> Vec<(String, String)> {
    let pkg = format!("mod_{}", id.replace('-', "_"));
    vec![
        ("mod.toml".into(), fill(JAVA_MOD, id)),
        (format!("src/main/java/{pkg}/Main.java"), fill(JAVA_MAIN, id)),
        ("build.sh".into(), fill(JAVA_BUILD, id)),
        ("build.gradle.kts".into(), fill(JAVA_GRADLE, id)),
        (".gitignore".into(), JAVA_GITIGNORE.to_string()),
        ("README.md".into(), fill(JAVA_README, id)),
    ]
}
