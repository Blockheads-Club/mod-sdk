//! Voice channels: the admin's fixed channels (optional password) and temporary channels players create. A player is
//! in at most one channel. Names are compared case-insensitively; a temporary channel goes away when its last
//! member leaves. Membership lives in memory only (it starts empty after a restart).

use std::collections::{BTreeMap, BTreeSet};

use crate::config::FixedChannel;

/// Why a channel action failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChanError {
    /// Players may not create channels.
    CreateOff,
    /// The player owns `max_per_player` channels already.
    TooMany(u32),
    /// The name is taken.
    Exists,
    /// No such channel.
    NotFound,
    /// The password does not match.
    WrongPassword,
    /// The name is not 1-24 characters of `A-Z a-z 0-9 _ -`.
    BadName,
    /// The player is in no channel.
    NotInChannel,
}

impl ChanError {
    /// A chat line explaining the error.
    pub fn message(&self) -> String {
        match self {
            ChanError::CreateOff => "Players may not create voice channels on this server.".into(),
            ChanError::TooMany(n) => format!("You may own at most {n} voice channel(s)."),
            ChanError::Exists => "A channel with that name exists already.".into(),
            ChanError::NotFound => "No such channel.".into(),
            ChanError::WrongPassword => "Wrong channel password.".into(),
            ChanError::BadName => "Channel names are 1-24 letters, digits, _ or -.".into(),
            ChanError::NotInChannel => "You are not in a voice channel.".into(),
        }
    }
}

/// One channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// The name as created.
    pub name: String,
    /// The password, if any.
    pub password: Option<String>,
    /// Defined by the admin (kept when empty).
    pub fixed: bool,
    /// Who created a temporary channel (player id).
    pub owner: Option<String>,
    /// Member player ids.
    pub members: BTreeSet<String>,
}

/// A line of `/voice channel list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// Name.
    pub name: String,
    /// Members.
    pub members: usize,
    /// Has a password.
    pub locked: bool,
    /// Admin channel.
    pub fixed: bool,
}

/// Every channel, by lower-case name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Channels {
    by_key: BTreeMap<String, Channel>,
}

fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// True for a valid channel name.
pub fn valid_name(name: &str) -> bool {
    (1..=24).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl Channels {
    /// Applies the admin's fixed channel list: new ones appear, passwords update, removed ones become temporary (and
    /// go away when empty). A temporary channel with a fixed channel's name becomes fixed.
    pub fn set_fixed(&mut self, fixed: &[FixedChannel]) {
        let keys: BTreeSet<String> = fixed.iter().map(|f| key(&f.name)).collect();
        for (k, c) in self.by_key.iter_mut() {
            if c.fixed && !keys.contains(k) {
                c.fixed = false;
            }
        }
        self.by_key.retain(|_, c| c.fixed || !c.members.is_empty());
        for f in fixed {
            let c = self.by_key.entry(key(&f.name)).or_insert_with(|| Channel {
                name: f.name.clone(),
                password: None,
                fixed: true,
                owner: None,
                members: BTreeSet::new(),
            });
            c.fixed = true;
            c.owner = None;
            c.password = f.password.clone();
        }
    }

    /// Creates a temporary channel and puts its creator in it.
    pub fn create(
        &mut self,
        player: &str,
        name: &str,
        password: Option<&str>,
        allowed: bool,
        max_per_player: u32,
    ) -> Result<(), ChanError> {
        if !allowed {
            return Err(ChanError::CreateOff);
        }
        if !valid_name(name) {
            return Err(ChanError::BadName);
        }
        if self.by_key.contains_key(&key(name)) {
            return Err(ChanError::Exists);
        }
        let owned = self.by_key.values().filter(|c| c.owner.as_deref() == Some(player)).count() as u32;
        if owned >= max_per_player {
            return Err(ChanError::TooMany(max_per_player));
        }
        self.leave(player);
        let mut members = BTreeSet::new();
        members.insert(player.to_string());
        let password = password.filter(|p| !p.is_empty()).map(str::to_string);
        let c = Channel { name: name.to_string(), password, fixed: false, owner: Some(player.to_string()), members };
        self.by_key.insert(key(name), c);
        Ok(())
    }

    /// Moves a player into a channel. `bypass` skips the password (admins, other mods through the service).
    pub fn join(
        &mut self,
        player: &str,
        name: &str,
        password: Option<&str>,
        bypass: bool,
    ) -> Result<String, ChanError> {
        let c = self.by_key.get(&key(name)).ok_or(ChanError::NotFound)?;
        if !bypass && c.password.is_some() && c.password.as_deref() != password {
            return Err(ChanError::WrongPassword);
        }
        let shown = c.name.clone();
        if c.members.contains(player) {
            return Ok(shown);
        }
        self.leave(player);
        self.by_key.get_mut(&key(name)).expect("only the player's old channel can go").members.insert(player.into());
        Ok(shown)
    }

    /// Takes a player out of their channel; returns its name. An empty temporary channel is removed.
    pub fn leave(&mut self, player: &str) -> Option<String> {
        let (k, c) = self.by_key.iter_mut().find(|(_, c)| c.members.contains(player))?;
        c.members.remove(player);
        let (k, name, gone) = (k.clone(), c.name.clone(), !c.fixed && c.members.is_empty());
        if gone {
            self.by_key.remove(&k);
        }
        Some(name)
    }

    /// The lower-case key of the player's channel.
    pub fn channel_of(&self, player: &str) -> Option<String> {
        self.by_key.iter().find(|(_, c)| c.members.contains(player)).map(|(k, _)| k.clone())
    }

    /// The display name of the channel with key or name `name`.
    pub fn display(&self, name: &str) -> Option<&str> {
        self.by_key.get(&key(name)).map(|c| c.name.as_str())
    }

    /// Every channel for `/voice channel list`, fixed ones first.
    pub fn list(&self) -> Vec<Listing> {
        let mut out: Vec<Listing> = self
            .by_key
            .values()
            .map(|c| Listing {
                name: c.name.clone(),
                members: c.members.len(),
                locked: c.password.is_some(),
                fixed: c.fixed,
            })
            .collect();
        out.sort_by_key(|l| !l.fixed);
        out
    }
}

/// The chat lines of `/voice channel list`.
pub fn list_lines(list: &[Listing], current: Option<&str>) -> Vec<String> {
    if list.is_empty() {
        return vec!["No voice channels.".into()];
    }
    list.iter()
        .map(|l| {
            let mut s = format!("{} ({})", l.name, l.members);
            if l.locked {
                s.push_str(" [password]");
            }
            if !l.fixed {
                s.push_str(" [temporary]");
            }
            if current == Some(key(&l.name).as_str()) {
                s.push_str(" <- you");
            }
            s
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(name: &str, pw: Option<&str>) -> FixedChannel {
        FixedChannel { name: name.into(), password: pw.map(str::to_string) }
    }

    #[test]
    fn errors_have_text() {
        let all = [
            ChanError::CreateOff,
            ChanError::TooMany(2),
            ChanError::Exists,
            ChanError::NotFound,
            ChanError::WrongPassword,
            ChanError::BadName,
            ChanError::NotInChannel,
        ];
        for e in all {
            assert!(!e.message().is_empty());
        }
        assert!(ChanError::TooMany(2).message().contains('2'));
    }

    #[test]
    fn names() {
        assert!(valid_name("Team-1_a"));
        assert!(!valid_name(""));
        assert!(!valid_name("has space"));
        assert!(!valid_name(&"x".repeat(25)));
    }

    #[test]
    fn fixed_channels() {
        let mut ch = Channels::default();
        ch.set_fixed(&[fixed("Red", None), fixed("Blue", Some("pw"))]);
        assert_eq!(ch.join("a", "blue", None, false), Err(ChanError::WrongPassword));
        assert_eq!(ch.join("a", "blue", Some("pw"), false).as_deref(), Ok("Blue"));
        assert_eq!(ch.join("b", "BLUE", None, true).as_deref(), Ok("Blue"));
        assert_eq!(ch.channel_of("a").as_deref(), Some("blue"));
        assert_eq!(ch.join("a", "red", None, false).as_deref(), Ok("Red"));
        assert_eq!(ch.join("a", "green", None, false), Err(ChanError::NotFound));
        // Blue loses its fixed status but keeps member b.
        ch.set_fixed(&[fixed("Red", Some("new"))]);
        let l = ch.list();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].name.as_str(), l[0].locked, l[0].fixed), ("Red", true, true));
        assert_eq!((l[1].name.as_str(), l[1].fixed, l[1].members), ("Blue", false, 1));
        assert_eq!(ch.leave("b").as_deref(), Some("Blue"));
        assert_eq!(ch.display("blue"), None, "empty temporary channel removed");
        assert_eq!(ch.leave("a").as_deref(), Some("Red"));
        assert_eq!(ch.display("RED"), Some("Red"), "fixed channels stay when empty");
        assert_eq!(ch.leave("a"), None);
        // An empty former fixed channel is dropped by the next set_fixed.
        ch.set_fixed(&[]);
        assert!(ch.list().is_empty());
    }

    #[test]
    fn temporary_channels() {
        let mut ch = Channels::default();
        assert_eq!(ch.create("a", "x", None, false, 1), Err(ChanError::CreateOff));
        assert_eq!(ch.create("a", "bad name", None, true, 1), Err(ChanError::BadName));
        assert_eq!(ch.create("a", "Mine", Some(""), true, 1), Ok(()));
        assert_eq!(ch.create("b", "mine", None, true, 1), Err(ChanError::Exists));
        assert_eq!(ch.create("a", "Two", None, true, 1), Err(ChanError::TooMany(1)));
        assert_eq!(ch.join("b", "mine", None, false).as_deref(), Ok("Mine"), "empty password = open");
        assert_eq!(ch.create("b", "Locked", Some("pw"), true, 1), Ok(()));
        assert_eq!(ch.channel_of("b").as_deref(), Some("locked"), "creator moves into the new channel");
        assert!(ch.list().iter().any(|l| l.name == "Locked" && l.locked));
        // a owns "Mine" and is its last member; joining Locked removes it.
        assert_eq!(ch.join("a", "locked", Some("pw"), false).as_deref(), Ok("Locked"));
        assert_eq!(ch.display("mine"), None);
        // A fixed channel with a temporary's name takes it over.
        ch.set_fixed(&[fixed("locked", None)]);
        let l = ch.list();
        assert_eq!((l.len(), l[0].fixed, l[0].locked, l[0].members), (1, true, false, 2));
    }

    #[test]
    fn joining_the_own_channel_again() {
        let mut ch = Channels::default();
        ch.create("a", "solo", None, true, 1).unwrap();
        assert_eq!(ch.join("a", "SOLO", None, false).as_deref(), Ok("solo"));
        assert_eq!(ch.channel_of("a").as_deref(), Some("solo"));
    }

    #[test]
    fn listing_lines() {
        assert_eq!(list_lines(&[], None), ["No voice channels."]);
        let l = vec![
            Listing { name: "Red".into(), members: 2, locked: false, fixed: true },
            Listing { name: "Mine".into(), members: 1, locked: true, fixed: false },
        ];
        assert_eq!(list_lines(&l, Some("mine")), ["Red (2)", "Mine (1) [password] [temporary] <- you"]);
    }
}
