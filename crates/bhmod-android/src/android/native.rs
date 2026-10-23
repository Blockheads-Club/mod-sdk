//! Native mods on Android: the `libbhmod` host. The game process is the host, so the [`Host`] of the shared
//! [`Engine`] is plain system calls: `mprotect` + `cacheflush` to patch code, an anonymous executable arena for
//! trampolines, `dl_iterate_phdr`/`dlsym` for modules and symbols, the Objective-C runtime for swizzles.
//!
//! A mod's library is `dlopen`ed and `bhmod_native_init(api)` runs on the thread that loads the mods (the game's
//! frame thread). The `api` table is the C ABI of `include/bhmod.h`; every function takes the table as its first
//! argument, and the table lives inside a private [`Api`] that also knows the mod.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Mutex;

use bhmod_core::types::LogLevel;
use bhmod_native::abi::{self, Code};
use bhmod_native::engine::{Engine, Host, ModuleInfo};

use super::objc;

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i32) -> *mut c_void;
    fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    fn pipe(fds: *mut c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
    fn cacheflush(start: usize, end: usize, flags: usize) -> c_int;
    fn dl_iterate_phdr(cb: extern "C" fn(*mut PhdrInfo, usize, *mut c_void) -> c_int, data: *mut c_void) -> c_int;
}

const RTLD_NOW: c_int = 2;
const RTLD_NOLOAD: c_int = 4;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const PROT_EXEC: c_int = 4;
const MAP_PRIVATE: c_int = 2;
const MAP_ANONYMOUS: c_int = 0x20;
const PT_DYNAMIC: u32 = 2;
const PAGE: usize = 4096;
const ARENA: usize = 64 * 1024;

/// `struct dl_phdr_info` (bionic, 32-bit).
#[repr(C)]
struct PhdrInfo {
    addr: u32,
    name: *const c_char,
    phdr: *const Phdr,
    phnum: u16,
}

/// `Elf32_Phdr`.
#[repr(C)]
struct Phdr {
    kind: u32,
    offset: u32,
    vaddr: u32,
    paddr: u32,
    filesz: u32,
    memsz: u32,
    flags: u32,
    align: u32,
}

extern "C" fn collect_module(info: *mut PhdrInfo, _size: usize, out: *mut c_void) -> c_int {
    // SAFETY: bionic passes a valid info and our own vector.
    let (info, out) = unsafe { (&*info, &mut *(out as *mut Vec<ModuleInfo>)) };
    let name = if info.name.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(info.name) }.to_string_lossy().into_owned()
    };
    let phdrs = unsafe { std::slice::from_raw_parts(info.phdr, info.phnum as usize) };
    if let Some(d) = phdrs.iter().find(|p| p.kind == PT_DYNAMIC) {
        out.push(ModuleInfo { name, base: info.addr, dynamic: info.addr.wrapping_add(d.vaddr) });
    }
    0
}

/// The permissions of the mapping that holds `addr`, from `/proc/self/maps`.
fn protection_of(addr: usize) -> c_int {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
    for line in maps.lines() {
        let mut parts = line.split_whitespace();
        let (Some(range), Some(perms)) = (parts.next(), parts.next()) else { continue };
        let Some((a, b)) = range.split_once('-') else { continue };
        let (Ok(a), Ok(b)) = (usize::from_str_radix(a, 16), usize::from_str_radix(b, 16)) else { continue };
        if (a..b).contains(&addr) {
            let p = perms.as_bytes();
            return (if p[0] == b'r' { PROT_READ } else { 0 })
                | (if p[1] == b'w' { PROT_WRITE } else { 0 })
                | (if p[2] == b'x' { PROT_EXEC } else { 0 });
        }
    }
    PROT_READ | PROT_EXEC
}

/// The process as an engine [`Host`].
struct Process {
    arena: usize,
    arena_end: usize,
}

impl Process {
    fn dlsym_in(&self, module: &str, name: &str) -> Option<u32> {
        let m = CString::new(module).ok()?;
        let n = CString::new(name).ok()?;
        unsafe {
            let h = dlopen(m.as_ptr(), RTLD_NOW | RTLD_NOLOAD);
            if h.is_null() {
                return None;
            }
            let p = dlsym(h, n.as_ptr());
            (!p.is_null()).then_some(p as usize as u32)
        }
    }
}

impl Host for Process {
    fn read(&self, addr: u32, len: usize) -> Option<Vec<u8>> {
        probe_read(addr, len)
    }

    fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), String> {
        patch(addr, bytes, false)
    }

    fn write_code(&mut self, addr: u32, bytes: &[u8]) -> Result<(), String> {
        patch(addr, bytes, true)
    }

    fn alloc_exec(&mut self, len: usize) -> Result<u32, String> {
        let len = len.next_multiple_of(4);
        if self.arena + len > self.arena_end {
            let p = unsafe {
                mmap(
                    std::ptr::null_mut(),
                    ARENA,
                    PROT_READ | PROT_WRITE | PROT_EXEC,
                    MAP_PRIVATE | MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if p as isize == -1 {
                return Err(format!("mmap failed (errno {})", errno()));
            }
            self.arena = p as usize;
            self.arena_end = self.arena + ARENA;
        }
        let a = self.arena;
        self.arena += len;
        Ok(a as u32)
    }

    fn modules(&self) -> Vec<ModuleInfo> {
        let mut v: Vec<ModuleInfo> = Vec::new();
        unsafe { dl_iterate_phdr(collect_module, &mut v as *mut _ as *mut c_void) };
        v
    }

    fn symbol(&self, module: Option<&str>, name: &str) -> Option<u32> {
        match module {
            Some(m) => self.dlsym_in(m, name),
            None => self.modules().iter().find_map(|m| self.dlsym_in(&m.name, name)),
        }
    }

    fn objc_method(&mut self, class: &str, selector: &str, meta: bool) -> Option<u32> {
        let m = objc::method(class, selector, meta);
        (!m.is_null()).then_some(m as usize as u32)
    }

    fn objc_set_imp(&mut self, method: u32, imp: u32) -> Option<u32> {
        let old = objc::set_imp(method as usize as *mut c_void, imp as usize as objc::Imp);
        (!old.is_null()).then_some(old as usize as u32)
    }
}

/// Writes `bytes` at `addr`. The page is made writable (and executable for code) for the write. Afterwards it gets
/// its protection back, except a code page whose mapping was not executable to begin with: a binary translator (the
/// emulator's ARM translation shows the game's code as `r--p` and crashes when it is restored) keeps it RWX.
fn patch(addr: u32, bytes: &[u8], code: bool) -> Result<(), String> {
    let (start, end) = (addr as usize & !(PAGE - 1), (addr as usize + bytes.len()).next_multiple_of(PAGE));
    let original = protection_of(addr as usize);
    let open = if code { PROT_READ | PROT_WRITE | PROT_EXEC } else { PROT_READ | PROT_WRITE };
    unsafe {
        if mprotect(start as *mut c_void, end - start, open) != 0 {
            return Err(format!("mprotect failed (errno {})", errno()));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as usize as *mut u8, bytes.len());
        if code {
            cacheflush(addr as usize, addr as usize + bytes.len(), 0);
        }
        if !code || original & PROT_EXEC != 0 {
            mprotect(start as *mut c_void, end - start, original);
        }
    }
    Ok(())
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Reads `len` bytes at `addr` without crashing on bad memory: a write(2) from the address into a pipe fails with
/// EFAULT instead.
fn probe_read(addr: u32, len: usize) -> Option<Vec<u8>> {
    static PIPE: Mutex<Option<(c_int, c_int)>> = Mutex::new(None);
    if len > PAGE {
        return None;
    }
    let mut guard = PIPE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let mut fds = [0 as c_int; 2];
        if unsafe { pipe(fds.as_mut_ptr()) } != 0 {
            return None;
        }
        *guard = Some((fds[0], fds[1]));
    }
    let (r, w) = (*guard)?;
    let n = unsafe { write(w, addr as usize as *const c_void, len) };
    if n != len as isize {
        // Drain a short write so the pipe stays empty.
        if n > 0 {
            let mut sink = vec![0u8; n as usize];
            unsafe { read(r, sink.as_mut_ptr() as *mut c_void, n as usize) };
        }
        return None;
    }
    let mut out = vec![0u8; len];
    (unsafe { read(r, out.as_mut_ptr() as *mut c_void, len) } == len as isize).then_some(out)
}

// ---- the C table --------------------------------------------------------------------------------------------------

type LogFn = extern "C" fn(*const Api, c_int, *const c_char);
type HookObjcFn =
    extern "C" fn(*const Api, *const c_char, *const c_char, c_int, *mut c_void, *mut *mut c_void) -> c_int;
type HookNamedFn = extern "C" fn(*const Api, *const c_char, *const c_char, *mut c_void, *mut *mut c_void) -> c_int;
type HookAddrFn = extern "C" fn(*const Api, *const c_char, u32, *mut c_void, *mut *mut c_void) -> c_int;
type UnhookFn = extern "C" fn(*const Api, c_int) -> c_int;
type FindFn = extern "C" fn(*const Api, *const c_char, *const c_char) -> *mut c_void;
type BaseFn = extern "C" fn(*const Api, *const c_char) -> *mut c_void;
type ErrFn = extern "C" fn(*const Api) -> *const c_char;

/// `bhmod_api_t` (the first fields, as in `include/bhmod.h`) followed by what only the host sees.
#[repr(C)]
struct Api {
    abi: u32,
    size: u32,
    mod_id: *const c_char,
    data_dir: *const c_char,
    log: LogFn,
    hook_objc: HookObjcFn,
    hook_symbol: HookNamedFn,
    hook_address: HookAddrFn,
    hook_import: HookNamedFn,
    unhook: UnhookFn,
    find_symbol: FindFn,
    module_base: BaseFn,
    last_error: ErrFn,
    /// Engine owner index of the mod (not part of the ABI: after `size`).
    owner: u32,
}

struct NativeMod {
    id: String,
    api: *mut Api,
    libs: Vec<*mut c_void>,
}
// SAFETY: raw pointers to leaked, never freed memory and to dlopen handles; used under the STATE lock.
unsafe impl Send for NativeMod {}

struct State {
    engine: Engine,
    process: Process,
    mods: Vec<NativeMod>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(|| State {
        engine: Engine::new(),
        process: Process { arena: 0, arena_end: 0 },
        mods: Vec::new(),
    });
    f(s)
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::new(msg.replace('\0', " ")).unwrap_or_default());
}

fn text<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(p) }.to_str().ok()
    }
}

fn owner_of(api: *const Api) -> u32 {
    if api.is_null() {
        u32::MAX
    } else {
        unsafe { (*api).owner }
    }
}

fn guarded(what: &str, f: impl FnOnce() -> c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        super::log(LogLevel::Error, &format!("panic in libbhmod {what}"));
        Code::Memory.ret()
    })
}

fn finish(
    result: Result<bhmod_native::engine::Hooked, bhmod_native::engine::HookError>,
    original: *mut *mut c_void,
) -> c_int {
    match result {
        Ok(h) => {
            if !original.is_null() {
                unsafe { *original = h.original as usize as *mut c_void };
            }
            h.handle as c_int
        }
        Err(e) => {
            set_error(&e.message);
            e.code.ret()
        }
    }
}

fn bad_arg(what: &str) -> c_int {
    set_error(&format!("{what}: a required argument is null"));
    Code::Arg.ret()
}

extern "C" fn api_log(api: *const Api, level: c_int, message: *const c_char) {
    let Some(msg) = text(message) else { return };
    let id = if api.is_null() { "native" } else { text(unsafe { (*api).mod_id }).unwrap_or("native") };
    let level = match level {
        abi::LOG_TRACE => LogLevel::Trace,
        abi::LOG_DEBUG => LogLevel::Debug,
        abi::LOG_WARN => LogLevel::Warn,
        abi::LOG_ERROR => LogLevel::Error,
        _ => LogLevel::Info,
    };
    super::log(level, &format!("[{id}] {msg}"));
}

extern "C" fn api_hook_objc(
    api: *const Api,
    cls: *const c_char,
    sel: *const c_char,
    class_method: c_int,
    replacement: *mut c_void,
    original: *mut *mut c_void,
) -> c_int {
    guarded("hook_objc", || {
        let (Some(c), Some(s)) = (text(cls), text(sel)) else { return bad_arg("hook_objc") };
        let r = with_state(|st| {
            st.engine.hook_objc(&mut st.process, owner_of(api), c, s, class_method != 0, replacement as usize as u32)
        });
        finish(r, original)
    })
}

extern "C" fn api_hook_symbol(
    api: *const Api,
    module: *const c_char,
    symbol: *const c_char,
    replacement: *mut c_void,
    original: *mut *mut c_void,
) -> c_int {
    guarded("hook_symbol", || {
        let Some(s) = text(symbol) else { return bad_arg("hook_symbol") };
        let r = with_state(|st| {
            st.engine.hook_symbol(&mut st.process, owner_of(api), text(module), s, replacement as usize as u32)
        });
        finish(r, original)
    })
}

extern "C" fn api_hook_address(
    api: *const Api,
    module: *const c_char,
    offset: u32,
    replacement: *mut c_void,
    original: *mut *mut c_void,
) -> c_int {
    guarded("hook_address", || {
        let Some(m) = text(module) else { return bad_arg("hook_address") };
        let r = with_state(|st| {
            st.engine.hook_address(&mut st.process, owner_of(api), m, offset, replacement as usize as u32)
        });
        finish(r, original)
    })
}

extern "C" fn api_hook_import(
    api: *const Api,
    module: *const c_char,
    symbol: *const c_char,
    replacement: *mut c_void,
    original: *mut *mut c_void,
) -> c_int {
    guarded("hook_import", || {
        let Some(s) = text(symbol) else { return bad_arg("hook_import") };
        let r = with_state(|st| {
            st.engine.hook_import(&mut st.process, owner_of(api), text(module), s, replacement as usize as u32)
        });
        finish(r, original)
    })
}

extern "C" fn api_unhook(_api: *const Api, handle: c_int) -> c_int {
    guarded("unhook", || match with_state(|st| st.engine.unhook(&mut st.process, handle as u32)) {
        Ok(()) => 0,
        Err(e) => {
            set_error(&e.message);
            e.code.ret()
        }
    })
}

extern "C" fn api_find_symbol(_api: *const Api, module: *const c_char, symbol: *const c_char) -> *mut c_void {
    let Some(s) = text(symbol) else { return std::ptr::null_mut() };
    let found = catch_unwind(AssertUnwindSafe(|| with_state(|st| st.process.symbol(text(module), s)))).ok().flatten();
    found.map_or(std::ptr::null_mut(), |a| a as usize as *mut c_void)
}

extern "C" fn api_module_base(_api: *const Api, module: *const c_char) -> *mut c_void {
    let Some(m) = text(module) else { return std::ptr::null_mut() };
    let found = catch_unwind(AssertUnwindSafe(|| {
        with_state(|st| {
            st.process
                .modules()
                .into_iter()
                .find(|x| x.name == m || x.name.rsplit('/').next() == Some(m))
                .map(|x| x.base)
        })
    }))
    .ok()
    .flatten();
    found.map_or(std::ptr::null_mut(), |a| a as usize as *mut c_void)
}

extern "C" fn api_last_error(_api: *const Api) -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

fn new_api(id: &str, data_dir: &Path, owner: u32) -> *mut Api {
    let id = CString::new(id.replace('\0', "")).unwrap_or_default();
    let dir = CString::new(data_dir.to_string_lossy().replace('\0', "")).unwrap_or_default();
    // The strings live as long as the table, which is never freed (a mod may keep the pointer).
    let api = Api {
        abi: abi::ABI_VERSION,
        size: abi::TABLE_SIZE,
        mod_id: id.into_raw(),
        data_dir: dir.into_raw(),
        log: api_log,
        hook_objc: api_hook_objc,
        hook_symbol: api_hook_symbol,
        hook_address: api_hook_address,
        hook_import: api_hook_import,
        unhook: api_unhook,
        find_symbol: api_find_symbol,
        module_base: api_module_base,
        last_error: api_last_error,
        owner,
    };
    Box::into_raw(Box::new(api))
}

fn dl_error() -> String {
    let e = unsafe { dlerror() };
    text(e).unwrap_or("unknown error").to_string()
}

/// Loads one library of a mod and runs its `bhmod_native_init`.
pub fn load(mod_id: &str, lib: &Path, data_dir: &Path) -> Result<(), String> {
    let api = with_state(|st| {
        if let Some(m) = st.mods.iter().find(|m| m.id == mod_id) {
            return m.api;
        }
        let api = new_api(mod_id, data_dir, st.mods.len() as u32);
        st.mods.push(NativeMod { id: mod_id.to_string(), api, libs: Vec::new() });
        api
    });
    let path = CString::new(lib.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
    if handle.is_null() {
        return Err(format!("dlopen failed: {}", dl_error()));
    }
    let init = unsafe { dlsym(handle, c"bhmod_native_init".as_ptr()) };
    if init.is_null() {
        return Err("the library has no bhmod_native_init".into());
    }
    with_state(|st| {
        if let Some(m) = st.mods.iter_mut().find(|m| m.id == mod_id) {
            m.libs.push(handle);
        }
    });
    // The state lock is not held: init calls back into the table.
    let init: extern "C" fn(*const Api) -> c_int = unsafe { std::mem::transmute(init) };
    match catch_unwind(AssertUnwindSafe(|| init(api))) {
        Ok(0) => Ok(()),
        Ok(n) => Err(format!("bhmod_native_init returned {n}")),
        Err(_) => Err("bhmod_native_init panicked".into()),
    }
}

/// Runs the mod's `bhmod_native_shutdown` and undoes its hooks. The libraries stay loaded (a second `load` re-runs
/// `init` in the same image).
pub fn unload(mod_id: &str) {
    let libs: Vec<*mut c_void> =
        with_state(|st| st.mods.iter().find(|m| m.id == mod_id).map(|m| m.libs.clone()).unwrap_or_default());
    for h in &libs {
        let f = unsafe { dlsym(*h, c"bhmod_native_shutdown".as_ptr()) };
        if !f.is_null() {
            let f: extern "C" fn() = unsafe { std::mem::transmute(f) };
            let _ = catch_unwind(AssertUnwindSafe(|| f()));
        }
    }
    with_state(|st| {
        if let Some(i) = st.mods.iter().position(|m| m.id == mod_id) {
            let failed = st.engine.unhook_owner(&mut st.process, i as u32);
            if failed > 0 {
                super::log(LogLevel::Warn, &format!("{mod_id}: {failed} hook(s) could not be undone"));
            }
            st.mods[i].libs.clear();
        }
    });
}

/// Engine access for the game's own hooks (the weapon-skin renderer hook uses the same inline-hook primitive).
pub fn with_engine<R>(f: impl FnOnce(&mut Engine, &mut dyn Host) -> R) -> R {
    with_state(|st| f(&mut st.engine, &mut st.process))
}
