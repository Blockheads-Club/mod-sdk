//! Variant blocks: how a mod block is stored in the 16 spare bytes of a tile (`Tile.padding`, offset 0x30).
//!
//! A mod block is a vanilla block (its `base`) plus this record. Vanilla servers and clients carry the bytes
//! untouched (saves and the block packet copy whole 64-byte tiles) and never read them, so a world with variant
//! blocks stays loadable everywhere; without the mod the blocks are their base block.
//!
//! The record is valid only for the tile it was written for: it carries a magic, the base `typeIndex` (a vanilla path
//! that changes the block invalidates it) and a checksum over the bytes and the tile's coordinates (a tile copied
//! to another place, or stale bytes from a recycled block of memory, are rejected).
//!
//! ```text
//! [0..2]  magic 0xB1C0 (LE)
//! [2..4]  variant id (kind `blocks` of the registry, LE)
//! [4]     base typeIndex
//! [5]     layer (0 block; others reserved)
//! [6..8]  variant data (LE, mod defined)
//! [8..15] reserved, zero
//! [15]    checksum
//! ```

/// First two bytes of every record.
pub const MAGIC: u16 = 0xB1C0;
/// Size of `Tile.padding`.
pub const LEN: usize = 16;

/// What a tile's padding says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    /// The variant's id in the world registry (kind `blocks`).
    pub id: u16,
    /// `Tile.typeIndex` the record is valid for.
    pub base: u8,
    /// Reserved: 0 is the block.
    pub layer: u8,
    /// Mod defined data.
    pub data: u16,
}

fn checksum(bytes: &[u8; LEN], x: i32, y: i32) -> u8 {
    let sum = bytes[..LEN - 1].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    // Coordinates enter as the low byte of each; 0x5A keeps an all-zero record from checking out.
    sum.wrapping_add(x as u8).wrapping_add((y as u8).rotate_left(3)) ^ 0x5A
}

/// The 16 bytes for `variant` at tile `(x, y)`.
pub fn encode(variant: &Variant, x: i32, y: i32) -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[0..2].copy_from_slice(&MAGIC.to_le_bytes());
    b[2..4].copy_from_slice(&variant.id.to_le_bytes());
    b[4] = variant.base;
    b[5] = variant.layer;
    b[6..8].copy_from_slice(&variant.data.to_le_bytes());
    b[LEN - 1] = checksum(&b, x, y);
    b
}

/// The variant a tile shows: `padding` of the tile at `(x, y)` whose `typeIndex` is `type_index`. `None` for
/// anything that is not a valid record for that tile.
pub fn decode(padding: &[u8; LEN], type_index: u8, x: i32, y: i32) -> Option<Variant> {
    if u16::from_le_bytes([padding[0], padding[1]]) != MAGIC
        || padding[4] != type_index
        || padding[8..LEN - 1].iter().any(|b| *b != 0)
        || padding[LEN - 1] != checksum(padding, x, y)
    {
        return None;
    }
    Some(Variant {
        id: u16::from_le_bytes([padding[2], padding[3]]),
        base: padding[4],
        layer: padding[5],
        data: u16::from_le_bytes([padding[6], padding[7]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Variant = Variant { id: 7, base: 10, layer: 0, data: 0x1234 };

    #[test]
    fn round_trip_at_the_tile() {
        let pad = encode(&V, 100, -3);
        assert_eq!(&pad[..2], &[0xC0, 0xB1]);
        assert_eq!(decode(&pad, 10, 100, -3), Some(V));
    }

    #[test]
    fn rejects_other_tiles_types_and_garbage() {
        let pad = encode(&V, 100, 200);
        // moved to another tile
        assert_eq!(decode(&pad, 10, 101, 200), None);
        assert_eq!(decode(&pad, 10, 100, 201), None);
        // a vanilla path changed the block
        assert_eq!(decode(&pad, 11, 100, 200), None);
        // vanilla padding is zero
        assert_eq!(decode(&[0; LEN], 10, 0, 0), None);
        // flipped payload bits, reserved bytes, and the checksum itself
        for i in 0..LEN {
            let mut bad = pad;
            bad[i] ^= 0x01;
            assert_eq!(decode(&bad, 10, 100, 200), None, "byte {i}");
        }
    }
}
