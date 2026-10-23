//! Price check (Rust twin of the client half of `prices-lua`): `/price <item>` asks the server and shows the answer
//! as a toast.

use blockheads_mod::cbor::Value;
use blockheads_mod::prelude::*;
use blockheads_mod::services::{self, RemoteError, Target};

fn show(item: &str, result: Result<Value, RemoteError>) {
    let text = match result {
        Ok(Value::Int(coins)) => format!("{item} costs {coins} coins."),
        Ok(_) => format!("{item} is not for sale."),
        Err(e) if e.kind == "timeout" => "The server did not answer.".to_string(),
        Err(e) => format!("Price check failed: {}", e.kind),
    };
    let _ = ui::toast(&text);
}

struct Prices;

impl Mod for Prices {
    fn on_enable() -> Result<(), String> {
        commands::register(Command::new("price").usage("/price <item>"), |_, _, args| {
            let Some(item) = args.first().cloned() else { return Reply::usage(Vec::<String>::new()) };
            // Returns at once; the closure runs when the server has answered (or after 3 s).
            let asked = item.clone();
            let sent = services::call_remote(
                &Target::Server,
                "prices",
                "^1",
                "price",
                &[Value::Text(item)],
                3000,
                move |result| show(&asked, result),
            );
            match sent {
                Ok(_) => Reply::ok(),
                Err(e) => Reply::lines(vec![String::from(e)]),
            }
        })?;
        Ok(())
    }
}

blockheads_mod::export_mod!(Prices);
