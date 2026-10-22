//! shop-rust: `/buy <item>` takes coins through the `economy` service of the Luau economy mod. Shows cross-language
//! interop: service arguments and results are canonical CBOR, players travel as host handles `{"$h": n}`.

wit_bindgen::generate!({
    path: "../../wit/blockheads-api",
    world: "server-mod",
});

use blockheads::api::channels::Source;
use blockheads::api::commands::{self, CommandOutcome, CommandSpec};
use blockheads::api::events::{CommandSender, Event, Outcome};
use blockheads::api::players::Role;
use blockheads::api::services;
use exports::blockheads::api::guest::Guest;

/// Prices in coins.
const ITEMS: &[(&str, i64)] = &[("apple", 5), ("sword", 30)];

/// The few canonical CBOR encodings this mod needs.
mod cbor {
    fn head(out: &mut Vec<u8>, major: u8, n: u64) {
        let m = major << 5;
        match n {
            0..=23 => out.push(m | n as u8),
            24..=0xff => out.extend([m | 24, n as u8]),
            0x100..=0xffff => {
                out.push(m | 25);
                out.extend((n as u16).to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                out.push(m | 26);
                out.extend((n as u32).to_be_bytes());
            }
            _ => {
                out.push(m | 27);
                out.extend(n.to_be_bytes());
            }
        }
    }

    /// A non-negative integer.
    pub fn uint(out: &mut Vec<u8>, n: u64) {
        head(out, 0, n);
    }

    /// A host handle `{"$h": n}`.
    pub fn handle(out: &mut Vec<u8>, n: u64) {
        out.extend([0xa1, 0x62, b'$', b'h']);
        head(out, 0, n);
    }

    /// The start of a list of `n` items.
    pub fn list(out: &mut Vec<u8>, n: u64) {
        head(out, 4, n);
    }

    pub const TRUE: &[u8] = &[0xf5];
}

struct Shop;

fn reply(line: String) -> CommandOutcome {
    CommandOutcome { handled: true, reply: vec![line] }
}

impl Guest for Shop {
    fn on_enable() -> Result<(), String> {
        commands::register(&CommandSpec {
            name: "buy".into(),
            aliases: vec![],
            description: "Buys an item with coins.".into(),
            usage: "/buy <apple|sword>".into(),
            min_role: Role::Player,
            console: false,
        })
        .map_err(|e| format!("{e:?}"))?;
        Ok(())
    }

    fn on_disable() {}

    fn handle_event(_sub: u32, _ev: Event) -> Outcome {
        Outcome { cancelled: false, mutation: None }
    }

    fn run_command(_id: u32, sender: CommandSender, _label: String, args: Vec<String>) -> CommandOutcome {
        let CommandSender::Player(player) = sender else { return reply("Players only.".into()) };
        let Some(&(item, price)) = ITEMS.iter().find(|(name, _)| Some(*name) == args.first().map(String::as_str))
        else {
            return CommandOutcome { handled: false, reply: vec![] };
        };
        // economy.withdraw(player, price) -> bool
        let mut call = Vec::new();
        cbor::list(&mut call, 2);
        cbor::handle(&mut call, player.value_handle());
        cbor::uint(&mut call, price as u64);
        match services::call("economy", "^1", "withdraw", &call) {
            Ok(v) if v == cbor::TRUE => reply(format!("You bought a {item} for {price} coins.")),
            Ok(_) => reply(format!("A {item} costs {price} coins; you do not have enough.")),
            Err(e) => reply(format!("The shop is closed ({e:?}).")),
        }
    }

    fn run_task(_id: u64) {}

    fn call_service(_name: String, _method: String, _args: Vec<u8>) -> Result<Vec<u8>, String> {
        Err("shop-rust provides no services".into())
    }

    fn on_channel_message(_channel: u32, _origin: Source, _data: Vec<u8>) {}
}

export!(Shop);
