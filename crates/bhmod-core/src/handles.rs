//! Host handles: how players cross a language boundary inside a [`crate::value::Value`] (`{"$h": n}`).
//!
//! A handle is `generation << 32 | index`. A player keeps one handle while connected; when they leave the slot's
//! generation moves on, so old handles resolve to [`ApiError::StaleHandle`] instead of to whoever gets the slot next.

use std::collections::HashMap;

use crate::types::ApiError;

#[derive(Debug, Clone)]
struct Entry {
    generation: u32,
    player: Option<String>,
}

/// The handle table of one host.
#[derive(Debug, Default)]
pub struct Handles {
    entries: Vec<Entry>,
    by_player: HashMap<String, u32>,
    free: Vec<u32>,
}

impl Handles {
    /// The handle of `player` (allocated on first use).
    pub fn of_player(&mut self, player: &str) -> u64 {
        if let Some(&i) = self.by_player.get(player) {
            return (self.entries[i as usize].generation as u64) << 32 | i as u64;
        }
        let i = match self.free.pop() {
            Some(i) => {
                self.entries[i as usize].player = Some(player.to_string());
                i
            }
            None => {
                self.entries.push(Entry { generation: 0, player: Some(player.to_string()) });
                (self.entries.len() - 1) as u32
            }
        };
        self.by_player.insert(player.to_string(), i);
        (self.entries[i as usize].generation as u64) << 32 | i as u64
    }

    /// The player behind a handle.
    pub fn player(&self, handle: u64) -> Result<&str, ApiError> {
        let (generation, i) = ((handle >> 32) as u32, handle as u32);
        match self.entries.get(i as usize) {
            Some(Entry { generation: g, player: Some(p) }) if *g == generation => Ok(p),
            _ => Err(ApiError::StaleHandle),
        }
    }

    /// The player left: their handle becomes stale.
    pub fn invalidate(&mut self, player: &str) {
        if let Some(i) = self.by_player.remove(player) {
            let e = &mut self.entries[i as usize];
            e.generation = e.generation.wrapping_add(1);
            e.player = None;
            self.free.push(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_resolve_invalidate_reuse() {
        let mut h = Handles::default();
        let a = h.of_player("a");
        let b = h.of_player("b");
        assert_eq!(h.of_player("a"), a);
        assert_eq!(h.player(a), Ok("a"));
        assert_eq!(h.player(b), Ok("b"));
        h.invalidate("a");
        h.invalidate("nobody");
        assert_eq!(h.player(a), Err(ApiError::StaleHandle));
        let c = h.of_player("c");
        assert_eq!(c as u32, a as u32, "slot reused");
        assert_ne!(c, a, "with a new generation");
        assert_eq!(h.player(c), Ok("c"));
        assert_eq!(h.player(a), Err(ApiError::StaleHandle));
        assert_eq!(h.player(99), Err(ApiError::StaleHandle));
    }
}
