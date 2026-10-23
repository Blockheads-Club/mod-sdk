//! Imports of a loaded 32-bit ARM ELF module, read from the module's dynamic section in memory.
//!
//! `R_ARM_JUMP_SLOT`, `R_ARM_GLOB_DAT` and `R_ARM_ABS32` relocations against a named symbol are the GOT/PLT slots
//! that hold the address the module calls. Writing another address into the slot redirects that module's calls (an
//! *import hook*); the module's own copy of the symbol and other modules are untouched.

use std::fmt;

const DT_NULL: u32 = 0;
const DT_PLTRELSZ: u32 = 2;
const DT_STRTAB: u32 = 5;
const DT_SYMTAB: u32 = 6;
const DT_REL: u32 = 17;
const DT_RELSZ: u32 = 18;
const DT_JMPREL: u32 = 23;
const R_ARM_ABS32: u32 = 2;
const R_ARM_GLOB_DAT: u32 = 21;
const R_ARM_JUMP_SLOT: u32 = 22;
/// Upper bound for the number of dynamic entries / relocations, so a corrupt table cannot loop for long.
const LIMIT: u32 = 1 << 20;
/// Relocation entries read per call.
const BLOCK: u32 = 512;

/// What kind of relocation owns a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `R_ARM_JUMP_SLOT`: a PLT call.
    JumpSlot,
    /// `R_ARM_GLOB_DAT`: a GOT entry.
    GlobDat,
    /// `R_ARM_ABS32`: a data word holding the symbol's address.
    Abs32,
}

/// One import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The symbol name.
    pub name: String,
    /// Address of the slot (module base + `r_offset`).
    pub slot: u32,
    /// The relocation type.
    pub kind: Kind,
}

/// Why the dynamic section could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfError {
    /// Memory at this address could not be read.
    Unreadable(u32),
    /// There is no symbol or string table.
    NoSymbols,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ElfError::Unreadable(a) => write!(f, "cannot read module memory at {a:#x}"),
            ElfError::NoSymbols => write!(f, "the module has no symbol table"),
        }
    }
}

impl std::error::Error for ElfError {}

type Read<'a> = &'a dyn Fn(u32, usize) -> Option<Vec<u8>>;

fn u32_at(read: Read<'_>, addr: u32) -> Result<u32, ElfError> {
    let b = read(addr, 4).filter(|b| b.len() == 4).ok_or(ElfError::Unreadable(addr))?;
    Ok(u32::from_le_bytes(b[..4].try_into().expect("4 bytes")))
}

fn cstr(read: Read<'_>, addr: u32) -> Result<String, ElfError> {
    let mut out = Vec::new();
    let mut at = addr;
    while out.len() < 256 {
        // Up to the end of a 64-byte block, so a name never reads across a page it did not need.
        let n = (64 - (at % 64)) as usize;
        let chunk = read(at, n).filter(|b| b.len() == n).ok_or(ElfError::Unreadable(at))?;
        if let Some(end) = chunk.iter().position(|b| *b == 0) {
            out.extend_from_slice(&chunk[..end]);
            return Ok(String::from_utf8_lossy(&out).into_owned());
        }
        out.extend_from_slice(&chunk);
        at += n as u32;
    }
    out.truncate(256);
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The imports of the module loaded at `base` whose `PT_DYNAMIC` is at `dynamic`. Pointers in the dynamic section
/// may be link-time addresses (bionic leaves them) or already relocated; both are accepted.
pub fn imports(read: Read<'_>, base: u32, dynamic: u32) -> Result<Vec<Import>, ElfError> {
    let fix = |v: u32| if v < base { v.wrapping_add(base) } else { v };
    let (mut strtab, mut symtab, mut rel, mut relsz, mut jmprel, mut pltrelsz) = (0, 0, 0, 0, 0, 0);
    for i in 0..LIMIT {
        let tag = u32_at(read, dynamic + 8 * i)?;
        let val = u32_at(read, dynamic + 8 * i + 4)?;
        match tag {
            DT_NULL => break,
            DT_STRTAB => strtab = fix(val),
            DT_SYMTAB => symtab = fix(val),
            DT_REL => rel = fix(val),
            DT_RELSZ => relsz = val,
            DT_JMPREL => jmprel = fix(val),
            DT_PLTRELSZ => pltrelsz = val,
            _ => {}
        }
    }
    if strtab == 0 || symtab == 0 {
        return Err(ElfError::NoSymbols);
    }
    let mut out = Vec::new();
    for (table, size) in [(rel, relsz), (jmprel, pltrelsz)] {
        if table == 0 {
            continue;
        }
        // The table is read in blocks: a game library has tens of thousands of relocations.
        let count = (size / 8).min(LIMIT);
        let mut block: Vec<u8> = Vec::new();
        for i in 0..count {
            if i % BLOCK == 0 {
                let n = (count - i).min(BLOCK) as usize * 8;
                block = read(table + 8 * i, n).filter(|b| b.len() == n).ok_or(ElfError::Unreadable(table + 8 * i))?;
            }
            let at = (i % BLOCK) as usize * 8;
            let offset = u32::from_le_bytes(block[at..at + 4].try_into().expect("4 bytes"));
            let info = u32::from_le_bytes(block[at + 4..at + 8].try_into().expect("4 bytes"));
            let kind = match info & 0xFF {
                R_ARM_JUMP_SLOT => Kind::JumpSlot,
                R_ARM_GLOB_DAT => Kind::GlobDat,
                R_ARM_ABS32 => Kind::Abs32,
                _ => continue,
            };
            let sym = info >> 8;
            if sym == 0 {
                continue;
            }
            let name_off = u32_at(read, symtab + 16 * sym)?;
            let name = cstr(read, strtab + name_off)?;
            if !name.is_empty() {
                out.push(Import { name, slot: base.wrapping_add(offset), kind });
            }
        }
    }
    Ok(out)
}
