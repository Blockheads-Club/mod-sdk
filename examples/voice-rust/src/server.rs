//! The server half: settings, routes at 5 Hz, `/voice`, mutes and gains, channels, the `voice` service.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use blockheads_mod::cbor::Value;
use blockheads_mod::prelude::*;
use blockheads_mod::{terrain, voice as wv};

use crate::channels::{list_lines, Channels};
use crate::command::{self, format_duration, VoiceCmd};
use crate::config::ServerConfig;
use crate::glue::{log_err, now_ms, on_custom, role_of, wit_activation, wit_mode};
use crate::ledger::{Ledger, Mute};
use crate::model::{ModeSet, Role as R};
use crate::payload::changed_keys;
use crate::roles;
use crate::routing::{routes_for, Peer};

/// Ticks between permission-group refreshes (5 s).
const GROUP_REFRESH: u64 = 25;
const LEDGER_KEY: &str = "ledger";

#[derive(Default)]
struct Server {
    cfg: ServerConfig,
    channels: Channels,
    ledger: Ledger,
    /// Permission groups per player id (from the `permissions` service).
    groups: BTreeMap<String, BTreeSet<String>>,
    /// What `set-allowed` last told each player (modes, activations).
    allowed: BTreeMap<String, (u8, u8)>,
    /// Speakers whose routes are empty already.
    silent: BTreeSet<String>,
    tick: u64,
}

thread_local! {
    static S: RefCell<Server> = RefCell::new(Server::default());
}

fn with<T>(f: impl FnOnce(&mut Server) -> T) -> T {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn save_ledger(l: &Ledger) {
    log_err("saving mutes", storage::set(LEDGER_KEY, &l.to_value().encode()));
}

/// Applies the settings that are global to the host.
fn apply_global(cfg: &ServerConfig) {
    let caps = wv::Caps {
        max_bitrate: cfg.max_bitrate,
        max_speakers: cfg.max_speakers,
        max_frames_per_sec: cfg.max_frames_per_sec,
    };
    log_err("voice caps", wv::set_caps(caps));
    // Voice frames always use the voice UDP port (`mode` is ignored by the host); `None` = game port + 1.
    let port = wv::TransportConfig { mode: wv::Transport::Udp, udp_port: cfg.udp_port };
    log_err("voice UDP port", wv::set_transport(port));
    let i = cfg.icons;
    log_err(
        "voice icons",
        wv::set_icons(wv::IconVisibility {
            speaking: i.speaking,
            no_voice: i.no_voice,
            self_mute: i.self_mute,
            deafen: i.deafen,
            admin_mute: i.admin_mute,
        }),
    );
    log_err("broadcast/deafen", wv::set_broadcast_overrides_deafen(cfg.broadcast_overrides_deafen));
}

/// Reads the settings and applies them (on enable and after every change).
fn reload() {
    let cfg = ServerConfig::from_value(&settings::all());
    apply_global(&cfg);
    with(|s| {
        s.channels.set_fixed(&cfg.channels);
        s.allowed.clear();
        s.cfg = cfg;
    });
    for p in players::online() {
        apply_player(&p);
    }
    refresh_groups();
}

/// Re-applies a player's persisted mute and gain.
fn apply_player(p: &Player) {
    let (mute, gain) = with(|s| {
        let id = p.id();
        (s.ledger.mutes.get(&id).cloned(), s.ledger.gain_of(&id, &p.alias(), &s.cfg.speaker_gain))
    });
    if let Some(m) = mute {
        log_err("voice mute", wv::mute(p, m.until_ms));
    }
    log_err("speaker gain", wv::set_speaker_gain(p, gain));
}

/// Asks the permissions mod which matrix groups each online player is in.
fn refresh_groups() {
    let groups = with(|s| roles::groups(&s.cfg));
    let mut out = BTreeMap::new();
    for p in players::online() {
        let mut set = BTreeSet::new();
        for g in &groups {
            let who = Value::Map(vec![("$h".into(), Value::Int(p.value_handle() as i64))]);
            let args = blockheads_mod::cbor::args(&[who, Value::Text(roles::group_node(g))]);
            let yes = services::call("permissions", "^1", "has", &args)
                .ok()
                .and_then(|r| Value::decode(&r).ok())
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if yes {
                set.insert(g.clone());
            }
        }
        out.insert(p.id(), set);
    }
    with(|s| s.groups = out);
}

fn allowed_for(s: &Server, p: &Player) -> ModeSet {
    let empty = BTreeSet::new();
    let groups = s.groups.get(&p.id()).unwrap_or(&empty);
    roles::allowed_modes(&s.cfg, role_of(p), &|g| groups.contains(g))
}

/// One routing pass (every tick, 5 Hz).
fn tick() {
    let now = now_ms();
    let (expired, refresh) = with(|s| {
        s.tick += 1;
        let gone = s.ledger.expire(now);
        if !gone.is_empty() {
            save_ledger(&s.ledger);
        }
        (gone, s.tick % GROUP_REFRESH == 0 && !roles::groups(&s.cfg).is_empty())
    });
    for id in expired {
        if let Some(p) = players::get(&id) {
            let _ = wv::unmute(&p);
        }
    }
    if refresh {
        refresh_groups();
    }
    route_all();
}

/// Publishes every speaker's routes (each tick, and at once when someone sleeps, wakes or respawns).
fn route_all() {
    let online = players::online();
    let states: Vec<wv::VoiceState> = online.iter().map(|p| wv::state(p).unwrap_or(wv::VoiceState::None)).collect();
    let peers: Vec<Peer> = with(|s| {
        online
            .iter()
            .zip(&states)
            .map(|(p, st)| {
                let id = p.id();
                let voice = *st != wv::VoiceState::None;
                let allowed = allowed_for(s, p);
                let acts = s.cfg.activations;
                if voice && s.allowed.get(&id) != Some(&(allowed.0, acts.0)) {
                    let modes: Vec<_> = allowed.modes().into_iter().map(wit_mode).collect();
                    let a: Vec<_> = acts.list().into_iter().map(wit_activation).collect();
                    if wv::set_allowed(p, &modes, &a).is_ok() {
                        s.allowed.insert(id.clone(), (allowed.0, acts.0));
                    }
                }
                Peer {
                    pos: p.pos().map(|q| (q.x, q.y)),
                    allowed,
                    channel: s.channels.channel_of(&id),
                    dead: !p.alive(),
                    sleeping: p.sleeping(),
                    voice,
                }
            })
            .collect()
    });
    let cfg = with(|s| s.cfg.clone());
    let width = terrain::world_width();
    for (i, p) in online.iter().enumerate() {
        if !peers[i].voice {
            continue;
        }
        let routes =
            routes_for(&cfg, i, &peers, width, &mut |a, b| terrain::solid_count(a.0, a.1, b.0, b.1).unwrap_or(0));
        let id = p.id();
        let silent = routes.is_empty();
        if silent && with(|s| s.silent.contains(&id)) {
            continue;
        }
        let wit: Vec<wv::Route<'_>> = routes
            .iter()
            .map(|r| wv::Route {
                listener: &online[r.listener],
                modes: r.modes.modes().into_iter().map(wit_mode).collect(),
                gain: r.gain,
                pan: r.pan,
                muffle: r.muffle,
            })
            .collect();
        if wv::set_routes(p, &wit).is_ok() {
            with(|s| if silent { s.silent.insert(id) } else { s.silent.remove(&id) });
        }
    }
}

fn forget(id: &str) {
    with(|s| {
        s.channels.leave(id);
        s.groups.remove(id);
        s.allowed.remove(id);
        s.silent.remove(id);
    });
}

fn sender_role(sender: &Sender<'_>) -> R {
    sender.player().map_or(R::Owner, role_of)
}

fn run(sender: &Sender<'_>, args: &[String]) -> Reply {
    let cmd = match command::parse(args) {
        Ok(c) => c,
        Err(e) => return Reply::usage([e]),
    };
    if sender_role(sender) < cmd.min_role() {
        return Reply::lines(["You may not use that."]);
    }
    let me = sender.player();
    if cmd.needs_player() && me.is_none() {
        return Reply::lines(["Only players can do that."]);
    }
    match cmd {
        VoiceCmd::Help => Reply::lines(command::USAGE.iter().copied()),
        VoiceCmd::Mute { player, duration_ms } => {
            let Some(p) = players::find_by_alias(&player) else {
                return Reply::lines([format!("{player} is not online.")]);
            };
            let until_ms = duration_ms.map(|d| now_ms() + d);
            if let Err(e) = wv::mute(&p, until_ms) {
                return Reply::lines([format!("Could not mute: {}", String::from(e))]);
            }
            with(|s| {
                s.ledger.mutes.insert(p.id(), Mute { alias: p.alias(), until_ms });
                save_ledger(&s.ledger);
            });
            let how = duration_ms.map_or("until unmuted".to_string(), |d| format!("for {}", format_duration(d)));
            Reply::lines([format!("{} is muted {how}.", p.alias())])
        }
        VoiceCmd::Unmute { player } => {
            let online = players::find_by_alias(&player);
            let id = online.as_ref().map(|p| p.id()).or_else(|| with(|s| s.ledger.muted_id_by_alias(&player)));
            let was = with(|s| {
                let was = id.as_ref().is_some_and(|id| s.ledger.mutes.remove(id).is_some());
                save_ledger(&s.ledger);
                was
            });
            let live = online.is_some_and(|p| wv::unmute(&p).unwrap_or(false));
            Reply::lines([if was || live {
                format!("{player} is unmuted.")
            } else {
                format!("{player} was not muted.")
            }])
        }
        VoiceCmd::Gain { player, percent } => {
            let Some(p) = players::find_by_alias(&player) else {
                return Reply::lines([format!("{player} is not online.")]);
            };
            with(|s| {
                s.ledger.set_gain(&p.id(), &p.alias(), percent);
                save_ledger(&s.ledger);
            });
            apply_player(&p);
            Reply::lines([format!("{} now sounds at {percent} %.", p.alias())])
        }
        VoiceCmd::List => {
            let current = me.map(|p| p.id());
            let lines = with(|s| {
                let mine = current.and_then(|id| s.channels.channel_of(&id));
                list_lines(&s.channels.list(), mine.as_deref())
            });
            Reply::lines(lines)
        }
        VoiceCmd::Leave => {
            let id = me.map(Player::id).unwrap_or_default();
            match with(|s| s.channels.leave(&id)) {
                Some(name) => Reply::lines([format!("You left {name}.")]),
                None => Reply::lines([crate::channels::ChanError::NotInChannel.message()]),
            }
        }
        VoiceCmd::Join { name, password } => {
            let p = me.expect("checked");
            let bypass = role_of(p) >= R::Admin;
            match with(|s| s.channels.join(&p.id(), &name, password.as_deref(), bypass)) {
                Ok(shown) => Reply::lines([format!("You joined {shown}. Talk with your channel key (B).")]),
                Err(e) => Reply::lines([e.message()]),
            }
        }
        VoiceCmd::Create { name, password } => {
            let p = me.expect("checked");
            let r = with(|s| {
                let allowed = s.cfg.player_created || role_of(p) >= R::Admin;
                s.channels.create(&p.id(), &name, password.as_deref(), allowed, s.cfg.max_per_player)
            });
            match r {
                Ok(()) => Reply::lines([format!("Created {name}; you are in it.")]),
                Err(e) => Reply::lines([e.message()]),
            }
        }
    }
}

/// The `voice` service for clan / team mods: `channel-of(player)`, `join-channel(player, name)`,
/// `leave-channel(player)`.
pub fn call_service(method: &str, args: &Value) -> Result<Value, String> {
    let arg = |i: usize| match args {
        Value::List(l) => l.get(i),
        _ => None,
    };
    let player = |i: usize| -> Result<Player, String> {
        let h = arg(i).and_then(|v| v.get("$h")).and_then(Value::as_i64).ok_or("argument 1: a player")?;
        players::from_value_handle(h as u64).ok_or_else(|| "that player left".to_string())
    };
    match method {
        "channel-of" => {
            let p = player(0)?;
            Ok(with(|s| s.channels.channel_of(&p.id()).and_then(|k| s.channels.display(&k).map(str::to_string)))
                .map_or(Value::Null, Value::Text))
        }
        "join-channel" => {
            let p = player(0)?;
            let name = arg(1).and_then(Value::as_str).ok_or("argument 2: a channel name")?;
            // A channel another mod asks for is created on demand (fixed rules do not apply to mods).
            let ok = with(|s| {
                let id = p.id();
                if s.channels.display(name).is_none() {
                    s.channels.create(&id, name, None, true, u32::MAX).is_ok()
                } else {
                    s.channels.join(&id, name, None, true).is_ok()
                }
            });
            Ok(Value::Bool(ok))
        }
        "leave-channel" => {
            let p = player(0)?;
            Ok(Value::Bool(with(|s| s.channels.leave(&p.id()).is_some())))
        }
        other => Err(format!("unknown method {other}")),
    }
}

pub fn enable() -> Result<(), String> {
    let ledger = storage::get(LEDGER_KEY).and_then(|b| Value::decode(&b).ok()).map(|v| Ledger::from_value(&v));
    with(|s| s.ledger = ledger.unwrap_or_default());
    services::provide("voice", "1.0.0")?;
    reload();
    on_custom("voice:settings-changed", |v| {
        log::debug(&format!("settings changed: {:?}", changed_keys(v)));
        reload();
    })?;
    events::on(EventKind::PlayerJoin, |ctx| {
        if let Some(p) = ctx.player() {
            forget(&p.id());
            apply_player(p);
        }
    })?;
    events::on(EventKind::PlayerQuit, |ctx| {
        if let Some(p) = ctx.player() {
            forget(&p.id());
        }
    })?;
    // Sleeping and death change who may talk nearby: re-route at once instead of at the next tick.
    for name in ["player:sleep", "player:wake", "player:respawn"] {
        on_custom(name, |_| route_all())?;
    }
    let cmd = Command::new("voice")
        .description("Voice chat: channels, mutes and gains.")
        .usage("/voice help")
        .min_role(Role::Player)
        .console(true);
    commands::register(cmd, |sender, _label, args| run(sender, args))?;
    scheduler::run_every(1, 1, tick)?;
    Ok(())
}

pub fn disable() {
    with(|s| save_ledger(&s.ledger));
}
