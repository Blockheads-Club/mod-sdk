//! scheduler-rust: the Rust twin of `scheduler-lua`. Delayed and repeating tasks; one tick is one world step
//! (0.2 s on the server), so 5 ticks = 1 s.

use std::cell::Cell;

use blockheads_mod::prelude::*;
use blockheads_mod::scheduler::TICKS_PER_SECOND;

/// Longest reminder delay (one day).
const MAX_REMIND_SECONDS: f64 = 86_400.0;

thread_local! {
    /// The running countdown task.
    static COUNTDOWN: Cell<Option<u64>> = const { Cell::new(None) };
}

struct Scheduler;

impl Mod for Scheduler {
    fn on_enable() -> Result<(), String> {
        // A repeating tip every 5 minutes.
        scheduler::run_every(300 * TICKS_PER_SECOND, 300 * TICKS_PER_SECOND, || {
            let _ = players::broadcast("Tip: type /remind 30 drink water");
        })?;

        // /countdown [seconds]: counts down in chat once per second.
        let cmd = Command::new("countdown").usage("/countdown [seconds]").description("Counts down in chat.");
        commands::register(cmd, |_, _, args| {
            if COUNTDOWN.get().is_some() {
                return Reply::lines(["A countdown is already running."]);
            }
            // A whole number of seconds: a fraction such as 2.5 (or NaN) would never reach 0 and the countdown would run forever.
            let mut left = args.first().and_then(|a| a.parse::<f64>().ok()).filter(|n| n.is_finite()).unwrap_or(3.0).clamp(1.0, 10.0).floor();
            let id = scheduler::run_every(0, TICKS_PER_SECOND, move || {
                if left == 0.0 {
                    let _ = players::broadcast("Go!");
                    if let Some(id) = COUNTDOWN.take() {
                        scheduler::cancel(id);
                    }
                } else {
                    let _ = players::broadcast(&left.to_string());
                    left -= 1.0;
                }
            });
            COUNTDOWN.set(id.ok());
            Reply::ok()
        })?;

        // /remind <seconds> <text>: a one-shot task, answered right away from the next network poll.
        let cmd = Command::new("remind").usage("/remind <seconds> <text>").description("Reminds you later.");
        commands::register(cmd, |sender, _, args| {
            let seconds = args.first().and_then(|a| a.parse::<f64>().ok()).filter(|n| n.is_finite()).map(|n| n.clamp(0.0, MAX_REMIND_SECONDS));
            let (Some(seconds), Some(who), true) = (seconds, sender.player(), args.len() >= 2) else {
                return Reply::usage(Vec::<String>::new());
            };
            // Player handles only live for one call: keep the id and look the player up later.
            let id = who.id();
            let text = args[1..].join(" ");
            let later = id.clone();
            let _ = scheduler::run_later((seconds * f64::from(TICKS_PER_SECOND)).floor() as u32, move || {
                if let Some(p) = players::get(&later).filter(|p| p.is_online()) {
                    let _ = p.send_chat(&format!("Reminder: {text}"));
                }
            });
            let _ = scheduler::run_next_poll(move || {
                if let Some(p) = players::get(&id).filter(|p| p.is_online()) {
                    let _ = p.send_chat(&format!("OK, I will remind you in {seconds} s."));
                }
            });
            Reply::ok()
        })?;
        Ok(())
    }
}

blockheads_mod::export_mod!(Scheduler);
