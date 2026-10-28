//! Who hears whom: the routes the server half publishes with `voice.set-routes` about five times a second.
//!
//! - **proximity**: listeners within the radius; gain by distance (linear or logarithmic falloff), stereo pan by the
//!   horizontal offset, muffling by the solid blocks between the two (`terrain.solid-count`). Dead or sleeping
//!   speakers are silent here unless the admin allows it.
//! - **global**: everyone at full gain.
//! - **channel**: the members of the speaker's channel at full gain.
//! - **broadcast**: everyone at full gain (the host decides whether deafened players hear it).
//!
//! A speaker only gets routes for the modes they may talk in; listeners without voice get none.

use crate::config::{Curve, ServerConfig};
use crate::model::{Mode, ModeSet};

/// The host's limit of routes per speaker.
pub const MAX_ROUTES: usize = 256;

/// What the router knows about one online player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Peer {
    /// The player's tile, `None` when not in the world.
    pub pos: Option<(i32, i32)>,
    /// The modes they may talk in.
    pub allowed: ModeSet,
    /// Their channel (lower-case name).
    pub channel: Option<String>,
    /// Dead (between `death` and the next move).
    pub dead: bool,
    /// Asleep in a bed.
    pub sleeping: bool,
    /// Has a voice session (state other than `none`).
    pub voice: bool,
}

/// One route: listener index into the peer slice, the modes it carries and how it sounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    /// Index of the listener.
    pub listener: usize,
    /// The modes.
    pub modes: ModeSet,
    /// 0..=1.
    pub gain: f32,
    /// -1..=1.
    pub pan: f32,
    /// 0..=1.
    pub muffle: f32,
}

/// The gain at distance `d` within `radius` (`None` beyond it).
pub fn falloff(d: f32, radius: f32, curve: Curve) -> Option<f32> {
    if d > radius {
        return None;
    }
    Some(match curve {
        Curve::Linear => 1.0 - d / radius,
        Curve::Log => 1.0 - (1.0 + d).ln() / (1.0 + radius).ln(),
    })
}

/// The horizontal offset from `from` to `to` taking the short way round a world of `width` tiles (no wrap when
/// unknown).
pub fn wrap_dx(from: i32, to: i32, width: Option<i32>) -> i32 {
    let dx = to - from;
    match width {
        Some(w) if w > 0 => {
            let dx = dx.rem_euclid(w);
            if dx > w / 2 {
                dx - w
            } else {
                dx
            }
        }
        _ => dx,
    }
}

/// Stereo pan of a speaker `dx` tiles to the listener's right.
pub fn pan(dx: i32, radius: f32) -> f32 {
    (dx as f32 / radius).clamp(-1.0, 1.0)
}

/// Muffling for `solid` blocks in the way.
pub fn muffle(solid: u32, per_block: f32) -> f32 {
    (solid as f32 * per_block).clamp(0.0, 1.0)
}

/// The routes of speaker `s`. `solid(from, to)` counts the solid tiles between two tiles; it is only called for
/// listeners within the radius when muffling is on.
pub fn routes_for(
    cfg: &ServerConfig,
    s: usize,
    peers: &[Peer],
    width: Option<i32>,
    solid: &mut dyn FnMut((i32, i32), (i32, i32)) -> u32,
) -> Vec<Route> {
    let sp = &peers[s];
    let mut shared = Vec::new();
    let mut near = Vec::new();
    let proximity_ok =
        sp.allowed.has(Mode::Proximity) && (!sp.dead || cfg.dead_can_talk) && (!sp.sleeping || cfg.sleeping_can_talk);
    for (i, lp) in peers.iter().enumerate() {
        if i == s || !lp.voice {
            continue;
        }
        let mut modes = ModeSet::EMPTY;
        for m in [Mode::Global, Mode::Broadcast] {
            if sp.allowed.has(m) {
                modes = modes.with(m);
            }
        }
        if sp.allowed.has(Mode::Channel) && sp.channel.is_some() && sp.channel == lp.channel {
            modes = modes.with(Mode::Channel);
        }
        if !modes.is_empty() {
            shared.push(Route { listener: i, modes, gain: 1.0, pan: 0.0, muffle: 0.0 });
        }
        let (Some(a), Some(b), true) = (sp.pos, lp.pos, proximity_ok) else { continue };
        let dx = wrap_dx(b.0, a.0, width);
        let dy = a.1 - b.1;
        let d = ((dx * dx + dy * dy) as f32).sqrt();
        let Some(gain) = falloff(d, cfg.radius, cfg.curve) else { continue };
        let route = Route {
            listener: i,
            modes: ModeSet::EMPTY.with(Mode::Proximity),
            gain,
            pan: if cfg.pan { pan(dx, cfg.radius) } else { 0.0 },
            muffle: if cfg.muffle { muffle(solid(b, a), cfg.muffle_per_block) } else { 0.0 },
        };
        near.push((d, route));
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    shared.extend(near.into_iter().map(|(_, r)| r));
    shared.truncate(MAX_ROUTES);
    shared
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(pos: (i32, i32), allowed: &[Mode]) -> Peer {
        Peer { pos: Some(pos), allowed: ModeSet::of(allowed), voice: true, ..Peer::default() }
    }

    fn no_solid(_: (i32, i32), _: (i32, i32)) -> u32 {
        0
    }

    #[test]
    fn falloff_curves() {
        assert_eq!(falloff(0.0, 32.0, Curve::Linear), Some(1.0));
        assert_eq!(falloff(16.0, 32.0, Curve::Linear), Some(0.5));
        assert_eq!(falloff(32.0, 32.0, Curve::Log), Some(0.0));
        assert_eq!(falloff(33.0, 32.0, Curve::Log), None);
        let near = falloff(4.0, 32.0, Curve::Log).unwrap();
        assert!(near < 1.0 - 4.0 / 32.0, "log falls faster near the speaker: {near}");
        assert_eq!(falloff(0.0, 32.0, Curve::Log), Some(1.0));
    }

    #[test]
    fn wrapping_pan_and_muffle() {
        assert_eq!(wrap_dx(10, 20, None), 10);
        assert_eq!(wrap_dx(10, 20, Some(0)), 10);
        assert_eq!(wrap_dx(2, 1020, Some(1024)), -6);
        assert_eq!(wrap_dx(1020, 2, Some(1024)), 6);
        assert_eq!(wrap_dx(0, 512, Some(1024)), 512);
        assert_eq!(pan(16, 32.0), 0.5);
        assert_eq!(pan(-64, 32.0), -1.0);
        assert_eq!(muffle(2, 0.15), 0.3);
        assert_eq!(muffle(10, 0.15), 1.0);
    }

    #[test]
    fn proximity() {
        let cfg = ServerConfig::default();
        let peers = vec![
            peer((0, 0), &[Mode::Proximity]),
            peer((8, 0), &[]),
            peer((-40, 0), &[]),
            peer((0, 4), &[]),
            Peer { voice: false, ..peer((1, 0), &[]) },
            Peer { pos: None, ..peer((0, 0), &[]) },
        ];
        let mut asked = Vec::new();
        let mut solid = |a: (i32, i32), b: (i32, i32)| {
            asked.push((a, b));
            3
        };
        let r = routes_for(&cfg, 0, &peers, None, &mut solid);
        assert_eq!(r.iter().map(|r| r.listener).collect::<Vec<_>>(), [3, 1], "nearest first");
        assert_eq!(asked, [((8, 0), (0, 0)), ((0, 4), (0, 0))], "listener to speaker");
        let r1 = r[1];
        assert_eq!(r1.modes, ModeSet::of(&[Mode::Proximity]));
        assert!((r1.pan + 0.25).abs() < 1e-6, "speaker to the listener's left: {}", r1.pan);
        assert!((r1.muffle - 0.45).abs() < 1e-6);
        assert!(r1.gain > 0.0 && r1.gain < 1.0);
        // Pan and muffle off.
        let cfg2 = ServerConfig { pan: false, muffle: false, ..ServerConfig::default() };
        let mut calls = 0;
        let mut count = |_: (i32, i32), _: (i32, i32)| {
            calls += 1;
            0
        };
        routes_for(&cfg, 0, &peers, None, &mut count);
        let r = routes_for(&cfg2, 0, &peers, None, &mut count);
        assert!(r.iter().all(|r| r.pan == 0.0 && r.muffle == 0.0));
        assert_eq!(calls, 2, "no solid-count without muffling");
        // Wrapping world: x 1020 is 4 tiles left of x 0 in a world of 1024.
        let peers = vec![peer((0, 0), &[Mode::Proximity]), peer((1020, 0), &[])];
        assert_eq!(routes_for(&cfg, 0, &peers, Some(1024), &mut no_solid).len(), 1);
        assert!(routes_for(&cfg, 0, &peers, None, &mut no_solid).is_empty());
        // Not allowed or no position.
        let peers = vec![peer((0, 0), &[Mode::Global]), peer((1, 0), &[])];
        assert_eq!(routes_for(&cfg, 0, &peers, None, &mut no_solid)[0].modes, ModeSet::of(&[Mode::Global]));
        let peers = vec![Peer { pos: None, ..peer((0, 0), &[Mode::Proximity]) }, peer((1, 0), &[])];
        assert!(routes_for(&cfg, 0, &peers, None, &mut no_solid).is_empty());
    }

    #[test]
    fn dead_and_sleeping() {
        let peers = |dead, sleeping| {
            vec![Peer { dead, sleeping, ..peer((0, 0), &[Mode::Proximity, Mode::Global]) }, peer((1, 0), &[])]
        };
        let cfg = ServerConfig::default();
        let only_global = ModeSet::of(&[Mode::Global]);
        for (dead, sleeping) in [(true, false), (false, true)] {
            let r = routes_for(&cfg, 0, &peers(dead, sleeping), None, &mut no_solid);
            assert_eq!(r.len(), 1);
            assert_eq!(r[0].modes, only_global, "dead/sleeping speakers keep global");
        }
        let lenient = ServerConfig { dead_can_talk: true, sleeping_can_talk: true, ..cfg };
        assert_eq!(routes_for(&lenient, 0, &peers(true, true), None, &mut no_solid).len(), 2);
    }

    #[test]
    fn global_channel_broadcast() {
        let cfg = ServerConfig::default();
        let red = Some("red".to_string());
        let peers = vec![
            Peer { channel: red.clone(), ..peer((0, 0), &[Mode::Channel, Mode::Broadcast]) },
            Peer { channel: red.clone(), pos: None, ..peer((0, 0), &[]) },
            Peer { channel: Some("blue".into()), pos: None, ..peer((0, 0), &[]) },
        ];
        let r = routes_for(&cfg, 0, &peers, None, &mut no_solid);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].modes, ModeSet::of(&[Mode::Channel, Mode::Broadcast]));
        assert_eq!((r[0].gain, r[0].pan, r[0].muffle), (1.0, 0.0, 0.0));
        assert_eq!(r[1].modes, ModeSet::of(&[Mode::Broadcast]));
        // No channel: no channel route even when both have none.
        let peers = vec![peer((0, 0), &[Mode::Channel]), peer((500, 0), &[])];
        assert!(routes_for(&cfg, 0, &peers, None, &mut no_solid).is_empty());
    }

    #[test]
    fn capped_at_256() {
        let cfg = ServerConfig { radius: 256.0, ..ServerConfig::default() };
        let mut peers = vec![peer((0, 0), &[Mode::Proximity, Mode::Global])];
        peers.extend((1..200).map(|i| peer((i, 0), &[])));
        let r = routes_for(&cfg, 0, &peers, None, &mut no_solid);
        assert_eq!(r.len(), MAX_ROUTES);
        assert_eq!(r[198].modes, ModeSet::of(&[Mode::Global]));
        assert_eq!((r[199].listener, r[199].modes), (1, ModeSet::of(&[Mode::Proximity])));
    }
}
