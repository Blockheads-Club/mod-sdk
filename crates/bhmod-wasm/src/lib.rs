// bhmod-wasm: the WASM component runtime of the Blockheads Club mod host.
// Copyright (C) 2026 Blockheads Club contributors
//
// This program is free software: you can redistribute it and/or modify it under the terms of the GNU General
// Public License as published by the Free Software Foundation, either version 3 of the License, or (at your
// option) any later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the
// implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
// more details.
//
// You should have received a copy of the GNU General Public License along with this program. If not, see
// <https://www.gnu.org/licenses/>.
//
// Additional permission under GNU GPL version 3 section 7: the "Blockheads Club Mod API Exception, version 1.0"
// in LICENSING.md at the root of the mod-sdk repository applies to this program.

//! The WASM runtime of the Blockheads mod host: [`WasmRuntime`] implements [`bhmod_core::Runtime`] for
//! `runtime = "wasm"` mods, which are WebAssembly **components** of the `blockheads:api` `server-mod` /
//! `client-mod` world (see `wit/` and `docs/wasm.md`).
//!
//! - wasmtime with Cranelift on x86_64 / aarch64 / riscv64 / s390x; on every other host (armv7, i686, ...) the
//!   Pulley interpreter, whose bytecode Cranelift also emits.
//! - Time: epoch interruption against [`HostApi::deadline`](bhmod_core::HostApi::deadline) →
//!   [`CallError::Timeout`](bhmod_core::CallError::Timeout); fuel is off.
//! - Memory: `InstanceLimits.memory_bytes` caps the linear memories of the instance (`StoreLimits`).
//! - WASI p2 is linked with an empty context (no files, network, environment or arguments) so `wasm32-wasip2`
//!   components work; stdout and stderr go to the mod's log.
//! - Compiled code is cached as `.cwasm` in [`WasmConfig::cache_dir`], keyed by the component's SHA-256 and the
//!   engine's compatibility hash. With [`WasmConfig::trust_precompiled`] a package may ship
//!   `<entry>.<target>.cwasm` (for example `server/mod.pulley32.cwasm`, made by [`precompile_for`]) so 32-bit
//!   devices skip the compile.
#![warn(missing_docs)]

pub mod bindings;
mod instance;
mod state;
mod ticker;
pub mod wit;

#[cfg(feature = "compiler")]
use std::collections::hash_map::DefaultHasher;
#[cfg(feature = "compiler")]
use std::hash::{Hash, Hasher};
#[cfg(feature = "compiler")]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "compiler")]
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine};

use bhmod_core::runtime::{Instance, InstanceLimits, ModFiles, Runtime};

pub use instance::{WasmInstance, INSTANTIATE_BUDGET};
pub use ticker::TICK;

use state::State;
use ticker::Ticker;

/// Which code generator runs the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// Native code where Cranelift supports the host, Pulley elsewhere.
    #[default]
    Auto,
    /// Native code through Cranelift, even in builds without the `compiler` feature (precompiled code only); Pulley
    /// on hosts Cranelift has no backend for.
    Native,
    /// The Pulley interpreter (`pulley32` / `pulley64` matching the host).
    Pulley,
}

/// True when Cranelift can emit native code for this host.
pub const NATIVE_HOST: bool =
    cfg!(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64", target_arch = "s390x"));

/// The Pulley target of this host (`pulley32`, `pulley64`, with `be` on big-endian hosts).
pub const HOST_PULLEY: &str = if cfg!(target_pointer_width = "64") {
    if cfg!(target_endian = "big") {
        "pulley64be"
    } else {
        "pulley64"
    }
} else if cfg!(target_endian = "big") {
    "pulley32be"
} else {
    "pulley32"
};

/// Options of a [`WasmRuntime`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasmConfig {
    /// The code generator.
    pub backend: Backend,
    /// Where compiled components are cached (`<sha256>-<engine-hash>.cwasm`); `None` compiles on every load.
    pub cache_dir: Option<PathBuf>,
    /// Load `<entry stem>.<target>.cwasm` from the package when present. Precompiled code is trusted like native
    /// code (a forged artifact can break the sandbox), so only enable this for packages you trust.
    pub trust_precompiled: bool,
}

/// The engine configuration shared by the runtime and [`precompile_for`] (precompiled code must match it).
fn base_config(target: Option<&str>) -> Result<Config, String> {
    let mut c = Config::new();
    c.wasm_component_model(true);
    c.epoch_interruption(true);
    c.consume_fuel(false);
    if let Some(t) = target {
        c.target(t).map_err(text)?;
    }
    Ok(c)
}

/// The text of a wasmtime error with its causes.
fn text(e: wasmtime::Error) -> String {
    format!("{e:#}")
}

fn engine_for(target: Option<&str>) -> Result<Engine, String> {
    Engine::new(&base_config(target)?).map_err(text)
}

/// Compiles a component for another target, for example `pulley32` on a build machine for armv7 devices. The
/// result loads on a host whose runtime targets the same (see [`WasmRuntime::target`]).
#[cfg(feature = "compiler")]
pub fn precompile_for(target: &str, wasm: &[u8]) -> Result<Vec<u8>, String> {
    engine_for(Some(target))?.precompile_component(wasm).map_err(text)
}

/// The WASM component runtime.
pub struct WasmRuntime {
    engine: Engine,
    linker: Linker<State>,
    ticker: Arc<Ticker>,
    target: String,
    config: WasmConfig,
}

impl WasmRuntime {
    /// Creates the engine and links the `blockheads:api` imports and WASI.
    pub fn new(config: WasmConfig) -> Result<WasmRuntime, String> {
        let native = match config.backend {
            Backend::Auto => NATIVE_HOST && cfg!(feature = "compiler"),
            Backend::Native => NATIVE_HOST,
            Backend::Pulley => false,
        };
        let target = if native { None } else { Some(HOST_PULLEY) };
        let engine = engine_for(target)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(text)?;
        bindings::v03::add_to_linker(&mut linker).map_err(text)?;
        bindings::v02::add_to_linker(&mut linker).map_err(text)?;
        bindings::v01::add_to_linker(&mut linker).map_err(text)?;
        let ticker = Ticker::start(engine.clone());
        let target = target.map_or_else(|| std::env::consts::ARCH.to_string(), str::to_string);
        Ok(WasmRuntime { engine, linker, ticker, target, config })
    }

    /// What the engine compiles for: a CPU architecture (`x86_64`) or a Pulley target (`pulley32`). Precompiled
    /// artifacts in packages are named after it.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The engine's compatibility hash (hex); compiled code only loads into an engine with the same hash.
    #[cfg(feature = "compiler")]
    pub fn engine_hash(&self) -> String {
        let mut h = DefaultHasher::new();
        self.engine.precompile_compatibility_hash().hash(&mut h);
        format!("{:016x}", h.finish())
    }

    /// Compiles a component for this engine (the bytes of a `.cwasm`).
    #[cfg(feature = "compiler")]
    pub fn precompile(&self, wasm: &[u8]) -> Result<Vec<u8>, String> {
        self.engine.precompile_component(wasm).map_err(text)
    }

    /// The cache file of a component.
    #[cfg(feature = "compiler")]
    pub fn cache_path(&self, wasm: &[u8]) -> Option<PathBuf> {
        let dir = self.config.cache_dir.as_ref()?;
        let sha: String = Sha256::digest(wasm).iter().map(|b| format!("{b:02x}")).collect();
        Some(dir.join(format!("{sha}-{}.cwasm", self.engine_hash())))
    }

    /// Loads the component of `files`: a trusted precompiled artifact, the cache, or a fresh compile.
    pub fn load(&self, files: &ModFiles) -> Result<Component, String> {
        let Some(wasm) = files.files.get(&files.entry) else {
            return Err(format!("entry point {} is missing", files.entry));
        };
        match self.precompiled(files) {
            Some(c) => Ok(c),
            None => self.compile(wasm),
        }
    }

    /// The package's precompiled artifact for this target, when trusted, present and loadable.
    fn precompiled(&self, files: &ModFiles) -> Option<Component> {
        if !self.config.trust_precompiled {
            return None;
        }
        let stem = files.entry.strip_suffix(".wasm").unwrap_or(&files.entry);
        let bytes = files.files.get(&format!("{stem}.{}.cwasm", self.target))?;
        // SAFETY: the operator opted into trusting precompiled artifacts of installed packages.
        unsafe { Component::deserialize(&self.engine, bytes) }.ok()
    }

    #[cfg(feature = "compiler")]
    fn compile(&self, wasm: &[u8]) -> Result<Component, String> {
        let Some(path) = self.cache_path(wasm) else {
            return Component::new(&self.engine, wasm).map_err(text);
        };
        // SAFETY: the cache directory belongs to the host and holds only files this runtime wrote.
        if let Ok(c) = unsafe { Component::deserialize_file(&self.engine, &path) } {
            return Ok(c);
        }
        let bytes = self.precompile(wasm)?;
        write_cache(&path, &bytes);
        // SAFETY: the bytes were produced by this engine a moment ago.
        unsafe { Component::deserialize(&self.engine, &bytes) }.map_err(text)
    }

    #[cfg(not(feature = "compiler"))]
    fn compile(&self, _wasm: &[u8]) -> Result<Component, String> {
        Err(format!("this host cannot compile WASM; the package needs a trusted precompiled `.{}.cwasm`", self.target))
    }
}

/// Writes a cache file atomically; a cache that cannot be written only costs a compile next time.
#[cfg(feature = "compiler")]
fn write_cache(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    let ok = path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&tmp, bytes).is_ok();
    if !ok || std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

impl Runtime for WasmRuntime {
    fn kind(&self) -> bhmod_manifest::manifest::Runtime {
        bhmod_manifest::manifest::Runtime::Wasm
    }

    fn instantiate(&self, mod_id: &str, files: &ModFiles, limits: InstanceLimits) -> Result<Box<dyn Instance>, String> {
        let inst = self.load(files).and_then(|c| WasmInstance::new(&self.linker, &c, self.ticker.clone(), limits));
        Ok(Box::new(inst.map_err(|e| format!("{mod_id}: {e}"))?))
    }
}
