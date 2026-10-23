//! The `libbhmod` C ABI: the table `bhmod_api_t` a native mod receives, and the result codes.
//!
//! A native mod (`client/native/armeabi-v7a/<name>.so`) exports
//!
//! ```c
//! int  bhmod_native_init(const bhmod_api_t *api);   /* required: 0 = ok */
//! void bhmod_native_shutdown(void);                 /* optional */
//! ```
//!
//! and installs its hooks through the table by *name* (an Objective-C selector, an exported symbol, an address
//! inside a module, an import): the host decides how, so the same mod runs in the Android process and in the PC
//! runner's guest. `include/bhmod.h` is the C header; the layout below is checked against it by compiling the header
//! for armv7 (tests/header.rs).

/// Version of the table (`bhmod_api_t.abi`). Bumped on any incompatible change; fields are only ever appended.
pub const ABI_VERSION: u32 = 1;
/// Name of the required entry point of a native mod.
pub const INIT_SYMBOL: &str = "bhmod_native_init";
/// Name of the optional exit point of a native mod.
pub const SHUTDOWN_SYMBOL: &str = "bhmod_native_shutdown";
/// Size of a pointer in the guest (the game is 32-bit ARM).
pub const WORD: u32 = 4;
/// Non-function fields in front of the function pointers: `(name, byte offset)`.
pub const DATA_FIELDS: [(&str, u32); 4] = [("abi", 0), ("size", 4), ("mod_id", 8), ("data_dir", 12)];
/// The function pointers, in table order (the first sits at byte 16). Every function takes the table as its first
/// argument so the host knows which mod is calling.
pub const FUNCS: [&str; 9] = [
    "log",
    "hook_objc",
    "hook_symbol",
    "hook_address",
    "hook_import",
    "unhook",
    "find_symbol",
    "module_base",
    "last_error",
];
/// Size of the table in bytes.
pub const TABLE_SIZE: u32 = 16 + WORD * FUNCS.len() as u32;

/// Byte offset of a function pointer of the table.
pub fn func_offset(name: &str) -> Option<u32> {
    FUNCS.iter().position(|f| *f == name).map(|i| 16 + WORD * i as u32)
}

/// Result of a `hook_*` call: a positive handle on success, one of these negated otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Code {
    /// A bad argument (null pointer, empty name).
    Arg = 1,
    /// The class, method, symbol, module or import does not exist (yet).
    NotFound = 2,
    /// The target is Thumb code or not 4-byte aligned.
    Thumb = 3,
    /// The first instructions cannot be moved into a trampoline.
    Reloc = 4,
    /// The address is already hooked.
    Exists = 5,
    /// Game memory could not be read or patched.
    Memory = 6,
    /// The handle is unknown.
    Handle = 7,
}

impl Code {
    /// The value returned to C.
    pub fn ret(self) -> i32 {
        -(self as i32)
    }
}

/// Log levels of `log`.
pub const LOG_TRACE: i32 = 0;
/// Debug.
pub const LOG_DEBUG: i32 = 1;
/// Info.
pub const LOG_INFO: i32 = 2;
/// Warning.
pub const LOG_WARN: i32 = 3;
/// Error.
pub const LOG_ERROR: i32 = 4;
