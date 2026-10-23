//! Native tier of the Blockheads mod system (milestone M10).
//!
//! - [`arm`]: the inline-hook primitive for armv7 ARM-mode code: entry patch, trampoline, instruction relocation.
//! - [`elf`]: imports of a loaded module (GOT/PLT slots by symbol name) read from its dynamic section in memory.
//! - [`engine`]: the host-independent hook engine behind the `libbhmod` C ABI. A host (the Android loader in
//!   process, the PC runner for the guest) implements [`engine::Host`] (memory, symbols, Objective-C runtime) and
//!   gets `hook_objc`, `hook_symbol`, `hook_address`, `hook_import` and `unhook` with identical behaviour.
//! - [`abi`]: the layout of the C table `bhmod_api_t` handed to a native mod (`include/bhmod.h` is its header).
#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod abi;
pub mod arm;
pub mod elf;
pub mod engine;
