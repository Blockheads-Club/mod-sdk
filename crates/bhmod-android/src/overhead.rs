//! Overhead icons (API 0.3.4 `ui.set-overhead`): which icon each mod put above which player, and where on the screen
//! that player's blockheads are.
//!
//! The game draws the world with OpenGL and keeps the matrices it unprojects taps with in `World`
//! (`tapProjectionMatrix`, `tapModelviewMatrix`, `tapViewport`). Every blockhead's update (a hook the cosmetics
//! already have, on whatever thread the game updates on) reports its player and `renderPos` to a [`Tracker`]; once a
//! frame the loader thread projects the head of every tracked blockhead whose player has an icon with a [`Camera`]
//! and posts the list to the Java overlay, which draws the icons there. Nothing runs while no icon is set.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

/// Icon names in the order a player's icons are shown (most important first).
pub const ICONS: [&str; 5] = ["admin-mute", "deafen", "self-mute", "speaking", "no-voice"];

/// How far above `renderPos` the icon sits, in tiles.
pub const HEAD_TILES: f32 = 2.4;

/// A blockhead not updated for this long is gone (milliseconds).
pub const STALE_MS: u64 = 1000;

/// A tap this close to a blockhead's body (fraction of the screen height) picks it.
pub const PICK_RADIUS: f32 = 0.08;

/// Positions are posted at most this often (milliseconds), and only when something moved.
pub const POST_MS: u64 = 33;

/// Which icons the server's admin shows (`voice.icons`), all `true` before the server said anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    /// `speaking`.
    pub speaking: bool,
    /// `no-voice`.
    pub no_voice: bool,
    /// `self-mute`.
    pub self_mute: bool,
    /// `deafen`.
    pub deafen: bool,
    /// `admin-mute`.
    pub admin_mute: bool,
}

impl Default for Visibility {
    fn default() -> Visibility {
        Visibility { speaking: true, no_voice: true, self_mute: true, deafen: true, admin_mute: true }
    }
}

impl Visibility {
    /// True when `icon` may be drawn (unknown names are not).
    pub fn shows(&self, icon: &str) -> bool {
        match icon {
            "speaking" => self.speaking,
            "no-voice" => self.no_voice,
            "self-mute" => self.self_mute,
            "deafen" => self.deafen,
            "admin-mute" => self.admin_mute,
            _ => false,
        }
    }
}

/// The icons mods set: player -> mod -> icon.
#[derive(Debug, Default)]
pub struct Board {
    icons: BTreeMap<String, BTreeMap<String, String>>,
    visibility: Visibility,
    last: Option<Value>,
    last_post: u64,
}

impl Board {
    /// Sets (or replaces) the icon of `mod_id` on `player`.
    pub fn set(&mut self, mod_id: &str, player: &str, icon: &str) {
        self.icons.entry(player.to_string()).or_default().insert(mod_id.to_string(), icon.to_string());
    }

    /// Removes the icon of `mod_id` from `player`; false when it had none.
    pub fn clear(&mut self, mod_id: &str, player: &str) -> bool {
        let Some(m) = self.icons.get_mut(player) else { return false };
        let had = m.remove(mod_id).is_some();
        if m.is_empty() {
            self.icons.remove(player);
        }
        had
    }

    /// Removes every icon of `mod_id` (the mod stopped).
    pub fn clear_mod(&mut self, mod_id: &str) {
        self.icons.retain(|_, m| {
            m.remove(mod_id);
            !m.is_empty()
        });
    }

    /// Removes every icon (the world closed).
    pub fn clear_all(&mut self) {
        self.icons.clear();
    }

    /// The admin's icon visibility.
    pub fn set_visibility(&mut self, v: Visibility) {
        self.visibility = v;
    }

    /// The admin's icon visibility.
    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    /// True when any icon is set (the blockhead tracking runs only then).
    pub fn active(&self) -> bool {
        !self.icons.is_empty()
    }

    /// The icons drawn above `player`, most important first, without duplicates or hidden ones.
    pub fn shown(&self, player: &str) -> Vec<&'static str> {
        let Some(m) = self.icons.get(player) else { return Vec::new() };
        ICONS.iter().copied().filter(|i| self.visibility.shows(i) && m.values().any(|v| v == i)).collect()
    }

    /// Players with something to draw.
    pub fn players(&self) -> Vec<&str> {
        self.icons.keys().map(String::as_str).filter(|p| !self.shown(p).is_empty()).collect()
    }

    /// Forgets what was posted last (the overlay was cleared).
    pub fn reset_post(&mut self) {
        self.last = None;
    }

    /// The overlay command for this frame: every visible blockhead (player, screen fractions) with its icons.
    /// `None` when nothing changed since the last post or the last post is younger than [`POST_MS`] (the
    /// overlay keeps showing it).
    pub fn frame(&mut self, now: u64, heads: &[(String, f32, f32)]) -> Option<String> {
        let items: Vec<Value> = heads
            .iter()
            .filter_map(|(p, x, y)| {
                let icons = self.shown(p);
                (!icons.is_empty()).then(|| json!({"player": p, "icons": icons, "x": quant(*x), "y": quant(*y)}))
            })
            .collect();
        let v = json!({"t": "overhead", "items": items});
        if self.last.as_ref() == Some(&v) || (now < self.last_post + POST_MS && self.last.is_some()) {
            return None;
        }
        self.last_post = now;
        self.last = Some(v.clone());
        Some(v.to_string())
    }
}

/// Rounds a screen fraction so that sub-pixel jitter does not post again.
fn quant(f: f32) -> f32 {
    (f * 1000.0).round() / 1000.0
}

/// The game's world-to-screen transform (`World.tap*`): column-major 4x4 matrices and the GL viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Projection matrix.
    pub projection: [f32; 16],
    /// Model-view matrix.
    pub modelview: [f32; 16],
    /// `x, y, width, height` in pixels (GL: `y` from the bottom).
    pub viewport: [i32; 4],
}

impl Camera {
    /// Screen fractions (`0..1`, from the top left) of world point `(x, y)`; `None` when it is off screen (with a
    /// margin so an icon slides out at the edge) or the transform is degenerate.
    pub fn project(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let mul = |m: &[f32; 16], v: [f32; 4]| -> [f32; 4] {
            let mut o = [0.0; 4];
            for (r, out) in o.iter_mut().enumerate() {
                *out = (0..4).map(|c| m[c * 4 + r] * v[c]).sum();
            }
            o
        };
        let clip = mul(&self.projection, mul(&self.modelview, [x, y, 0.0, 1.0]));
        let [_, _, w, h] = self.viewport;
        if clip[3].abs() < f32::EPSILON || w <= 0 || h <= 0 {
            return None;
        }
        let (nx, ny) = (clip[0] / clip[3], clip[1] / clip[3]);
        let (fx, fy) = ((nx + 1.0) / 2.0, 1.0 - (ny + 1.0) / 2.0);
        ((-0.1..=1.1).contains(&fx) && (-0.1..=1.1).contains(&fy)).then_some((fx, fy))
    }
}

/// World units per tile, from a blockhead's `renderPos` (world units) and `pos` (tile): the game draws a tile as
/// a fixed number of units; far from the origin the ratio is exact enough, near it 1 is assumed.
pub fn unit(render_y: f32, tile_y: i32) -> f32 {
    if tile_y.abs() < 4 {
        return 1.0;
    }
    let u = render_y / tile_y as f32;
    if u.is_finite() && u > 0.0 {
        u.round().max(1.0)
    } else {
        1.0
    }
}

/// The world point the icon of a blockhead goes to.
pub fn head(render: (f32, f32), tile: (i32, i32)) -> (f32, f32) {
    (render.0, render.1 + HEAD_TILES * unit(render.1, tile.1))
}

/// One blockhead as its last update saw it.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    player: String,
    render: (f32, f32),
    tile: (i32, i32),
    at: u64,
}

/// Blockheads reported by the game's update threads (by object address), read by the loader thread.
#[derive(Debug, Default)]
pub struct Tracker {
    seen: Mutex<HashMap<usize, Seen>>,
    on: AtomicBool,
}

impl Tracker {
    /// An empty tracker.
    pub fn new() -> Tracker {
        Tracker::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<usize, Seen>> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Turns the reports on (some player has an icon) or off (and forgets every blockhead).
    pub fn set_active(&self, on: bool) {
        if !self.on.swap(on, Ordering::Relaxed) || on {
            return;
        }
        self.reset();
    }

    /// True while the game's update hook should report blockheads.
    pub fn active(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// A blockhead (address `bh`) of `player` was updated at `now`.
    pub fn report(&self, bh: usize, player: &str, render: (f32, f32), tile: (i32, i32), now: u64) {
        let mut s = self.lock();
        match s.get_mut(&bh) {
            Some(e) => {
                if e.player != player {
                    e.player = player.to_string();
                }
                e.render = render;
                e.tile = tile;
                e.at = now;
            }
            None => {
                s.insert(bh, Seen { player: player.to_string(), render, tile, at: now });
            }
        }
    }

    /// Forgets every blockhead (the world closed: addresses may be reused).
    pub fn reset(&self) {
        self.lock().clear();
    }

    /// Blockheads tracked now.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when none is tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The player whose blockhead (updated within [`STALE_MS`]) is nearest to the tap at screen fractions `(x, y)`,
    /// within [`PICK_RADIUS`] of its body centre (one tile above `renderPos`).
    pub fn pick(&self, now: u64, cam: &Camera, x: f32, y: f32) -> Option<String> {
        let s = self.lock();
        let (w, h) = (cam.viewport[2].max(1) as f32, cam.viewport[3].max(1) as f32);
        let mut best: Option<(f32, &str)> = None;
        for e in s.values().filter(|e| now.saturating_sub(e.at) <= STALE_MS) {
            let Some((fx, fy)) = cam.project(e.render.0, e.render.1 + unit(e.render.1, e.tile.1)) else { continue };
            let d = ((fx - x) * w).hypot((fy - y) * h) / h;
            if d <= PICK_RADIUS && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, &e.player));
            }
        }
        best.map(|(_, p)| p.to_string())
    }

    /// Screen positions of the heads of `players`' blockheads updated within [`STALE_MS`] (older ones are dropped),
    /// sorted by player then position.
    pub fn heads(&self, now: u64, players: &BTreeSet<&str>, cam: &Camera) -> Vec<(String, f32, f32)> {
        let mut s = self.lock();
        s.retain(|_, e| now.saturating_sub(e.at) <= STALE_MS);
        let mut v: Vec<(String, f32, f32)> = s
            .values()
            .filter(|e| players.contains(e.player.as_str()))
            .filter_map(|e| {
                let (x, y) = head(e.render, e.tile);
                cam.project(x, y).map(|(fx, fy)| (e.player.clone(), fx, fy))
            })
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

    /// An orthographic camera over world `0..100 x 0..50` (column-major, like GLKMatrix4MakeOrtho).
    fn cam() -> Camera {
        let (l, r, b, t) = (0.0f32, 100.0f32, 0.0f32, 50.0f32);
        let mut p = ID;
        p[0] = 2.0 / (r - l);
        p[5] = 2.0 / (t - b);
        p[12] = -(r + l) / (r - l);
        p[13] = -(t + b) / (t - b);
        Camera { projection: p, modelview: ID, viewport: [0, 0, 800, 400] }
    }

    #[test]
    fn projection_maps_the_world_to_screen_fractions() {
        let c = cam();
        assert_eq!(c.project(50.0, 25.0), Some((0.5, 0.5)));
        assert_eq!(c.project(0.0, 50.0), Some((0.0, 0.0)));
        assert_eq!(c.project(100.0, 0.0), Some((1.0, 1.0)));
        assert_eq!(c.project(200.0, 25.0), None);
        assert_eq!(c.project(50.0, -30.0), None);
        let mut moved = c;
        moved.modelview[12] = -50.0; // camera moved right by 50 units
        assert_eq!(moved.project(100.0, 25.0), Some((0.5, 0.5)));
        let flat = Camera { projection: [0.0; 16], ..c };
        assert_eq!(flat.project(1.0, 1.0), None);
        let empty = Camera { viewport: [0, 0, 0, 10], ..c };
        assert_eq!(empty.project(50.0, 25.0), None);
        let empty = Camera { viewport: [0, 0, 10, 0], ..c };
        assert_eq!(empty.project(50.0, 25.0), None);
    }

    #[test]
    fn units_and_heads() {
        assert_eq!(unit(5.0, 1), 1.0);
        assert_eq!(unit(200.0, 10), 20.0);
        assert_eq!(unit(-200.0, 10), 1.0);
        assert_eq!(unit(0.5, 10), 1.0);
        assert_eq!(unit(f32::NAN, 10), 1.0);
        assert_eq!(head((10.0, 200.0), (0, 10)), (10.0, 200.0 + HEAD_TILES * 20.0));
    }

    #[test]
    fn board_orders_filters_and_clears() {
        let mut b = Board::default();
        assert!(!b.active());
        b.set("voice", "p1", "speaking");
        b.set("other", "p1", "admin-mute");
        b.set("voice", "p2", "no-voice");
        b.set("voice", "p3", "bogus");
        assert!(b.active());
        assert_eq!(b.shown("p1"), vec!["admin-mute", "speaking"]);
        assert_eq!(b.shown("nobody"), Vec::<&str>::new());
        assert_eq!(b.players(), vec!["p1", "p2"]);
        b.set_visibility(Visibility { admin_mute: false, no_voice: false, ..Default::default() });
        assert!(!b.visibility().admin_mute);
        assert_eq!(b.shown("p1"), vec!["speaking"]);
        assert_eq!(b.players(), vec!["p1"]);
        assert!(b.clear("voice", "p1"));
        assert!(!b.clear("voice", "p1"));
        assert!(!b.clear("voice", "nobody"));
        assert!(b.clear("other", "p1"));
        b.clear_mod("voice");
        assert!(!b.clear("voice", "p3") && !b.active());
        b.set("voice", "p1", "speaking");
        b.clear_all();
        assert!(!b.active());
        let v = Visibility::default();
        assert!(v.speaking && v.no_voice && v.self_mute && v.deafen && v.admin_mute);
        assert!(v.shows("deafen") && v.shows("self-mute") && !v.shows("x"));
    }

    #[test]
    fn frames_post_only_changes_and_not_too_often() {
        let mut b = Board::default();
        b.set("voice", "p1", "speaking");
        let heads = vec![("p1".to_string(), 0.5, 0.25), ("quiet".to_string(), 0.1, 0.1)];
        let first: Value = serde_json::from_str(&b.frame(100, &heads).unwrap()).unwrap();
        assert_eq!(first["t"], "overhead");
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        assert_eq!(first["items"][0]["icons"][0], "speaking");
        assert_eq!(b.frame(200, &heads), None, "unchanged");
        let moved = vec![("p1".to_string(), 0.6, 0.25)];
        assert_eq!(b.frame(110, &moved), None, "too soon");
        assert!(b.frame(100 + POST_MS, &moved).is_some());
        assert!(b.frame(1000, &[]).is_some(), "the icon went off screen: an empty list clears the overlay");
        assert!(b.frame(2000, &[]).is_none());
        b.reset_post();
        assert!(b.frame(2000, &[]).is_some());
    }

    #[test]
    fn tracker_reports_projects_and_forgets() {
        let t = Tracker::new();
        assert!(t.is_empty());
        t.report(1, "p1", (50.0, 10.0), (0, 0), 0);
        t.report(2, "p2", (60.0, 10.0), (0, 0), 0);
        t.report(3, "p1", (500.0, 10.0), (0, 0), 0); // off screen
        t.report(2, "p3", (60.0, 10.0), (0, 0), 500); // the address now belongs to another player's blockhead
        assert_eq!(t.len(), 3);
        let players: BTreeSet<&str> = ["p1", "p3"].into();
        let h = t.heads(900, &players, &cam());
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].0, "p1");
        assert!((h[0].1 - 0.5).abs() < 1e-6);
        assert_eq!(h[1].0, "p3");
        // blockheads 1 and 3 were last seen at 0: gone after STALE_MS
        let h = t.heads(1400, &players, &cam());
        assert_eq!(h.len(), 1);
        assert_eq!(t.len(), 1);
        t.reset();
        assert!(t.is_empty());
        assert!(!t.active());
        t.set_active(true);
        t.set_active(true);
        t.report(1, "p1", (50.0, 10.0), (0, 0), 0);
        assert!(t.active());
        t.set_active(false);
        assert!(!t.active() && t.is_empty());
        t.set_active(false);
    }

    #[test]
    fn taps_pick_the_nearest_blockhead() {
        let t = Tracker::new();
        let c = cam(); // 100 x 50 world units on an 800 x 400 viewport
        t.report(1, "near", (50.0, 24.0), (0, 0), 0); // body centre (50, 25): the middle of the screen
        t.report(2, "far", (53.0, 24.0), (0, 0), 0); // 3 units right: 0.06 of the height away
        t.report(3, "off", (500.0, 24.0), (0, 0), 0);
        t.report(4, "old", (49.5, 24.0), (0, 0), 0);
        assert_eq!(t.pick(500, &c, 0.5, 0.5).as_deref(), Some("near"));
        assert_eq!(t.pick(500, &c, 0.53, 0.5).as_deref(), Some("far"));
        assert_eq!(t.pick(500, &c, 0.9, 0.1), None);
        assert_eq!(t.pick(500, &c, 0.49, 0.5).as_deref(), Some("old"));
        t.report(1, "near", (50.0, 24.0), (0, 0), 2000);
        assert_eq!(t.pick(2000, &c, 0.49, 0.5).as_deref(), Some("near"), "only fresh blockheads");
    }

    #[test]
    fn a_poisoned_tracker_still_works() {
        let t = std::sync::Arc::new(Tracker::new());
        let other = std::sync::Arc::clone(&t);
        let _ = std::thread::spawn(move || {
            let _g = other.seen.lock().unwrap();
            panic!("poison");
        })
        .join();
        t.report(1, "p", (0.0, 0.0), (0, 0), 0);
        assert_eq!(t.len(), 1);
    }
}
