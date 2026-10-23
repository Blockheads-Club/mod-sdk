//! ARM-mode (A32) instruction relocation for inline hooks.
//!
//! An inline hook overwrites the first two instructions of a function with an absolute jump
//! (`ldr pc, [pc, #-4]` followed by the target address, [`entry_patch`]). The overwritten instructions move into a
//! *trampoline* ([`trampoline`]) that runs them at their new address and jumps back to `function + 8`; calling the
//! trampoline is calling the original function.
//!
//! Instructions that depend on their own address cannot be copied byte for byte. The relocator handles the ones that
//! occur in function prologues and refuses (with a reason) everything it cannot prove safe:
//!
//! - `b`/`bl` (any condition): re-emitted as an absolute jump / call, behind an inverted-condition branch when
//!   conditional.
//! - `ldr`/`ldrb Rt, [pc, #imm]` and `vldr`: the literal is copied into a pool behind the trampoline.
//! - `add`/`sub`/`mov Rd, pc, ...`: executed in place, then corrected by `add Rd, Rd, #chunk` for the difference
//!   between the old and the new `pc`.
//! - everything else that does not name `pc` is copied unchanged.
//!
//! Refused: Thumb targets, unconditional terminators in the first slot (the function would be shorter than the
//! patch), `pc` as a destination or base register in any other form, the unconditional (`0b1111`) instruction space.

use std::fmt;

/// `ldr pc, [pc, #-4]`: loads the word right behind this instruction into `pc`.
pub const LDR_PC: u32 = 0xE51F_F004;
/// Bytes an inline hook overwrites (two instructions).
pub const PATCH_LEN: usize = 8;
/// An upper bound for the size of any trampoline [`trampoline`] builds, so hosts can allocate before building.
pub const MAX_TRAMPOLINE: usize = 128;

/// Why an instruction cannot be moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelocError {
    /// The address is odd or not 4-byte aligned (Thumb code is not supported).
    Unaligned(u32),
    /// The first instruction ends the function (or jumps away unconditionally): there is no room for the patch.
    TooShort,
    /// An instruction the relocator does not know how to move.
    Unsupported {
        /// Address of the instruction.
        at: u32,
        /// The instruction word.
        word: u32,
    },
    /// A PC-relative literal could not be read.
    Literal(u32),
}

impl fmt::Display for RelocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelocError::Unaligned(a) => write!(f, "{a:#x} is not 4-byte aligned ARM code (Thumb is not supported)"),
            RelocError::TooShort => write!(f, "the function is shorter than the 8-byte patch"),
            RelocError::Unsupported { at, word } => write!(f, "cannot relocate instruction {word:#010x} at {at:#x}"),
            RelocError::Literal(a) => write!(f, "cannot read the literal at {a:#x}"),
        }
    }
}

impl std::error::Error for RelocError {}

/// The 8 bytes that replace the start of a function: a jump to `target`.
pub fn entry_patch(target: u32) -> [u8; PATCH_LEN] {
    let mut p = [0u8; PATCH_LEN];
    p[..4].copy_from_slice(&LDR_PC.to_le_bytes());
    p[4..].copy_from_slice(&target.to_le_bytes());
    p
}

fn cond(w: u32) -> u32 {
    w >> 28
}

/// Registers a word names in the standard fields (used only for classes where those fields are registers).
fn is_pc(w: u32, shift: u32) -> bool {
    (w >> shift) & 0xF == 15
}

/// `add Rd, Rd, #(byte << shift)` chunks that add `delta` (wrapping), none for zero.
fn delta_chain(c: u32, rd: u32, delta: u32) -> Vec<u32> {
    (0..4)
        .filter_map(|i| {
            let byte = (delta >> (8 * i)) & 0xFF;
            // Rotate right by 2*r: an 8-bit value at bit 8*i is `byte` rotated by 32 - 8*i, i.e. r = (32 - 8*i) / 2.
            let rot = ((32 - 8 * i) % 32) / 2;
            (byte != 0).then_some((c << 28) | 0x0280_0000 | (rd << 16) | (rd << 12) | (rot << 8) | byte)
        })
        .collect()
}

struct Builder {
    dst: u32,
    words: Vec<u32>,
    /// (index of the instruction to fix, pool slot, whether it counts words like `vldr`)
    fixups: Vec<(usize, usize, bool)>,
    pool: Vec<Vec<u8>>,
}

impl Builder {
    fn here(&self) -> u32 {
        self.dst + 4 * self.words.len() as u32
    }
    fn push(&mut self, w: u32) {
        self.words.push(w);
    }
    /// A jump to an absolute address: `ldr pc, [pc, #-4]; .word target`.
    fn jump(&mut self, target: u32) {
        self.push(LDR_PC);
        self.push(target);
    }
}

/// Builds the trampoline for the two instructions `orig` that live at `src`, to be placed at `dst`. It ends with a
/// jump to `src + 8`. `read(addr, len)` reads game memory (for PC-relative literals).
pub fn trampoline(
    orig: [u8; PATCH_LEN],
    src: u32,
    dst: u32,
    read: &dyn Fn(u32, usize) -> Option<Vec<u8>>,
) -> Result<Vec<u8>, RelocError> {
    if src % 4 != 0 {
        return Err(RelocError::Unaligned(src));
    }
    if dst % 4 != 0 {
        return Err(RelocError::Unaligned(dst));
    }
    let mut b = Builder { dst, words: Vec::new(), fixups: Vec::new(), pool: Vec::new() };
    for i in 0..2u32 {
        let w = u32::from_le_bytes(orig[4 * i as usize..4 * i as usize + 4].try_into().expect("4 bytes"));
        relocate(&mut b, w, src + 4 * i, i == 0, read)?;
    }
    b.jump(src + PATCH_LEN as u32);
    // The literal pool follows the code.
    let mut slot_addr = Vec::new();
    for lit in &b.pool {
        slot_addr.push(b.dst + 4 * b.words.len() as u32);
        for chunk in lit.chunks(4) {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            b.words.push(u32::from_le_bytes(w));
        }
    }
    for &(at, slot, words) in &b.fixups {
        let pc = b.dst + 4 * at as u32 + 8;
        let off = slot_addr[slot] - pc; // pools are placed behind the code: always forward
        b.words[at] |= if words { off / 4 } else { off };
    }
    Ok(b.words.iter().flat_map(|w| w.to_le_bytes()).collect())
}

fn relocate(
    b: &mut Builder,
    w: u32,
    at: u32,
    first: bool,
    read: &dyn Fn(u32, usize) -> Option<Vec<u8>>,
) -> Result<(), RelocError> {
    let c = cond(w);
    let bad = RelocError::Unsupported { at, word: w };
    if c == 0xF {
        return Err(bad);
    }
    let always = c == 0xE;
    let class = (w >> 25) & 7;
    match class {
        // Data processing, multiply, halfword transfers, misc (bx, movw, clz, ...)
        0b000 | 0b001 => {
            // bx / blx register
            if w & 0x0FFF_FFD0 == 0x012F_FF10 {
                if is_pc(w, 0) {
                    return Err(bad);
                }
                if first && always && w & 0x20 == 0 {
                    return Err(RelocError::TooShort);
                }
                b.push(w);
                return Ok(());
            }
            // movw / movt: only Rd is a register
            if w & 0x0FB0_0000 == 0x0300_0000 {
                if is_pc(w, 12) {
                    return Err(bad);
                }
                b.push(w);
                return Ok(());
            }
            let imm_form = class == 0b001;
            let reg_shift = !imm_form && w & 0x10 != 0 && w & 0x80 == 0;
            let extra = !imm_form && w & 0x10 != 0 && w & 0x80 != 0; // multiply, halfword, swap, ldrex, ...
            let op = (w >> 21) & 0xF;
            let s = w & (1 << 20) != 0;
            let (rn_pc, rd_pc, rm_pc) = (is_pc(w, 16), is_pc(w, 12), !imm_form && is_pc(w, 0));
            let rs_pc = (reg_shift || extra) && is_pc(w, 8);
            if !(rn_pc || rd_pc || rm_pc || rs_pc) {
                b.push(w);
                return Ok(());
            }
            // Only `add/sub Rd, pc, #imm|Rm`, `mov Rd, pc` (flags untouched, plain destination).
            if extra || reg_shift || rd_pc || s || rs_pc {
                return Err(bad);
            }
            let rd = (w >> 12) & 0xF;
            let ok = match op {
                0b0100 | 0b0010 => rn_pc && !(rm_pc && !imm_form),    // add, sub
                0b1101 => !imm_form && rm_pc && (w >> 4) & 0xFF == 0, // mov Rd, pc
                _ => false,
            };
            if !ok || (!imm_form && rm_pc && rn_pc) {
                return Err(bad);
            }
            let here = b.here();
            b.push(w);
            // pc read 8 ahead of the instruction: original `at + 8`, now `here + 8`.
            for x in delta_chain(c, rd, at.wrapping_sub(here)) {
                b.push(x);
            }
            Ok(())
        }
        // Single data transfer (ldr, str, ldrb, strb)
        0b010 | 0b011 => {
            let reg_off = class == 0b011;
            let load = w & (1 << 20) != 0;
            let (rn_pc, rt_pc, rm_pc) = (is_pc(w, 16), is_pc(w, 12), reg_off && is_pc(w, 0));
            if reg_off && w & 0x10 != 0 {
                return Err(bad); // media instructions
            }
            if rt_pc || rm_pc {
                return Err(bad);
            }
            if !rn_pc {
                b.push(w);
                return Ok(());
            }
            let (p, wb) = (w & (1 << 24) != 0, w & (1 << 21) != 0);
            if reg_off || !load || !p || wb {
                return Err(bad);
            }
            let imm = w & 0xFFF;
            let lit = if w & (1 << 23) != 0 {
                at.wrapping_add(8).wrapping_add(imm)
            } else {
                at.wrapping_add(8).wrapping_sub(imm)
            };
            let data = read(lit, 4).ok_or(RelocError::Literal(lit))?;
            let slot = b.pool.len();
            b.pool.push(data);
            let idx = b.words.len();
            // Offset addressing, up, base pc; the offset comes from the pool fixup.
            b.push(
                (w & 0xF000_0000)
                    | 0x0400_0000
                    | (1 << 24)
                    | (1 << 23)
                    | (w & (1 << 22))
                    | (1 << 20)
                    | (0xF << 16)
                    | (w & 0xF000),
            );
            b.fixups.push((idx, slot, false));
            Ok(())
        }
        // Block transfer
        0b100 => {
            if is_pc(w, 16) || w & 0x8000 != 0 {
                return Err(bad);
            }
            b.push(w);
            Ok(())
        }
        // Branch
        0b101 => {
            let link = w & (1 << 24) != 0;
            let imm = (((w & 0x00FF_FFFF) << 8) as i32 >> 8) as u32;
            let target = at.wrapping_add(8).wrapping_add(imm << 2);
            if !link && always {
                if first {
                    return Err(RelocError::TooShort);
                }
                b.jump(target);
                return Ok(());
            }
            let words = if link { 3 } else { 2 };
            if !always {
                // b<!cc> over the call sequence
                b.push(((c ^ 1) << 28) | 0x0A00_0000 | (words - 1));
            }
            if link {
                b.push(0xE28F_E004); // add lr, pc, #4: the word after the literal
            }
            b.jump(target);
            Ok(())
        }
        // Coprocessor: vldr/vstr/vldm/..., VFP data processing, svc
        _ => {
            let top = (w >> 24) & 0xF;
            if top == 0b1110 || top == 0b1111 {
                // VFP data processing, register transfers (`vmrs APSR_nzcv`) and svc: no addresses.
                b.push(w);
                return Ok(());
            }
            if !is_pc(w, 16) {
                b.push(w);
                return Ok(());
            }
            // vldr Sd/Dd, [pc, #imm]
            let vldr = w & 0x0F30_0E00 == 0x0D10_0A00;
            if !vldr {
                return Err(bad);
            }
            let imm = (w & 0xFF) << 2;
            let lit = if w & (1 << 23) != 0 {
                at.wrapping_add(8).wrapping_add(imm)
            } else {
                at.wrapping_add(8).wrapping_sub(imm)
            };
            let len = if w & 0x100 != 0 { 8 } else { 4 };
            let data = read(lit, len).ok_or(RelocError::Literal(lit))?;
            let slot = b.pool.len();
            b.pool.push(data);
            let idx = b.words.len();
            b.push((w & !0x00FF) | (1 << 23));
            b.fixups.push((idx, slot, true));
            Ok(())
        }
    }
}
