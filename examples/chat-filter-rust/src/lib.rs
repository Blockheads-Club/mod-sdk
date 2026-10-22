//! chat-filter-rust: the Rust twin of `chat-filter`. Cancels chat lines with blocked links, masks rude words and
//! logs what it did. Shows event priorities, cancellation, `ignore_cancelled`, text rewriting and a moderator-only
//! command.

use blockheads_mod::events::On;
use blockheads_mod::prelude::*;

/// A message containing one of these is cancelled (lower case).
const BLOCKED: &[&str] = &["spam.example", "free-diamonds.example"];
/// These are replaced by asterisks.
const MASKED: &[&str] = &["darn", "heck"];

fn enabled() -> bool {
    storage::get_string("enabled").as_deref() != Some("off")
}

fn mask(text: &str) -> String {
    MASKED.iter().fold(text.to_string(), |t, w| t.replace(w, &"*".repeat(w.len())))
}

struct ChatFilter;

impl Mod for ChatFilter {
    fn on_enable() -> Result<(), String> {
        // Low: decide early whether the message is dropped.
        events::on_with(EventKind::Chat, On::priority(Priority::Low), |ev| {
            if !enabled() {
                return;
            }
            let lower = ev.text().unwrap_or_default().to_ascii_lowercase();
            if let Some(b) = BLOCKED.iter().find(|b| lower.contains(*b)) {
                ev.cancel();
                if let Some(p) = ev.player() {
                    let _ = p.send_chat(&format!("Your message was blocked: links to {b} are not allowed."));
                }
            }
        })?;

        // Normal: rewrite what is left. `ignore_cancelled` skips messages a lower priority already dropped.
        events::on_with(EventKind::Chat, On::default().ignore_cancelled(), |ev| {
            if enabled() {
                let masked = mask(ev.text().unwrap_or_default());
                ev.set_text(masked);
            }
        })?;

        // Monitor: sees the final state and only logs.
        events::on_with(EventKind::Chat, On::priority(Priority::Monitor), |ev| {
            if ev.cancelled() {
                let who = ev.player().map(|p| p.alias()).unwrap_or_default();
                log::info(&format!("blocked a message from {who}"));
            }
        })?;

        let cmd = Command::new("chatfilter")
            .description("Turns the chat filter on or off.")
            .usage("/chatfilter on|off")
            .min_role(Role::Moderator);
        commands::register(cmd, |_, _, args| match args.first().map(String::as_str) {
            Some(mode @ ("on" | "off")) => {
                let _ = storage::set_string("enabled", mode);
                Reply::lines([format!("Chat filter {mode}.")])
            }
            _ => Reply::usage([format!("The filter is {}.", if enabled() { "on" } else { "off" })]),
        })?;
        Ok(())
    }
}

blockheads_mod::export_mod!(ChatFilter);
