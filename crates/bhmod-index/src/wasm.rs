//! WASM checks: the module or component must validate and import only the mod API and allowed WASI interfaces.

use wasmparser::{Parser, Payload, Validator};

/// Interface prefixes a component may import.
pub const COMPONENT_IMPORTS: [&str; 6] =
    ["blockheads:api/", "wasi:cli/", "wasi:io/", "wasi:clocks/", "wasi:random/", "wasi:filesystem/"];

/// Module names a core module (`bh-core-abi`) may import from.
pub const CORE_IMPORTS: [&str; 3] = ["bh", "env", "wasi_snapshot_preview1"];

/// Checks `bytes`; returns the problems found (empty = fine).
pub fn check(bytes: &[u8]) -> Vec<String> {
    if let Err(e) = Validator::new().validate_all(bytes) {
        return vec![format!("invalid WASM: {e}")];
    }
    let mut problems = Vec::new();
    let mut depth = 0usize;
    for payload in Parser::new(0).parse_all(bytes).flatten() {
        match payload {
            Payload::Version { .. } => depth += 1,
            Payload::End(_) => depth -= 1,
            Payload::ComponentImportSection(s) if depth == 1 => {
                for name in s.into_iter().flatten().map(|i| i.name.name) {
                    if !COMPONENT_IMPORTS.iter().any(|p| name.starts_with(p)) {
                        problems.push(format!("imports `{name}`, only blockheads:api and sandboxed WASI are allowed"));
                    }
                }
            }
            Payload::ImportSection(s) if depth == 1 => {
                for i in s.into_imports().flatten() {
                    if !CORE_IMPORTS.contains(&i.module) {
                        problems.push(format!(
                            "imports `{}.{}`, only {} are allowed",
                            i.module,
                            i.name,
                            CORE_IMPORTS.join(", ")
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    problems
}
