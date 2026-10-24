//! {{id}}: a Blockheads server mod compiled to a WASM component.

wit_bindgen::generate!({
    path: "bindings/blockheads-api",
    world: "server-mod",
});

use blockheads::api::channels::Source;
use blockheads::api::commands::{self, CommandOutcome, CommandSpec};
use blockheads::api::events::{self, CommandSender, Event, EventKind, Outcome};
use blockheads::api::log::{self, Level};
use blockheads::api::players::Role;
use blockheads::api::types::Priority;
use exports::blockheads::api::guest::Guest;

struct Mod;

impl Guest for Mod {
    fn on_enable() -> Result<(), String> {
        events::subscribe(EventKind::PlayerJoin, Priority::Normal, false).map_err(|e| format!("{e:?}"))?;
        commands::register(&CommandSpec {
            name: "{{id}}".into(),
            aliases: vec![],
            description: "Says hello.".into(),
            usage: "/{{id}}".into(),
            min_role: Role::Player,
            console: true,
        })
        .map_err(|e| format!("{e:?}"))?;
        // Offer a service to client mods: add `[provides] ... remote = true` to mod.toml (see there), then
        //     blockheads::api::services::provide("{{id}}", "1.0.0").map_err(|e| format!("{e:?}"))?;
        // and answer it in `call_service` below (arguments and result are canonical CBOR bytes).
        // Calling a client mod's service: `services::call_remote(RemoteTarget::Player(..), ..)`; the answer arrives
        // as the custom event `<mod-id>:service-result` (subscribe with `events::subscribe_custom`).
        log::log(Level::Info, "{{id}} enabled");
        Ok(())
    }

    fn on_disable() {}

    fn handle_event(_sub: u32, ev: Event) -> Outcome {
        if let Event::PlayerJoin(join) = ev {
            let _ = join.player.send_chat(&format!("Welcome, {}!", join.player.alias()));
        }
        Outcome { cancelled: false, mutation: None }
    }

    fn run_command(_id: u32, _sender: CommandSender, _label: String, _args: Vec<String>) -> CommandOutcome {
        CommandOutcome { handled: true, reply: vec!["Hello from {{id}}!".into()] }
    }

    fn run_task(_id: u64) {}

    fn call_service(_name: String, _method: String, _args: Vec<u8>) -> Result<Vec<u8>, String> {
        Err("no services".into())
    }

    fn on_channel_message(_channel: u32, _origin: Source, _data: Vec<u8>) {}
}

export!(Mod);
