//! A `blockheads:api@0.2.0` component (the frozen WIT, before `ui`) that calls every 0.2 import once in `on-enable`
//! and logs what it gets.
//! It depends on the 0.3 test guest loaded as `tour` (service `tour`, channel `tour:c`).

wit_bindgen::generate!({ path: "../../../../wit/compat/blockheads-api-0.2.0", world: "server-mod" });

use blockheads::api::channels::{self, Source, Target};
use blockheads::api::commands::{self, CommandOutcome, CommandSpec};
use blockheads::api::events::{self, CommandSender, Event, EventKind, Mutation, Outcome, RejectReason};
use blockheads::api::log::{self, Level};
use blockheads::api::players::{self, Role};
use blockheads::api::types::Priority;
use blockheads::api::{host, scheduler, services, storage};
use exports::blockheads::api::guest::Guest;

fn show<T: std::fmt::Debug>(what: &str, v: T) {
    log::log(Level::Info, &format!("{what} = {v:?}"));
}

struct Old;

impl Guest for Old {
    fn on_enable() -> Result<(), String> {
        show(
            "api",
            (host::api_version(), host::mod_id(), host::mod_version(), host::current_side(), host::host_name()),
        );
        for l in [Level::Trace, Level::Debug, Level::Warn, Level::Error] {
            log::log(l, "lvl");
        }
        let p = players::get("p1").ok_or("p1 missing")?;
        show("p", (p.id(), p.alias(), p.is_online(), p.role(), p.send_chat("hi from 0.2"), p.kick("old kick")));
        show("pos", (p.pos(), p.value_handle() < 1000, players::from_value_handle(p.value_handle()).map(|q| q.id())));
        show("find", players::find_by_alias("alice").map(|p| p.id()));
        show("online", players::online().len());
        show("broadcast", players::broadcast("0.2 says hi"));
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
            events::subscribe(kind, Priority::Low, false).map_err(|e| format!("{e:?}"))?;
        }
        let s = events::subscribe(EventKind::Tick, Priority::Lowest, true).map_err(|e| format!("{e:?}"))?;
        show("unsubscribe", events::unsubscribe(s));
        show("custom", events::subscribe_custom("tour:ping", Priority::Monitor, false).is_ok());
        show("fire", events::fire_custom("old:ping", &[0xf6]));
        for (name, role) in
            [("old", Role::Player), ("old-m", Role::Moderator), ("old-a", Role::Admin), ("old-o", Role::Owner)]
        {
            let spec = CommandSpec {
                name: name.into(),
                aliases: vec![],
                description: "d".into(),
                usage: format!("/{name}"),
                min_role: role,
                console: true,
            };
            let id = commands::register(&spec).map_err(|e| format!("{e:?}"))?;
            if name != "old" {
                show("unregister", commands::unregister(id));
            }
        }
        show("tasks", (scheduler::run_later(1), scheduler::run_every(1, 2), scheduler::run_next_poll()));
        show("cancel", scheduler::cancel(12345));
        show("storage", (storage::set("k", b"v"), storage::get("k"), storage::keys(""), storage::delete("k")));
        show("provide", services::provide("old", "1.0.0"));
        show("call", services::call("tour", "^1", "echo", &[0x81, 0x01]));
        let c = channels::open("tour:c").map_err(|e| format!("{e:?}"))?;
        show("send", channels::send(c, &Target::Local, b"from 0.2"));
        show("close", channels::close(9999));
        Ok(())
    }

    fn on_disable() {
        log::log(Level::Info, "old disabled");
    }

    fn handle_event(_sub: u32, ev: Event) -> Outcome {
        let mut out = Outcome { cancelled: false, mutation: None };
        match ev {
            Event::Chat(c) => {
                show("old chat", (c.player.id(), &c.text));
                out.mutation = Some(Mutation::ChatText(format!("{} (0.2)", c.text)));
            }
            Event::Command(c) => {
                show("old command", &c.name);
                out.cancelled = c.name == "swallow";
            }
            Event::PlayerPreLogin(e) => {
                show("old pre-login", (&e.player_id, &e.alias));
                out.cancelled = true;
                out.mutation = Some(Mutation::RejectReason(RejectReason::Banned));
            }
            Event::Damage(d) => {
                show("old damage", (d.blockhead, d.amount));
                out.mutation = Some(Mutation::DamageAmount(d.amount * 2.0));
            }
            Event::Custom(c) => {
                show("old custom", (&c.name, &c.source));
                out.mutation = Some(Mutation::CustomPayload(c.payload));
            }
            other => show("old event", other),
        }
        out
    }

    fn run_command(_id: u32, sender: CommandSender, label: String, args: Vec<String>) -> CommandOutcome {
        let who = match sender {
            CommandSender::Console => "console".to_string(),
            CommandSender::Player(p) => p.alias(),
        };
        CommandOutcome { handled: !args.is_empty(), reply: vec![format!("{who} ran {label}")] }
    }

    fn run_task(id: u64) {
        show("old task", id);
    }

    fn call_service(name: String, method: String, args: Vec<u8>) -> Result<Vec<u8>, String> {
        match method.as_str() {
            "echo" => Ok(args),
            _ => Err(format!("{name}: no method {method}")),
        }
    }

    fn on_channel_message(channel: u32, origin: Source, data: Vec<u8>) {
        let from = match origin {
            Source::Local(m) => m,
            Source::Server => "server".into(),
            Source::Player(p) => p.id(),
        };
        show("old message", (channel, from, String::from_utf8_lossy(&data)));
    }
}

export!(Old);
