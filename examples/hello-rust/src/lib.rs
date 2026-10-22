//! hello-rust: the Rust twin of `hello-lua`. It adds a /hello command, greets players who join and counts
//! greetings in mod storage. The SDK's conformance suite checks that both behave the same.

use blockheads_mod::events::On;
use blockheads_mod::prelude::*;

struct Hello;

fn greetings() -> u64 {
    storage::get_string("greetings").and_then(|s| s.parse().ok()).unwrap_or(0)
}

impl Mod for Hello {
    fn on_enable() -> Result<(), String> {
        events::on(EventKind::PlayerJoin, |ev| {
            let count = greetings() + 1;
            let _ = storage::set_string("greetings", &count.to_string());
            if let Some(p) = ev.player() {
                let _ = p.send_chat(&format!("Hello, {}! You are greeting number {count}.", p.alias()));
            }
        })?;

        // Lowest priority runs first: later handlers may still change their mind.
        events::on_with(EventKind::Chat, On::priority(Priority::Lowest), |ev| {
            if ev.text().is_some_and(|t| t.to_ascii_lowercase().contains("lua is bad")) {
                ev.cancel();
            }
        })?;

        commands::register(
            Command::new("hello").alias("hi").description("Says hello.").usage("/hello"),
            |sender, _, _| {
                let who = sender.name();
                let _ = players::broadcast(&format!("{who} says hello to everyone!"));
                Reply::lines([format!("Hello, {who}!")])
            },
        )?;

        log::info(&format!("{} enabled", host::mod_id()));
        Ok(())
    }

    fn on_disable() {
        log::info(&format!("{} disabled", host::mod_id()));
    }
}

blockheads_mod::export_mod!(Hello);
