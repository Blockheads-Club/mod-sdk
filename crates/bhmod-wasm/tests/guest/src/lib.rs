//! The bhmod-wasm test guest. `host::mod_id()` picks the behaviour:
//! - `tour`: calls every import once in `on-enable` and logs the results; prints to stdout/stderr.
//! - `fail`: `on-enable` returns an error.
//! - `trap`: commands panic; events after the trap are refused by the host.
//! - `spin`: the tick handler never returns (host deadline).
//! - `oom`: the tick handler allocates past the memory limit.

use blockheads_mod::bindings::blockheads::api::{channels, events as wev, services};
use blockheads_mod::events::On;
use blockheads_mod::prelude::*;

struct Guest;

fn show<T: std::fmt::Debug>(what: &str, v: T) {
    log::info(&format!("{what} = {v:?}"));
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

    for kind in [
        EventKind::PlayerJoin,
        EventKind::PlayerQuit,
        EventKind::Chat,
        EventKind::Command,
        EventKind::Tick,
        EventKind::WorldLoaded,
        EventKind::WorldSave,
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
            ev.set_cancelled(cancel);
        })?;
    }
    for prio in [Priority::Lowest, Priority::Low, Priority::Normal, Priority::Highest, Priority::Monitor] {
        let id = events::on_with(EventKind::Tick, On::priority(prio), |_| {})?;
        show("off", events::off(id));
    }
    show("off-missing", events::off(999));
    show("custom", wev::subscribe_custom("tour:x", Priority::Normal, false));
    show("fire", wev::fire_custom("tour:x", &[]));

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
    let every = scheduler::run_every(1, 1, || log::info("every ran"))?;
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
    show("call", services::call("x", "^1", "m", &[]));
    show("open", channels::open("tour:c"));
    show("send", channels::send(1, &channels::Target::Local, &[1]));
    show("close", channels::close(1));
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
            "fail" => Err("nope".into()),
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
}

blockheads_mod::export_mod!(Guest);
