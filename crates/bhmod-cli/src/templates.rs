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

/// The `blockheads:api` WIT files, copied into Rust projects.
pub const WIT: [(&str, &str); 12] = [
    ("channels.wit", include_str!("../../../wit/blockheads-api/channels.wit")),
    ("commands.wit", include_str!("../../../wit/blockheads-api/commands.wit")),
    ("events.wit", include_str!("../../../wit/blockheads-api/events.wit")),
    ("guest.wit", include_str!("../../../wit/blockheads-api/guest.wit")),
    ("host.wit", include_str!("../../../wit/blockheads-api/host.wit")),
    ("log.wit", include_str!("../../../wit/blockheads-api/log.wit")),
    ("players.wit", include_str!("../../../wit/blockheads-api/players.wit")),
    ("scheduler.wit", include_str!("../../../wit/blockheads-api/scheduler.wit")),
    ("services.wit", include_str!("../../../wit/blockheads-api/services.wit")),
    ("storage.wit", include_str!("../../../wit/blockheads-api/storage.wit")),
    ("types.wit", include_str!("../../../wit/blockheads-api/types.wit")),
    ("worlds.wit", include_str!("../../../wit/blockheads-api/worlds.wit")),
];

fn fill(template: &str, id: &str) -> String {
    // Cargo package names may not start with a digit.
    let package = if id.starts_with(|c: char| c.is_ascii_digit()) { format!("mod-{id}") } else { id.to_string() };
    template.replace("{{id}}", id).replace("{{package}}", &package).replace("{{crate}}", &package.replace('-', "_"))
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
