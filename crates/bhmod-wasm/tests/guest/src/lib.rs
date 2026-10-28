//! The bhmod-wasm test guest. `host::mod_id()` picks the behaviour:
//! - `tour`: calls every import once in `on-enable` and logs the results; prints to stdout/stderr.
//! - `gear`: calls every `content` import (API 0.3) with the mod's own cosmetic `gear:cap`, and the `settings`
//!   imports (API 0.3.3) with its setting `level`.
//! - `fail`: `on-enable` returns an error.
//! - `trap`: commands panic; events after the trap are refused by the host.
//! - `spin`: the tick handler never returns (host deadline).
//! - `oom`: the tick handler allocates past the memory limit.

use blockheads_mod::bindings::blockheads::api::events as wev;
use blockheads_mod::cbor;
use blockheads_mod::events::On;
use blockheads_mod::prelude::*;

struct Guest;

fn show<T: std::fmt::Debug>(what: &str, v: T) {
    log::info(&format!("{what} = {v:?}"));
}

fn gear() -> Result<(), String> {
    use blockheads_mod::content::{self, Kind};
    let p = players::get("p1").ok_or("p1 missing")?;
    show("cosmetics", content::cosmetics());
    show("id-of", (content::id_of(Kind::Cosmetic, "gear:cap"), content::id_of(Kind::Item, "gear:cap")));
    show("key-of", (content::key_of(Kind::Cosmetic, 1), content::key_of(Kind::Block, 1)));
    show("equip", content::equip(&p, "gear:cap"));
    show("equipped", content::equipped(&p));
    show("equip-other", content::equip(&p, "other:x").map_err(String::from));
    show("unequip", content::unequip(&p, "hat"));
    show("unequip-bad", content::unequip(&p, "cape").map_err(String::from));
    show("equipped-after", content::equipped(&p));
    show("give-item", content::give_item(&p, "gear:lamp", 2));
    show("give-other", content::give_item(&p, "other:lamp", 1).map_err(String::from));
    show("place-block", content::place_block(4, -5, "gear:brick"));
    show("block-at", (content::block_at(4, -5), content::block_at(0, 0)));
    use blockheads_mod::settings;
    show("setting", (settings::get("level"), settings::get("nope")));
    show("settings-all", settings::all());
    show("setting-set", settings::set("level", &cbor::Value::Int(7)));
    show("setting-bad", settings::set("level", &cbor::Value::from("x")).map_err(String::from));
    show("setting-after", settings::get("level"));
    Ok(())
}

/// Every API 0.3.4 import: `voice`, `terrain`, `input` and the `ui` additions.
fn vox() -> Result<(), String> {
    use blockheads_mod::voice as v;
    use blockheads_mod::{input, terrain, ui};
    let a = players::get("p1").ok_or("p1 missing")?;
    let b = players::get("p2").ok_or("p2 missing")?;
    show("state", v::state(&a));
    show("icons", v::icons());
    let route = v::Route { listener: &b, modes: vec![v::Mode::Proximity, v::Mode::Broadcast], gain: 0.5, pan: 0.0, muffle: 0.0 };
    show("routes", v::set_routes(&a, &[route]));
    show("mute", v::mute(&b, Some(5)));
    show("unmute", v::unmute(&b));
    show("gain", v::set_speaker_gain(&a, 120));
    show("caps", v::set_caps(v::Caps { max_bitrate: 16_000, max_speakers: 4, max_frames_per_sec: 50 }));
    show("transport", v::set_transport(v::TransportConfig { mode: v::Transport::Inband, udp_port: None }));
    show("allowed", v::set_allowed(&a, &[v::Mode::Global, v::Mode::Channel], &[v::Activation::Vad]));
    let icons = v::IconVisibility { speaking: true, no_voice: false, self_mute: true, deafen: true, admin_mute: true };
    show("set-icons", v::set_icons(icons));
    show("override", v::set_broadcast_overrides_deafen(true));
    show("default-mode", v::set_default_mode(v::Mode::Global));
    show("activation", v::set_activation(v::Activation::Toggle));
    show("talk", v::talk(Some(v::Mode::Global), true));
    show("mic", v::set_mic(true));
    show("self-mute", v::set_self_mute(true));
    show("deafen", v::set_deafen(true));
    show("vad", v::set_vad_threshold(-40.0));
    show("master", (v::set_master(90), v::master()));
    show("volume", (v::set_player_volume(&a, 90), v::player_volume(&a)));
    show("mute-player", (v::mute_player(&a, true), v::player_muted(&a)));
    show("solid", terrain::solid_count(0, 0, 4, 0));
    show("bind", (input::bind("ptt", "v"), input::unbind("ptt"), input::key_of("ptt")));
    show("overhead", (ui::set_overhead(&a, ui::OverheadIcon::Speaking), ui::clear_overhead(&a)));
    let button = ui::Button { icon: "mic".into(), label: None, x: 0.5, y: 0.5, size: 64, opacity: 80 };
    show("button", (ui::set_button("ptt", &button), ui::remove_button("ptt")));
    show("settings-screen", ui::open_settings("vox"));
    show("gaps", (host::platform(), a.sleeping(), a.alive(), players::local_player().is_none(), players::has_mod(&a, "vox")));
    show("gaps2", (terrain::world_width(), ui::has_touch()));
    show("client-state", (v::allowed().is_err(), v::self_muted(), v::deafened(), v::mic()));
    Ok(())
}

fn tour() -> Result<(), String> {
    show("api", host::api_version());
    show("id", host::mod_id());
    show("version", host::mod_version());
    show("side", host::current_side());
    show("host", host::host_name());
    log::trace("t");
    log::debug("d");
    log::warn("w");
    log::error("e");
    log::at(log::LogLevel::Info, "at");
    println!("to stdout");
    eprintln!("to stderr");

    show("get-missing", players::get("nobody").is_none());
    let p = players::get("p1").ok_or("p1 missing")?;
    show("p.id", p.id());
    show("p.alias", p.alias());
    show("p.online", p.is_online());
    show("p.role", p.role());
    show("p.chat", p.send_chat("hi p1"));
    show("by-alias", players::find_by_alias("alice").map(|p| p.id()));
    show("online", players::online().iter().map(|p| p.id()).collect::<Vec<_>>());
    show("broadcast", players::broadcast("to all"));
    let k = players::get("p2").ok_or("p2 missing")?;
    show("kick", k.kick("bye"));

    show("p.pos", p.pos());
    let h = p.value_handle();
    show("from-handle", players::from_value_handle(h).map(|p| p.id()));
    show("from-stale-handle", players::from_value_handle(999_999).is_none());
    for kind in [
        EventKind::PlayerJoin,
        EventKind::PlayerQuit,
        EventKind::Chat,
        EventKind::Command,
        EventKind::Tick,
        EventKind::WorldLoaded,
        EventKind::WorldSave,
        EventKind::PlayerPreLogin,
        EventKind::BlockPlace,
        EventKind::BlockBreak,
        EventKind::BlockChanged,
        EventKind::ObjectPlace,
        EventKind::Damage,
        EventKind::Death,
        EventKind::PlayerMove,
    ] {
        events::on_with(kind, On::priority(Priority::High).ignore_cancelled(), move |ev| {
            let who = ev.player().map(|p| (p.id(), p.alias(), p.role()));
            show("event", (ev.kind(), who, ev.text().map(str::to_string), ev.cancelled()));
            let mut cancel = false;
            if let blockheads_mod::events::Event::Command(c) = ev.event() {
                show("command-event", (&c.name, &c.args, &c.raw));
                cancel = c.name == "x";
            }
            ev.set_text("changed");
            ev.set_reject_reason(blockheads_mod::events::RejectReason::Banned);
            ev.set_damage(0.25);
            show("ev", ev.event());
            let k = ev.kind();
            ev.set_cancelled(cancel || k == Some(EventKind::PlayerPreLogin) || k == Some(EventKind::BlockBreak));
        })?;
    }
    for prio in [Priority::Lowest, Priority::Low, Priority::Normal, Priority::Highest, Priority::Monitor] {
        let id = events::on_with(EventKind::Tick, On::priority(prio), |_| {})?;
        show("off", events::off(id));
    }
    show("off-missing", events::off(999));
    events::on_custom("tour:ping", On::default(), |ev| {
        show("custom-event", (ev.kind(), ev.cancelled()));
        ev.cancel();
    })?;
    show("custom-bad", wev::subscribe_custom("nocolon", Priority::Normal, false));
    show("fire", events::fire_custom("tour:ping", &[0xf6]));
    show("fire-bad", events::fire_custom("other:ping", &[0xf6]));

    let cmd = Command::new("tour").alias("t").description("d").usage("/tour x").min_role(Role::Admin).console(false);
    show("spec", cmd.spec());
    let id = commands::register(cmd, |sender, label, args| {
        let lines = vec![sender.name(), label.to_string(), args.join(",")];
        if args.is_empty() {
            Reply::usage(lines)
        } else {
            Reply::lines(lines)
        }
    })?;
    show("command", id);
    for (name, role) in [("rp", Role::Player), ("rm", Role::Moderator), ("ro", Role::Owner)] {
        let id = commands::register(Command::new(name).min_role(role), |_, _, _| Reply::ok())?;
        show("unregister", commands::unregister(id));
    }
    show("bad-command", commands::register(Command::new("help"), |_, _, _| Reply::ok()).map_err(String::from));

    let later = scheduler::run_later(1, || log::info("later ran"))?;
    let every = scheduler::run_every(1, 1, || {
        log::info("every ran");
        let _ = events::fire_custom("tour:ping", &[0xf6]);
    })?;
    let poll = scheduler::run_next_poll(|| log::info("poll ran"))?;
    show("tasks", (later, every, poll));
    show("cancel-poll", scheduler::cancel(poll));
    show("bad-every", scheduler::run_every(1, 0, || {}).map_err(String::from));

    show("set", storage::set("k1", b"v1"));
    show("set-string", storage::set_string("k2", "v2"));
    show("get", storage::get("k1"));
    show("get-string", storage::get_string("k2"));
    show("keys", storage::keys("k"));
    show("delete", storage::delete("k1"));
    show("bad-key", storage::set("", b"x").map_err(String::from));

    show("provide", services::provide("tour", "1.0.0"));
    show("call-missing", services::call("x", "^1", "m", &[0x80]));
    show("call-version", services::call("tour", "^2", "m", &[0x80]));
    show("call-self", services::call("tour", "^1", "echo", &[0x80]));
    // Remote service calls (0.3.1): the answer arrives later, in the closure.
    let id =
        services::call_remote(&services::Target::Player(&p), "hud", "^1", "title", &[cbor::Value::Int(7)], 0, |r| {
            show("remote-answer", r)
        });
    show("call-remote", &id);
    let gone = services::call_remote(&services::Target::Player(&p), "hud", "^1", "gone", &[], 1000, |_| {});
    show("cancel-remote", gone.map(services::cancel_remote));
    show("cancel-remote-unknown", services::cancel_remote(999));
    show(
        "call-remote-server",
        services::call_remote(&services::Target::Server, "hud", "^1", "title", &[], 0, |_| {}).map_err(String::from),
    );
    show("caller", services::caller().map(|c| c.mod_id));
    // The raw import checks what the SDK helper cannot get wrong: arguments must be a canonical CBOR list.
    let raw = |args: &[u8]| {
        blockheads_mod::bindings::blockheads::api::services::call_remote(
            &services::Target::Player(&p),
            "hud",
            "^1",
            "title",
            args,
            0,
        )
        .map_err(String::from)
    };
    show("call-remote-garbage", raw(&[0xff]));
    show("call-remote-scalar", raw(&[0x01]));
    let c = channels::open("tour:c").map_err(String::from)?;
    show("send-local", channels::send(c, &channels::Target::Local, &[1]));
    show("send-server", channels::send(c, &channels::Target::Server, &[2]).map_err(String::from));
    show("send-player", channels::send(c, &channels::Target::Player(&p), &[3]));
    show("send-all", channels::send(c, &channels::Target::AllPlayers, &[4]));
    show("close", channels::close(c));
    show("reopen", channels::open("tour:c"));
    show(
        "errors",
        [
            String::from(ApiError::NotPermitted("p".into())),
            String::from(ApiError::InvalidArgument("a".into())),
            String::from(ApiError::LimitExceeded("l".into())),
            String::from(ApiError::StaleHandle),
            String::from(ApiError::Unsupported),
            String::from(ApiError::Internal("i".into())),
        ],
    );
    Ok(())
}

impl Mod for Guest {
    fn on_enable() -> Result<(), String> {
        match host::mod_id().as_str() {
            "tour" => tour(),
            "gear" => gear(),
            "vox" => vox(),
            "fail" => Err("nope".into()),
            "caller" => {
                show("echo", services::call("old", "^1", "echo", &[0x81, 0x02]));
                show("nope", services::call("old", "^1", "nope", &[0x80]));
                Ok(())
            }
            "trap" => {
                commands::register(Command::new("boom"), |_, _, _| panic!("boom"))?;
                events::on(EventKind::Tick, |_| log::info("tick"))?;
                Ok(())
            }
            "spin" => {
                events::on(EventKind::Tick, |_| loop {
                    std::hint::black_box(());
                })?;
                Ok(())
            }
            "oom" => {
                events::on(EventKind::Tick, |_| {
                    let v = vec![1u8; 256 << 20];
                    log::info(&format!("allocated {}", v.len()));
                })?;
                Ok(())
            }
            other => Err(format!("unknown test mod {other}")),
        }
    }

    fn on_disable() {
        log::info("disabled");
    }

    fn call_service(name: String, method: String, args: Vec<u8>) -> Result<Vec<u8>, String> {
        match method.as_str() {
            "echo" => Ok(args),
            // Not a value: the host refuses to pass it on.
            "garbage" => Ok(vec![0xff]),
            "whoami" => {
                let c = services::caller().ok_or("not a remote call")?;
                let player = c.player.map(|p| p.id());
                Ok(cbor::Value::Map(vec![
                    ("mod".into(), c.mod_id.into()),
                    ("player".into(), player.map_or(cbor::Value::Null, Into::into)),
                ])
                .encode())
            }
            _ => Err(format!("{name}: no method {method}")),
        }
    }

    fn on_channel_message(channel: u32, origin: channels::Source<'_>, data: Vec<u8>) {
        let from = match origin {
            channels::Source::Local(m) => m,
            channels::Source::Server => "server".into(),
            channels::Source::Player(p) => p.id(),
        };
        show("message", (channel, from, data));
    }
}

blockheads_mod::export_mod!(Guest);
