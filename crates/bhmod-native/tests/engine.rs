//! The hook engine and the ELF import reader on a fake process: a flat memory image with one module whose
//! dynamic section, symbols, relocations and GOT are laid out like a real armv7 library.

use bhmod_native::abi::Code;
use bhmod_native::arm::LDR_PC;
use bhmod_native::elf::{self, ElfError, Kind};
use bhmod_native::engine::{Engine, Host, ModuleInfo};
use std::collections::HashMap;

const MEM: u32 = 0x1_0000;
const BASE: u32 = 0x4000; // the module: dynamic 0x100, strtab 0x200, symtab 0x300, rel 0x400, jmprel 0x500, got 0x600
const CODE: u32 = BASE + 0x1000;
const EXEC: u32 = 0x8000;
const PUSH: u32 = 0xe92d4800;
const MOV_FP: u32 = 0xe1a0b00d;

struct Fake {
    mem: Vec<u8>,
    modules: Vec<ModuleInfo>,
    exec_next: u32,
    fail_alloc: bool,
    fail_write_at: Vec<u32>,
    fail_imp: bool,
    objc: HashMap<(String, String, bool), (u32, u32)>, // -> (method, imp)
    writes: Vec<u32>,
}

fn w32(m: &mut [u8], at: u32, v: u32) {
    m[at as usize..at as usize + 4].copy_from_slice(&v.to_le_bytes());
}

impl Fake {
    fn new() -> Fake {
        let mut mem = vec![0u8; MEM as usize];
        // strtab: "\0puts\0malloc\0"
        let strs = b"\0puts\0malloc\0empty\0";
        mem[(BASE + 0x200) as usize..(BASE + 0x200) as usize + strs.len()].copy_from_slice(strs);
        // symtab: 0 null, 1 puts (name 1), 2 malloc (name 6), 3 "" (name 13)
        w32(&mut mem, BASE + 0x310, 1);
        w32(&mut mem, BASE + 0x320, 6);
        w32(&mut mem, BASE + 0x330, 19);
        // .rel.dyn at 0x400: GLOB_DAT puts -> got 0x600, ABS32 malloc -> 0x608, relative (type 23) -> skipped,
        // GLOB_DAT with symbol 0 -> skipped, GLOB_DAT with the empty name -> skipped
        for (i, (off, info)) in
            [(0x600, 1 << 8 | 21), (0x608, 2 << 8 | 2), (0x610, 23), (0x614, 21), (0x618, 3 << 8 | 21)]
                .into_iter()
                .enumerate()
        {
            w32(&mut mem, BASE + 0x400 + 8 * i as u32, off);
            w32(&mut mem, BASE + 0x404 + 8 * i as u32, info);
        }
        // .rel.plt at 0x500: JUMP_SLOT puts -> got 0x620
        w32(&mut mem, BASE + 0x500, 0x620);
        w32(&mut mem, BASE + 0x504, 1 << 8 | 22);
        // GOT contents
        w32(&mut mem, BASE + 0x600, 0x1111);
        w32(&mut mem, BASE + 0x608, 0x2222);
        w32(&mut mem, BASE + 0x620, 0x1111);
        // dynamic: link-time addresses (below the base), as bionic leaves them
        let dynamic = [(5, 0x200), (6, 0x300), (17, 0x400), (18, 40), (23, 0x500), (2, 8), (1, 0), (0, 0)];
        for (i, (t, v)) in dynamic.into_iter().enumerate() {
            w32(&mut mem, BASE + 0x100 + 8 * i as u32, t);
            w32(&mut mem, BASE + 0x104 + 8 * i as u32, v);
        }
        // code at CODE: push {r11, lr}; mov r11, sp; ... ; a bx lr function at CODE + 0x20
        w32(&mut mem, CODE, PUSH);
        w32(&mut mem, CODE + 4, MOV_FP);
        w32(&mut mem, CODE + 0x20, 0xe12fff1e);
        w32(&mut mem, CODE + 0x40, 0xe59f1010); // ldr r1, [pc, #16]
        w32(&mut mem, CODE + 0x44, MOV_FP);
        w32(&mut mem, CODE + 0x58, 0xcafe_f00d);
        Fake {
            mem,
            modules: vec![ModuleInfo {
                name: "/data/app/lib/libApplication.so".into(),
                base: BASE,
                dynamic: BASE + 0x100,
            }],
            exec_next: EXEC,
            fail_alloc: false,
            fail_write_at: vec![],
            fail_imp: false,
            objc: HashMap::new(),
            writes: vec![],
        }
    }
    fn w(&self, at: u32) -> u32 {
        u32::from_le_bytes(self.mem[at as usize..at as usize + 4].try_into().unwrap())
    }
}

impl Host for Fake {
    fn read(&self, addr: u32, len: usize) -> Option<Vec<u8>> {
        self.mem.get(addr as usize..addr as usize + len).map(<[u8]>::to_vec)
    }
    fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), String> {
        if self.fail_write_at.contains(&addr) {
            return Err("protected".into());
        }
        self.writes.push(addr);
        self.mem[addr as usize..addr as usize + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    fn write_code(&mut self, addr: u32, bytes: &[u8]) -> Result<(), String> {
        self.write(addr, bytes)
    }
    fn alloc_exec(&mut self, len: usize) -> Result<u32, String> {
        if self.fail_alloc {
            return Err("no memory".into());
        }
        let a = self.exec_next;
        self.exec_next += len as u32;
        Ok(a)
    }
    fn modules(&self) -> Vec<ModuleInfo> {
        self.modules.clone()
    }
    fn symbol(&self, module: Option<&str>, name: &str) -> Option<u32> {
        match (module, name) {
            (None | Some("libApplication.so"), "func") => Some(CODE),
            (_, "short") => Some(CODE + 0x20),
            (_, "literal") => Some(CODE + 0x40),
            (_, "thumb") => Some(CODE + 1),
            (_, "garbage") => Some(MEM - 4),
            _ => None,
        }
    }
    fn objc_method(&mut self, class: &str, selector: &str, meta: bool) -> Option<u32> {
        self.objc.get(&(class.into(), selector.into(), meta)).map(|m| m.0)
    }
    fn objc_set_imp(&mut self, method: u32, imp: u32) -> Option<u32> {
        if self.fail_imp {
            return None;
        }
        let entry = self.objc.values_mut().find(|m| m.0 == method)?;
        Some(std::mem::replace(&mut entry.1, imp))
    }
}

#[test]
fn inline_hook_patches_and_restores() {
    let mut h = Fake::new();
    let mut e = Engine::new();
    assert!(e.is_empty());
    let r = e.hook_symbol(&mut h, 1, None, "func", 0xBEEF0).unwrap();
    assert_eq!((r.handle, r.original), (1, EXEC));
    assert_eq!((h.w(CODE), h.w(CODE + 4)), (LDR_PC, 0xBEEF0));
    // the trampoline runs the two original instructions and jumps back behind the patch
    let t = [h.w(EXEC), h.w(EXEC + 4), h.w(EXEC + 8), h.w(EXEC + 12)];
    assert_eq!(t, [PUSH, MOV_FP, LDR_PC, CODE + 8]);
    assert_eq!(e.len(), 1);
    // pc-relative literals are read through the host
    let r = e.hook_symbol(&mut h, 1, None, "literal", 0xBEEF8).unwrap();
    assert_eq!(h.w(r.original + 16), 0xcafe_f00d);
    e.unhook(&mut h, r.handle).unwrap();
    // a second hook of the same function is refused, another function is fine
    let err = e.hook_symbol(&mut h, 1, Some("libApplication.so"), "func", 0xBEEF4).unwrap_err();
    assert_eq!(err.code, Code::Exists);
    assert_eq!(e.last_error(), err.to_string());
    e.unhook(&mut h, 1).unwrap();
    assert_eq!((h.w(CODE), h.w(CODE + 4)), (PUSH, MOV_FP));
    assert!(e.is_empty());
    assert_eq!(e.unhook(&mut h, 1).unwrap_err().code, Code::Handle);
}

#[test]
fn inline_hook_failures() {
    let mut h = Fake::new();
    let mut e = Engine::new();
    let code = |e: &mut Engine, h: &mut Fake, sym: &str| e.hook_symbol(h, 1, None, sym, 0x100).unwrap_err().code;
    assert_eq!(code(&mut e, &mut h, ""), Code::Arg);
    assert_eq!(code(&mut e, &mut h, "nothing"), Code::NotFound);
    assert_eq!(code(&mut e, &mut h, "thumb"), Code::Thumb);
    assert_eq!(code(&mut e, &mut h, "short"), Code::Reloc);
    assert_eq!(code(&mut e, &mut h, "garbage"), Code::Memory);
    assert_eq!(e.hook_symbol(&mut h, 1, None, "func", 0).unwrap_err().code, Code::Arg);
    h.fail_alloc = true;
    assert_eq!(code(&mut e, &mut h, "func"), Code::Memory);
    h.fail_alloc = false;
    // the trampoline cannot be written, then the patch cannot be written
    h.fail_write_at = vec![EXEC];
    h.exec_next = EXEC;
    assert_eq!(code(&mut e, &mut h, "func"), Code::Memory);
    h.fail_write_at = vec![CODE];
    assert_eq!(code(&mut e, &mut h, "func"), Code::Memory);
    assert!(e.is_empty());
    // an unaligned trampoline arena is reported as a Thumb-like alignment problem
    h.fail_write_at.clear();
    h.exec_next = EXEC + 2;
    assert_eq!(code(&mut e, &mut h, "func"), Code::Thumb);
}

#[test]
fn hook_by_address() {
    let mut h = Fake::new();
    let mut e = Engine::new();
    let r = e.hook_address(&mut h, 2, "libApplication.so", 0x1000, 0x500).unwrap();
    assert_eq!(h.w(CODE + 4), 0x500);
    assert_eq!(r.original, EXEC);
    assert_eq!(e.hook_address(&mut h, 2, "libnope.so", 0, 0x500).unwrap_err().code, Code::NotFound);
    assert_eq!(e.hook_address(&mut h, 2, "libApplication.so", 0x1001, 0x500).unwrap_err().code, Code::Thumb);
}

#[test]
fn import_hooks_patch_every_slot() {
    let mut h = Fake::new();
    let mut e = Engine::new();
    let r = e.hook_import(&mut h, 3, None, "puts", 0x7770).unwrap();
    assert_eq!(r.original, 0x1111);
    assert_eq!((h.w(BASE + 0x600), h.w(BASE + 0x620)), (0x7770, 0x7770));
    e.unhook(&mut h, r.handle).unwrap();
    assert_eq!((h.w(BASE + 0x600), h.w(BASE + 0x620)), (0x1111, 0x1111));
    // a data word that holds the address of a symbol (R_ARM_ABS32) is a slot as well
    assert_eq!(e.hook_import(&mut h, 3, Some("libApplication.so"), "malloc", 0x7780).unwrap().original, 0x2222);
    assert_eq!(h.w(BASE + 0x608), 0x7780);
    let code =
        |e: &mut Engine, h: &mut Fake, m: Option<&str>, s: &str, r: u32| e.hook_import(h, 3, m, s, r).unwrap_err().code;
    assert_eq!(code(&mut e, &mut h, None, "", 1), Code::Arg);
    assert_eq!(code(&mut e, &mut h, None, "puts", 0), Code::Arg);
    assert_eq!(code(&mut e, &mut h, Some("libnope.so"), "puts", 1), Code::NotFound);
    assert_eq!(code(&mut e, &mut h, None, "exit", 1), Code::NotFound);
    // a module without a symbol table and a module with unreadable memory are skipped
    h.modules.push(ModuleInfo { name: "libbad.so".into(), base: BASE, dynamic: BASE + 0x2000 });
    h.modules.push(ModuleInfo { name: "libgone.so".into(), base: 0, dynamic: MEM + 8 });
    assert_eq!(code(&mut e, &mut h, None, "exit", 1), Code::NotFound);
    assert!(e.hook_import(&mut h, 3, None, "puts", 0x7790).is_ok());
}

#[test]
fn import_hook_rolls_back_when_a_slot_cannot_be_written() {
    let mut h = Fake::new();
    let mut e = Engine::new();
    h.fail_write_at = vec![BASE + 0x620];
    assert_eq!(e.hook_import(&mut h, 3, None, "puts", 0x7770).unwrap_err().code, Code::Memory);
    assert_eq!(h.w(BASE + 0x600), 0x1111);
    assert!(e.is_empty());
}

#[test]
fn objc_hooks() {
    let mut h = Fake::new();
    let mut e = Engine::new();
    h.objc.insert(("Blockhead".into(), "update:".into(), false), (0x50, 0xAAA0));
    h.objc.insert(("World".into(), "shared".into(), true), (0x60, 0xBBB0));
    let r = e.hook_objc(&mut h, 4, "Blockhead", "update:", false, 0x900).unwrap();
    assert_eq!(r.original, 0xAAA0);
    assert_eq!(h.objc[&("Blockhead".into(), "update:".into(), false)].1, 0x900);
    e.unhook(&mut h, r.handle).unwrap();
    assert_eq!(h.objc[&("Blockhead".into(), "update:".into(), false)].1, 0xAAA0);
    assert!(e.hook_objc(&mut h, 4, "World", "shared", true, 0x900).is_ok());
    let err = e.hook_objc(&mut h, 4, "World", "shared", false, 0x900).unwrap_err();
    assert_eq!(err.code, Code::NotFound);
    assert!(err.message.contains("-[World shared]"), "{err}");
    assert_eq!(e.hook_objc(&mut h, 4, "", "x", false, 1).unwrap_err().code, Code::Arg);
    assert_eq!(e.hook_objc(&mut h, 4, "World", "shared", true, 0).unwrap_err().code, Code::Arg);
    h.fail_imp = true;
    assert_eq!(e.hook_objc(&mut h, 4, "Blockhead", "update:", false, 0x900).unwrap_err().code, Code::Memory);
}

#[test]
fn unhook_owner_undoes_a_mods_hooks_newest_first() {
    let mut h = Fake::new();
    let mut e = Engine::new();
    h.objc.insert(("A".into(), "b".into(), false), (0x50, 0xAAA0));
    e.hook_symbol(&mut h, 1, None, "func", 0x100).unwrap();
    e.hook_import(&mut h, 2, None, "puts", 0x200).unwrap();
    e.hook_objc(&mut h, 1, "A", "b", false, 0x300).unwrap();
    assert_eq!(e.unhook_owner(&mut h, 1), 0);
    assert_eq!(e.len(), 1);
    assert_eq!(h.w(CODE), PUSH);
    // failures to restore are counted and reported by unhook
    h.fail_write_at = vec![BASE + 0x600];
    assert_eq!(e.unhook_owner(&mut h, 2), 1);
    e.hook_symbol(&mut h, 5, None, "func", 0x100).unwrap();
    h.fail_write_at = vec![CODE];
    assert_eq!(e.unhook_owner(&mut h, 5), 1);
    assert!(e.last_error().contains("could not be restored"));
    e.hook_objc(&mut h, 6, "A", "b", false, 0x300).unwrap();
    h.fail_imp = true;
    assert_eq!(e.unhook_owner(&mut h, 6), 1);
}

#[test]
fn reads_imports_with_relocated_pointers() {
    let h = Fake::new();
    let read = |a: u32, n: usize| h.read(a, n);
    let imports = elf::imports(&read, BASE, BASE + 0x100).unwrap();
    let got: Vec<(&str, u32, Kind)> = imports.iter().map(|i| (i.name.as_str(), i.slot, i.kind)).collect();
    assert_eq!(
        got,
        [
            ("puts", BASE + 0x600, Kind::GlobDat),
            ("malloc", BASE + 0x608, Kind::Abs32),
            ("puts", BASE + 0x620, Kind::JumpSlot)
        ]
    );
    // the same section with the pointers already relocated (glibc style) reads the same
    let mut h2 = Fake::new();
    for (i, v) in [(0, 0x200), (1, 0x300), (2, 0x400), (4, 0x500)] {
        let at = BASE + 0x104 + 8 * i;
        w32(&mut h2.mem, at, BASE + v);
    }
    let read2 = |a: u32, n: usize| h2.read(a, n);
    assert_eq!(elf::imports(&read2, BASE, BASE + 0x100).unwrap(), imports);
}

#[test]
fn elf_errors() {
    let h = Fake::new();
    let read = |a: u32, n: usize| h.read(a, n);
    assert_eq!(elf::imports(&read, BASE, BASE + 0x2000), Err(ElfError::NoSymbols));
    assert_eq!(elf::imports(&read, BASE, MEM), Err(ElfError::Unreadable(MEM)));
    assert!(ElfError::NoSymbols.to_string().contains("symbol"));
    assert!(ElfError::Unreadable(5).to_string().contains("0x5"));
    // a name that runs off the end of memory
    let mut h = Fake::new();
    w32(&mut h.mem, BASE + 0x310, MEM - BASE - 0x200 - 2); // name offset near the end
    let strtab_end = (MEM - 2) as usize;
    h.mem[strtab_end..].copy_from_slice(b"ab");
    let read = |a: u32, n: usize| h.read(a, n);
    assert!(matches!(elf::imports(&read, BASE, BASE + 0x100), Err(ElfError::Unreadable(_))));
    // a name that never ends is cut at 256 bytes
    let mut h = Fake::new();
    for i in 0..300 {
        h.mem[(BASE + 0x800 + i) as usize] = b'x';
    }
    w32(&mut h.mem, BASE + 0x310, 0x600);
    let read = |a: u32, n: usize| h.read(a, n);
    let imports = elf::imports(&read, BASE, BASE + 0x100).unwrap();
    assert_eq!(imports[0].name.len(), 256);
    // a module with symbols but no relocation tables has no imports
    let mut h = Fake::new();
    for (i, (t, v)) in [(5, 0x200), (6, 0x300), (0, 0)].into_iter().enumerate() {
        w32(&mut h.mem, BASE + 0x140 + 8 * i as u32, t);
        w32(&mut h.mem, BASE + 0x144 + 8 * i as u32, v);
    }
    let read = |a: u32, n: usize| h.read(a, n);
    assert_eq!(elf::imports(&read, BASE, BASE + 0x140), Ok(vec![]));
}
