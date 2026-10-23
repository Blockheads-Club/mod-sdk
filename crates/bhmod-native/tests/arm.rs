//! Relocation of the first instructions of a function into a trampoline. The instruction words are what the NDK's
//! clang assembles for the listed mnemonics (checked with llvm-objdump); the expected output was disassembled the
//! same way.

use bhmod_native::arm::{entry_patch, trampoline, RelocError, LDR_PC, MAX_TRAMPOLINE};

const SRC: u32 = 0x1000;
const DST: u32 = 0x8000;
const JUMP_BACK: [u32; 2] = [LDR_PC, SRC + 8];

fn lit(addr: u32, n: usize) -> Option<Vec<u8>> {
    // The literal pool of the fake game: the bytes aa bb cc dd 01 02 03 04 at any address except 0xdead0000.
    (addr != 0xdead_0000).then(|| [0xAA, 0xBB, 0xCC, 0xDD, 1, 2, 3, 4][..n].to_vec())
}

fn words(w0: u32, w1: u32) -> Result<Vec<u32>, RelocError> {
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&w0.to_le_bytes());
    o[4..].copy_from_slice(&w1.to_le_bytes());
    trampoline(o, SRC, DST, &lit).map(|b| b.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
}

const PUSH: u32 = 0xe92d4800; // push {r11, lr}
const MOV_FP: u32 = 0xe1a0b00d; // mov r11, sp
const SUB_SP: u32 = 0xe24dd008; // sub sp, sp, #8

#[test]
fn plain_instructions_are_copied() {
    assert_eq!(words(PUSH, MOV_FP).unwrap(), [PUSH, MOV_FP, JUMP_BACK[0], JUMP_BACK[1]]);
    // movw/movt with 0xf in the immediate nibble, vmrs APSR_nzcv, svc, mul, a store relative to r11
    for w in [0xe30f000f, 0xe34f000f, 0xed920a01, 0xeef1fa10, 0xef000000, 0xe0000291, 0xe50b0008, 0xe8bd0810] {
        assert_eq!(words(w, SUB_SP).unwrap(), [w, SUB_SP, JUMP_BACK[0], JUMP_BACK[1]], "{w:#x}");
    }
    // a register-shifted data instruction and a conditional bx / blx reg in the second slot
    assert_eq!(words(PUSH, 0x012fff1e).unwrap()[1], 0x012fff1e);
    assert_eq!(words(PUSH, 0xe12fff33).unwrap()[1], 0xe12fff33);
}

#[test]
fn branches_become_absolute_jumps() {
    // bne .+16 at SRC: target SRC + 8 + 8 = 0x1010
    assert_eq!(words(0x1a000002, MOV_FP).unwrap(), [0x0a000001, LDR_PC, 0x1010, MOV_FP, JUMP_BACK[0], JUMP_BACK[1]]);
    // bl .+16
    assert_eq!(words(0xeb000002, MOV_FP).unwrap(), [0xe28fe004, LDR_PC, 0x1010, MOV_FP, JUMP_BACK[0], JUMP_BACK[1]]);
    // blne .+16 (second slot, target = SRC + 4 + 8 + 8)
    assert_eq!(
        words(SUB_SP, 0x1b000002).unwrap(),
        [SUB_SP, 0x0a000002, 0xe28fe004, LDR_PC, 0x1014, JUMP_BACK[0], JUMP_BACK[1]]
    );
    // an unconditional b in the second slot ends the function: jump there
    assert_eq!(words(PUSH, 0xea000002).unwrap()[1..3], [LDR_PC, 0x1014]);
    // backwards branch
    assert_eq!(words(0x1affffff, MOV_FP).unwrap()[1..3], [LDR_PC, 0x1004]);
}

#[test]
fn pc_relative_loads_copy_the_literal() {
    // ldr r1, [pc, #16] ; add r1, pc, r1: the literal at SRC+8+16 is copied behind the code, the add is corrected
    let t = words(0xe59f1010, 0xe08f1001).unwrap();
    assert_eq!(t, [0xe59f1014, 0xe08f1001, 0xe2811c90, 0xe28118ff, 0xe28114ff, LDR_PC, SRC + 8, 0xddccbbaa]);
    // ldrb with a negative offset: same slot, byte 0
    let t = words(0xe55f1010, MOV_FP).unwrap();
    assert_eq!(t[0] & 0xFFFF_F000, 0xe5df1000);
    assert_eq!(t.last(), Some(&0xddccbbaa));
    // vldr s0 / d0
    let t = words(0xed9f0a04, MOV_FP).unwrap();
    assert_eq!(t[0], 0xed9f0a02); // the pool is 2 words ahead of the pc of the vldr
    let d = words(0xed9f0b04, MOV_FP).unwrap();
    assert_eq!(d[0], 0xed9f0b02);
    assert_eq!(d[d.len() - 2..], [0xddccbbaa, 0x04030201]);
}

#[test]
fn pc_arithmetic_is_corrected() {
    // add r1, pc, #16 / sub r1, pc, #16 / mov r2, pc / addeq (condition kept on the fix-ups)
    let t = words(0xe28f1010, MOV_FP).unwrap();
    assert_eq!(t[..4], [0xe28f1010, 0xe2811c90, 0xe28118ff, 0xe28114ff]);
    assert_eq!(words(0xe24f1010, MOV_FP).unwrap()[0], 0xe24f1010);
    assert_eq!(words(0xe1a0200f, MOV_FP).unwrap()[1], 0xe2822c90);
    let t = words(0x028f1010, MOV_FP).unwrap();
    assert_eq!(t[1] >> 28, 0);
    // the correction of a second-slot instruction uses its own trampoline address: SRC+4 -> DST+4 (same delta)
    assert_eq!(words(PUSH, 0xe28f1010).unwrap()[2], 0xe2811c90);
    // a delta that is zero needs no fix-up: SRC == DST
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&0xe28f1010u32.to_le_bytes());
    o[4..].copy_from_slice(&MOV_FP.to_le_bytes());
    let t = trampoline(o, DST, DST, &lit).unwrap();
    assert_eq!(t.len(), 4 * 4);
}

#[test]
fn refused_instructions() {
    let unsupported = |w0, w1| matches!(words(w0, w1), Err(RelocError::Unsupported { .. }));
    // ldrh literal, add r1, r2, pc, cmp pc, pop {r11, pc}, mov pc, lr, ldr pc, blx imm / unconditional space,
    // str to pc-relative, ldr with write-back, ldr pc-relative register offset, adds r1, pc, #0, add r1, pc, pc
    for w in [
        0xe1df10b8,
        0xe082100f,
        0xe35f0000,
        0xe8bd8800,
        0xe1a0f00e,
        0xe51ff004,
        0xfa000000,
        0xe58f1010,
        0xe5bf1010,
        0xe79f1002,
        0xe29f1000,
        0xe08f100f,
        0xe92d8000,
        0xe8bf0001,
        0xed8f0a04,
        0xe1a0100f | 1 << 4,
        0xe3a0f000,
        0xe300f000,
        0xe38f1001,
        0xe7ff1fd1, // movw pc, orr with pc, media instruction space
        0xe12fff1f, // bx pc
    ] {
        assert!(unsupported(w, MOV_FP), "{w:#x}: {:?}", words(w, MOV_FP));
    }
    // too short: unconditional bx lr / b first
    assert_eq!(words(0xe12fff1e, MOV_FP), Err(RelocError::TooShort));
    assert_eq!(words(0xea000002, MOV_FP), Err(RelocError::TooShort));
    // unreadable literal
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&0xe59f1010u32.to_le_bytes());
    o[4..].copy_from_slice(&MOV_FP.to_le_bytes());
    assert_eq!(trampoline(o, 0xdead_0000 - 0x18, DST, &lit), Err(RelocError::Literal(0xdead_0000)));
    // unaligned source or destination
    assert_eq!(trampoline(o, SRC + 1, DST, &lit), Err(RelocError::Unaligned(SRC + 1)));
    assert_eq!(trampoline(o, SRC, DST + 2, &lit), Err(RelocError::Unaligned(DST + 2)));
}

#[test]
fn the_largest_trampoline_fits_the_reserved_space() {
    // blne (4 words) + add pc fix-ups (1 + 3) + ... both slots at their worst, plus jump back and a pool
    let worst = words(0x1b000002, 0xe28f1010).unwrap();
    assert!(worst.len() * 4 <= MAX_TRAMPOLINE);
    let pool = words(0xed9f0b04, 0xe59f1010).unwrap();
    assert!(pool.len() * 4 <= MAX_TRAMPOLINE);
}

#[test]
fn entry_patch_is_a_jump() {
    let p = entry_patch(0x1234_5678);
    assert_eq!(p[..4], LDR_PC.to_le_bytes());
    assert_eq!(p[4..], 0x1234_5678u32.to_le_bytes());
}

#[test]
fn errors_display() {
    for e in [
        RelocError::Unaligned(3),
        RelocError::TooShort,
        RelocError::Unsupported { at: 0x10, word: 0xe1a0f00e },
        RelocError::Literal(8),
    ] {
        assert!(!e.to_string().is_empty());
    }
}
