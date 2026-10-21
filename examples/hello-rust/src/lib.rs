//! hello-rust: a minimal Blockheads server mod written in Rust and compiled to a WASM component.
//!
//! It adds a `/hello` command and greets players who join.

wit_bindgen::generate!({
    path: "../../wit/blockheads-api",
    world: "server-mod",
});

use blockheads::api::commands::{self, CommandOutcome, CommandSpec};
use blockheads::api::events::{self, CommandSender, Event, EventKind, Outcome};
use blockheads::api::log::{self, Level};
use blockheads::api::players::Role;
use blockheads::api::types::Priority;
use exports::blockheads::api::guest::Guest;

struct HelloMod;

impl Guest for HelloMod {
    fn on_enable() -> Result<(), String> {
        events::subscribe(EventKind::PlayerJoin, Priority::Normal, false).map_err(|e| format!("{e:?}"))?;
        commands::register(&CommandSpec {
            name: "hello".into(),
            aliases: vec!["hi".into()],
            description: "Says hello.".into(),
            usage: "/hello".into(),
            min_role: Role::Player,
            console: true,
        })
        .map_err(|e| format!("{e:?}"))?;
        log::log(Level::Info, "hello-rust enabled");
        Ok(())
    }

    fn on_disable() {
        log::log(Level::Info, "hello-rust disabled");
    }

    fn handle_event(_sub: u32, ev: Event) -> Outcome {
        if let Event::PlayerJoin(join) = ev {
            let _ = join.player.send_chat(&format!("Hello, {}! This server runs Rust mods.", join.player.alias()));
        }
        Outcome { cancelled: false, mutation: None }
    }

    fn run_command(_id: u32, sender: CommandSender, _label: String, _args: Vec<String>) -> CommandOutcome {
        let who = match sender {
            CommandSender::Console => "console".to_string(),
            CommandSender::Player(p) => p.alias(),
        };
        CommandOutcome { handled: true, reply: vec![format!("Hello, {who}!")] }
    }

    fn run_task(_id: u64) {}

    fn call_service(_name: String, _method: String, _args: Vec<u8>) -> Result<Vec<u8>, String> {
        Err("hello-rust provides no services".into())
    }

    fn on_channel_message(_channel: u32, _origin: blockheads::api::channels::Source, _data: Vec<u8>) {}
}

export!(HelloMod);
