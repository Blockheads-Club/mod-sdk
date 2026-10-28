//! Parsing of the chat commands: the server's `/voice ...` and the client's volume commands (`/voicevol`, the
//! volume dialog's text field).

use crate::model::Role;

/// The server `/voice` usage lines.
pub const USAGE: &[&str] = &[
    "/voice channel list | join <name> [password] | leave | create <name> [password]",
    "/voice mute <player> [duration: 30s, 10m, 2h, 1d]   /voice unmute <player>   (moderators)",
    "/voice gain <player> <0-200>   (admins)",
];

/// A parsed `/voice` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceCmd {
    /// `/voice` or `/voice help`.
    Help,
    /// `/voice mute <player> [duration]` (`None` = until unmuted).
    Mute {
        /// Alias.
        player: String,
        /// Milliseconds.
        duration_ms: Option<u64>,
    },
    /// `/voice unmute <player>`.
    Unmute {
        /// Alias.
        player: String,
    },
    /// `/voice gain <player> <0-200>`.
    Gain {
        /// Alias.
        player: String,
        /// Percent.
        percent: u16,
    },
    /// `/voice channel create <name> [password]`.
    Create {
        /// Name.
        name: String,
        /// Password.
        password: Option<String>,
    },
    /// `/voice channel join <name> [password]`.
    Join {
        /// Name.
        name: String,
        /// Password.
        password: Option<String>,
    },
    /// `/voice channel leave`.
    Leave,
    /// `/voice channel list`.
    List,
}

impl VoiceCmd {
    /// The lowest role that may run it.
    pub fn min_role(&self) -> Role {
        match self {
            VoiceCmd::Mute { .. } | VoiceCmd::Unmute { .. } => Role::Moderator,
            VoiceCmd::Gain { .. } => Role::Admin,
            _ => Role::Player,
        }
    }

    /// Needs a player sender (the console has no channel).
    pub fn needs_player(&self) -> bool {
        matches!(self, VoiceCmd::Create { .. } | VoiceCmd::Join { .. } | VoiceCmd::Leave)
    }
}

/// Reads a duration: a number with `s`, `m`, `h`, `d` or `w` (a bare number is minutes), in milliseconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (num, unit) = match s.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => s.split_at(i),
        None => (s.as_str(), "m"),
    };
    let n: u64 = num.parse().ok().filter(|n| *n > 0)?;
    let ms = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return None,
    };
    n.checked_mul(ms)
}

/// A short text for a duration (`2h 5m`, `30s`).
pub fn format_duration(ms: u64) -> String {
    let s = ms / 1000;
    let (d, h, m, sec) = (s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60);
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "m"), (sec, "s")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .take(2)
        .map(|(n, u)| format!("{n}{u}"))
        .collect();
    if parts.is_empty() {
        "0s".into()
    } else {
        parts.join(" ")
    }
}

/// Reads a percentage 0..=200 (a trailing `%` is allowed).
pub fn parse_percent(s: &str) -> Option<u16> {
    s.trim().trim_end_matches('%').parse::<u16>().ok().filter(|p| *p <= 200)
}

fn arg(args: &[String], i: usize) -> Option<String> {
    args.get(i).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Parses the arguments of `/voice`. `Err` holds what was wrong.
pub fn parse(args: &[String]) -> Result<VoiceCmd, String> {
    let sub = arg(args, 0).map(|s| s.to_ascii_lowercase());
    match sub.as_deref() {
        None | Some("help") => Ok(VoiceCmd::Help),
        Some("mute") => {
            let player = arg(args, 1).ok_or("Name a player.")?;
            let duration_ms = match arg(args, 2) {
                Some(d) => Some(parse_duration(&d).ok_or(format!("Not a duration: {d} (try 30s, 10m, 2h, 1d)."))?),
                None => None,
            };
            Ok(VoiceCmd::Mute { player, duration_ms })
        }
        Some("unmute") => Ok(VoiceCmd::Unmute { player: arg(args, 1).ok_or("Name a player.")? }),
        Some("gain") => {
            let player = arg(args, 1).ok_or("Name a player.")?;
            let p = arg(args, 2).ok_or("Give a gain from 0 to 200 (percent).")?;
            let percent = parse_percent(&p).ok_or(format!("Not a gain from 0 to 200: {p}."))?;
            Ok(VoiceCmd::Gain { player, percent })
        }
        Some("channel") | Some("ch") => {
            let action = arg(args, 1).map(|s| s.to_ascii_lowercase());
            match action.as_deref() {
                Some("list") | None => Ok(VoiceCmd::List),
                Some("leave") => Ok(VoiceCmd::Leave),
                Some(a @ ("join" | "create")) => {
                    let name = arg(args, 2).ok_or("Name a channel.")?;
                    let password = arg(args, 3);
                    Ok(if a == "join" {
                        VoiceCmd::Join { name, password }
                    } else {
                        VoiceCmd::Create { name, password }
                    })
                }
                Some(other) => Err(format!("Unknown channel action: {other}.")),
            }
        }
        Some(other) => Err(format!("Unknown: /voice {other}.")),
    }
}

/// A local volume change from `/voicevol` or the volume dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VolumeCmd {
    /// One player's volume.
    Player(String, u16),
    /// Mute one player locally.
    Mute(String),
    /// Unmute one player locally.
    Unmute(String),
    /// The master volume.
    Master(u16),
}

/// Reads `<player> <0-200|mute|unmute>` or `master <0-200>`.
pub fn parse_volume(text: &str) -> Result<VolumeCmd, String> {
    let mut words = text.split_whitespace();
    let (Some(who), Some(what), None) = (words.next(), words.next(), words.next()) else {
        return Err("Type a player and a volume (0-200), mute or unmute; or master and a volume.".into());
    };
    match what.to_ascii_lowercase().as_str() {
        "mute" => Ok(VolumeCmd::Mute(who.into())),
        "unmute" => Ok(VolumeCmd::Unmute(who.into())),
        w => {
            let p = parse_percent(w).ok_or(format!("Not a volume from 0 to 200: {w}."))?;
            if who.eq_ignore_ascii_case("master") {
                Ok(VolumeCmd::Master(p))
            } else {
                Ok(VolumeCmd::Player(who.into(), p))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect()
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30s"), Some(30_000));
        assert_eq!(parse_duration("10"), Some(600_000));
        assert_eq!(parse_duration("2H"), Some(7_200_000));
        assert_eq!(parse_duration("1d"), Some(86_400_000));
        assert_eq!(parse_duration("1w"), Some(604_800_000));
        assert_eq!(parse_duration("0m"), None);
        assert_eq!(parse_duration("5y"), None);
        assert_eq!(parse_duration("m"), None);
        assert_eq!(parse_duration("99999999999999999w"), None);
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(30_000), "30s");
        assert_eq!(format_duration(7_500_000), "2h 5m");
        assert_eq!(format_duration(90_061_000), "1d 1h");
    }

    #[test]
    fn percents() {
        assert_eq!(parse_percent("60"), Some(60));
        assert_eq!(parse_percent("200%"), Some(200));
        assert_eq!(parse_percent("201"), None);
        assert_eq!(parse_percent("-1"), None);
    }

    #[test]
    fn voice_commands() {
        assert_eq!(parse(&[]), Ok(VoiceCmd::Help));
        assert_eq!(parse(&a("HELP")), Ok(VoiceCmd::Help));
        assert_eq!(parse(&a("mute Bob")), Ok(VoiceCmd::Mute { player: "Bob".into(), duration_ms: None }));
        assert_eq!(parse(&a("mute Bob 10m")), Ok(VoiceCmd::Mute { player: "Bob".into(), duration_ms: Some(600_000) }));
        assert!(parse(&a("mute Bob soon")).unwrap_err().contains("soon"));
        assert_eq!(parse(&a("mute")), Err("Name a player.".into()));
        assert_eq!(parse(&a("unmute Bob")), Ok(VoiceCmd::Unmute { player: "Bob".into() }));
        assert!(parse(&a("unmute")).is_err());
        assert_eq!(parse(&a("gain Bob 60%")), Ok(VoiceCmd::Gain { player: "Bob".into(), percent: 60 }));
        assert!(parse(&a("gain Bob")).is_err());
        assert!(parse(&a("gain")).is_err());
        assert!(parse(&a("gain Bob 300")).unwrap_err().contains("300"));
        assert_eq!(parse(&a("channel")), Ok(VoiceCmd::List));
        assert_eq!(parse(&a("ch list")), Ok(VoiceCmd::List));
        assert_eq!(parse(&a("channel leave")), Ok(VoiceCmd::Leave));
        assert_eq!(parse(&a("channel join Red")), Ok(VoiceCmd::Join { name: "Red".into(), password: None }));
        assert_eq!(
            parse(&a("channel create Red pw")),
            Ok(VoiceCmd::Create { name: "Red".into(), password: Some("pw".into()) })
        );
        assert!(parse(&a("channel join")).is_err());
        assert!(parse(&a("channel kick")).unwrap_err().contains("kick"));
        assert!(parse(&a("dance")).unwrap_err().contains("dance"));
        assert!(parse(&["  ".to_string()]) == Ok(VoiceCmd::Help));
    }

    #[test]
    fn roles_of_commands() {
        assert_eq!(VoiceCmd::Mute { player: "x".into(), duration_ms: None }.min_role(), Role::Moderator);
        assert_eq!(VoiceCmd::Unmute { player: "x".into() }.min_role(), Role::Moderator);
        assert_eq!(VoiceCmd::Gain { player: "x".into(), percent: 1 }.min_role(), Role::Admin);
        assert_eq!(VoiceCmd::List.min_role(), Role::Player);
        assert!(VoiceCmd::Leave.needs_player() && !VoiceCmd::List.needs_player() && !VoiceCmd::Help.needs_player());
        assert!(VoiceCmd::Join { name: "x".into(), password: None }.needs_player());
        assert!(VoiceCmd::Create { name: "x".into(), password: None }.needs_player());
        assert_eq!(USAGE.len(), 3);
    }

    #[test]
    fn volume_commands() {
        assert_eq!(parse_volume("Bob 50"), Ok(VolumeCmd::Player("Bob".into(), 50)));
        assert_eq!(parse_volume("Bob MUTE"), Ok(VolumeCmd::Mute("Bob".into())));
        assert_eq!(parse_volume(" Bob unmute "), Ok(VolumeCmd::Unmute("Bob".into())));
        assert_eq!(parse_volume("Master 150%"), Ok(VolumeCmd::Master(150)));
        assert!(parse_volume("Bob loud").unwrap_err().contains("loud"));
        assert!(parse_volume("Bob").is_err());
        assert!(parse_volume("Bob 1 2").is_err());
    }
}
