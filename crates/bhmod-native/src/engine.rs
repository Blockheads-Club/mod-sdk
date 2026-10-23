//! The hook engine behind the `libbhmod` C ABI, independent of where the game runs.
//!
//! A [`Host`] gives the engine memory access, symbol lookup, executable memory for trampolines and the Objective-C
//! runtime. The Android loader implements it over the process (`mprotect`, `dlsym`, `method_setImplementation`);
//! the PC runner implements it over the guest (guest memory writes plus a JIT invalidation, the linker's exports,
//! the same runtime functions called in the guest). Everything else is shared, so a mod behaves the same on both.

use std::collections::BTreeMap;
use std::fmt;

use crate::abi::Code;
use crate::arm::{self, RelocError, MAX_TRAMPOLINE, PATCH_LEN};
use crate::elf;

/// A loaded module as the engine needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    /// File name (`libApplication.so`) or path.
    pub name: String,
    /// Load address.
    pub base: u32,
    /// Address of the `PT_DYNAMIC` segment in memory.
    pub dynamic: u32,
}

/// What the engine needs from the process that runs the game.
pub trait Host {
    /// Reads `len` bytes of game memory.
    fn read(&self, addr: u32, len: usize) -> Option<Vec<u8>>;
    /// Overwrites data in game memory (a GOT slot): the host makes the page writable for the moment.
    fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), String>;
    /// Overwrites code (an entry patch, a trampoline): like [`Host::write`], and the host flushes instruction caches
    /// (or invalidates its JIT). Kept apart from data writes because a host may have to leave code pages writable
    /// and executable (a binary translator that tracks code pages).
    fn write_code(&mut self, addr: u32, bytes: &[u8]) -> Result<(), String>;
    /// `len` bytes of executable, writable memory aligned to 4 (trampolines).
    fn alloc_exec(&mut self, len: usize) -> Result<u32, String>;
    /// The loaded modules.
    fn modules(&self) -> Vec<ModuleInfo>;
    /// An exported symbol of `module` (all modules when `None`), like `dlsym`.
    fn symbol(&self, module: Option<&str>, name: &str) -> Option<u32>;
    /// The `Method` of `-[class selector]` (or `+[...]` when `meta`), `None` when the class or method is missing.
    fn objc_method(&mut self, class: &str, selector: &str, meta: bool) -> Option<u32>;
    /// `method_setImplementation`: sets the method's IMP and returns the old one.
    fn objc_set_imp(&mut self, method: u32, imp: u32) -> Option<u32>;
}

/// A failed hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookError {
    /// The code returned to C.
    pub code: Code,
    /// A sentence for `last_error` and the log.
    pub message: String,
}

impl HookError {
    fn new(code: Code, message: impl Into<String>) -> HookError {
        HookError { code, message: message.into() }
    }
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HookError {}

/// A successful hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hooked {
    /// Positive handle for `unhook`.
    pub handle: u32,
    /// The function to call for the original behaviour: a trampoline (inline), the previous IMP (Objective-C) or
    /// the previous slot value (import).
    pub original: u32,
}

enum Kind {
    Inline { addr: u32, saved: [u8; PATCH_LEN] },
    Import { slots: Vec<(u32, u32)> },
    Objc { method: u32, old: u32 },
}

struct Record {
    owner: u32,
    kind: Kind,
}

/// All hooks of one process. `owner` is the mod index the host assigns; `unhook_owner` undoes a mod's hooks.
#[derive(Default)]
pub struct Engine {
    hooks: BTreeMap<u32, Record>,
    next: u32,
    last_error: String,
}

fn module_matches(m: &ModuleInfo, wanted: &str) -> bool {
    m.name == wanted || m.name.rsplit('/').next() == Some(wanted)
}

impl Engine {
    /// An engine without hooks.
    pub fn new() -> Engine {
        Engine::default()
    }

    /// The message of the last failure.
    pub fn last_error(&self) -> &str {
        &self.last_error
    }

    /// Number of live hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// True when nothing is hooked.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    fn fail<T>(&mut self, code: Code, message: impl Into<String>) -> Result<T, HookError> {
        let e = HookError::new(code, message);
        self.last_error = e.message.clone();
        Err(e)
    }

    fn add(&mut self, owner: u32, kind: Kind, original: u32) -> Hooked {
        self.next += 1;
        self.hooks.insert(self.next, Record { owner, kind });
        Hooked { handle: self.next, original }
    }

    /// Replaces the method `-[class selector]` (`+[...]` when `meta`) with `replacement`.
    pub fn hook_objc(
        &mut self,
        host: &mut dyn Host,
        owner: u32,
        class: &str,
        selector: &str,
        meta: bool,
        replacement: u32,
    ) -> Result<Hooked, HookError> {
        if class.is_empty() || selector.is_empty() || replacement == 0 {
            return self.fail(Code::Arg, "hook_objc: class, selector and replacement are required");
        }
        let Some(method) = host.objc_method(class, selector, meta) else {
            return self
                .fail(Code::NotFound, format!("no method {}[{class} {selector}]", if meta { '+' } else { '-' }));
        };
        let Some(old) = host.objc_set_imp(method, replacement) else {
            return self.fail(Code::Memory, format!("cannot replace {class} {selector}"));
        };
        Ok(self.add(owner, Kind::Objc { method, old }, old))
    }

    /// Inline-hooks the exported function `symbol` of `module` (any module when `None`).
    pub fn hook_symbol(
        &mut self,
        host: &mut dyn Host,
        owner: u32,
        module: Option<&str>,
        symbol: &str,
        replacement: u32,
    ) -> Result<Hooked, HookError> {
        if symbol.is_empty() {
            return self.fail(Code::Arg, "hook_symbol: a symbol name is required");
        }
        let Some(addr) = host.symbol(module, symbol) else {
            return self.fail(Code::NotFound, format!("no symbol {symbol}"));
        };
        self.inline(host, owner, addr, replacement)
    }

    /// Inline-hooks the ARM-mode code at `offset` bytes into `module`.
    pub fn hook_address(
        &mut self,
        host: &mut dyn Host,
        owner: u32,
        module: &str,
        offset: u32,
        replacement: u32,
    ) -> Result<Hooked, HookError> {
        let Some(m) = host.modules().into_iter().find(|m| module_matches(m, module)) else {
            return self.fail(Code::NotFound, format!("module {module} is not loaded"));
        };
        self.inline(host, owner, m.base.wrapping_add(offset), replacement)
    }

    fn inline(&mut self, host: &mut dyn Host, owner: u32, addr: u32, replacement: u32) -> Result<Hooked, HookError> {
        if replacement == 0 {
            return self.fail(Code::Arg, "the replacement is null");
        }
        if addr % 4 != 0 {
            return self.fail(Code::Thumb, format!("{addr:#x} is not ARM-mode code (Thumb is not supported)"));
        }
        if self.hooks.values().any(|h| matches!(h.kind, Kind::Inline { addr: a, .. } if a == addr)) {
            return self.fail(Code::Exists, format!("{addr:#x} is already hooked"));
        }
        let Some(code) = host.read(addr, PATCH_LEN).and_then(|b| <[u8; PATCH_LEN]>::try_from(b).ok()) else {
            return self.fail(Code::Memory, format!("cannot read {addr:#x}"));
        };
        let tramp = match host.alloc_exec(MAX_TRAMPOLINE) {
            Ok(t) => t,
            Err(e) => return self.fail(Code::Memory, format!("no executable memory: {e}")),
        };
        let bytes = match arm::trampoline(code, addr, tramp, &|a, n| host.read(a, n)) {
            Ok(b) => b,
            Err(e @ RelocError::Unaligned(_)) => return self.fail(Code::Thumb, e.to_string()),
            Err(e) => return self.fail(Code::Reloc, e.to_string()),
        };
        if let Err(e) = host.write_code(tramp, &bytes) {
            return self.fail(Code::Memory, format!("cannot write the trampoline: {e}"));
        }
        if let Err(e) = host.write_code(addr, &arm::entry_patch(replacement)) {
            return self.fail(Code::Memory, format!("cannot patch {addr:#x}: {e}"));
        }
        Ok(self.add(owner, Kind::Inline { addr, saved: code }, tramp))
    }

    /// Redirects the import `symbol` of `module` (every module when `None`) to `replacement`.
    pub fn hook_import(
        &mut self,
        host: &mut dyn Host,
        owner: u32,
        module: Option<&str>,
        symbol: &str,
        replacement: u32,
    ) -> Result<Hooked, HookError> {
        if symbol.is_empty() || replacement == 0 {
            return self.fail(Code::Arg, "hook_import: a symbol name and a replacement are required");
        }
        let modules: Vec<ModuleInfo> =
            host.modules().into_iter().filter(|m| module.is_none_or(|w| module_matches(m, w))).collect();
        if modules.is_empty() {
            return self.fail(Code::NotFound, format!("module {} is not loaded", module.unwrap_or("*")));
        }
        let mut slots = Vec::new();
        for m in &modules {
            let imports = match elf::imports(&|a, n| host.read(a, n), m.base, m.dynamic) {
                Ok(i) => i,
                Err(_) => continue,
            };
            for i in imports.into_iter().filter(|i| i.name == symbol) {
                if let Some(old) = host.read(i.slot, 4).and_then(|b| <[u8; 4]>::try_from(b).ok()) {
                    slots.push((i.slot, u32::from_le_bytes(old)));
                }
            }
        }
        let Some(&(_, original)) = slots.first() else {
            return self.fail(Code::NotFound, format!("no module imports {symbol}"));
        };
        let mut done = Vec::new();
        for &(slot, old) in &slots {
            if let Err(e) = host.write(slot, &replacement.to_le_bytes()) {
                for (s, o) in done {
                    let _ = host.write(s, &u32::to_le_bytes(o));
                }
                return self.fail(Code::Memory, format!("cannot patch the slot at {slot:#x}: {e}"));
            }
            done.push((slot, old));
        }
        Ok(self.add(owner, Kind::Import { slots }, original))
    }

    /// Undoes one hook.
    pub fn unhook(&mut self, host: &mut dyn Host, handle: u32) -> Result<(), HookError> {
        let Some(rec) = self.hooks.remove(&handle) else {
            return self.fail(Code::Handle, format!("unknown hook handle {handle}"));
        };
        let failed = match rec.kind {
            Kind::Inline { addr, saved } => host.write_code(addr, &saved).is_err(),
            Kind::Import { slots } => {
                slots.iter().fold(false, |bad, &(s, o)| host.write(s, &o.to_le_bytes()).is_err() || bad)
            }
            Kind::Objc { method, old } => host.objc_set_imp(method, old).is_none(),
        };
        if failed {
            return self.fail(Code::Memory, format!("hook {handle} could not be restored"));
        }
        Ok(())
    }

    /// Undoes every hook of a mod (newest first). Returns how many could not be restored.
    pub fn unhook_owner(&mut self, host: &mut dyn Host, owner: u32) -> usize {
        let handles: Vec<u32> = self.hooks.iter().filter(|(_, r)| r.owner == owner).map(|(h, _)| *h).rev().collect();
        handles.into_iter().filter(|h| self.unhook(host, *h).is_err()).count()
    }
}
