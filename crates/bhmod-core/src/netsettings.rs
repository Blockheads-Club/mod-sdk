//! The server half of the 0xBC settings ops (API 0.3.4): `SETTINGS_SCHEMA` to admins, `SETTINGS_SET` from them,
//! `SETTINGS_RESULT` back (`docs/protocol-0xbc.md`, ops 0x19-0x1b). Host-agnostic and sans-IO: the server program
//! tells [`SettingsNet`] who joined (with their `HELLO.features`), hands it every `SETTINGS_SET` with the sender's
//! admin right, calls [`SettingsNet::refresh`] once per poll, and sends the returned packets reliably.

use std::collections::{BTreeMap, BTreeSet};

use bhmod_manifest::settings::{Scope, SettingsError};
use bhmod_proto::{feature, settings_error, Message, SettingsFailure, SettingsResult, SettingsSchema, SettingsSet};

use crate::host::ModHost;
use crate::settings as cfg;

/// Most `SETTINGS_SET`s one player may send per [`RATE_WINDOW_MS`].
pub const RATE_LIMIT: u32 = 20;
/// The rate limit window in milliseconds.
pub const RATE_WINDOW_MS: u64 = 10_000;

/// The `SETTINGS_SCHEMA` of mod `id`: its server-scope settings, or `None` when it has none.
pub fn schema(host: &ModHost, id: &str) -> Option<SettingsSchema> {
    let (form, values) = host.settings_form(id, Scope::Server)?;
    Some(SettingsSchema { mod_id: id.to_string(), schema: cfg::to_cbor(&form), values: cfg::to_cbor(&values) })
}

/// Admins' settings screens across the connected players.
#[derive(Debug, Default)]
pub struct SettingsNet {
    /// Players who listed `settings` in `HELLO`.
    speakers: BTreeSet<String>,
    /// Of those, the ones that got the schemas (admins).
    sent: BTreeSet<String>,
    /// Player -> (window start, sets in the window).
    rate: BTreeMap<String, (u64, u32)>,
}

impl SettingsNet {
    /// Every mod's `SETTINGS_SCHEMA` packet.
    fn all(host: &ModHost) -> Vec<Vec<u8>> {
        host.mods().iter().filter_map(|m| schema(host, &m.id)).map(|s| Message::SettingsSchema(s).encode()).collect()
    }

    /// A player is in the game. When they listed `settings` and are an admin, the returned packets (one
    /// `SETTINGS_SCHEMA` per mod with server settings) go to them.
    pub fn join(&mut self, host: &ModHost, player: &str, is_admin: bool, features: &[String]) -> Vec<Vec<u8>> {
        if !features.iter().any(|f| f == feature::SETTINGS) {
            return Vec::new();
        }
        self.speakers.insert(player.to_string());
        if !is_admin {
            return Vec::new();
        }
        self.sent.insert(player.to_string());
        SettingsNet::all(host)
    }

    /// A player left.
    pub fn leave(&mut self, player: &str) {
        self.speakers.remove(player);
        self.sent.remove(player);
        self.rate.remove(player);
    }

    /// A `SETTINGS_SET` from `player` (`now` in ms, any monotonic clock): checks the right, the rate, the mod and
    /// the key, stores the value and returns the `SETTINGS_RESULT` packet for that player. Admins get the new
    /// values through the next [`SettingsNet::refresh`].
    pub fn receive_set(
        &mut self,
        host: &mut ModHost,
        player: &str,
        is_admin: bool,
        set: &SettingsSet,
        now: u64,
    ) -> Vec<u8> {
        let outcome = self.check_set(host, player, is_admin, set, now);
        Message::SettingsResult(SettingsResult { outcome }).encode()
    }

    fn check_set(
        &mut self,
        host: &mut ModHost,
        player: &str,
        is_admin: bool,
        set: &SettingsSet,
        now: u64,
    ) -> Result<(), SettingsFailure> {
        let fail = |code, message: Option<String>| Err(SettingsFailure { code, message });
        if !is_admin {
            return fail(settings_error::NOT_PERMITTED, None);
        }
        let (start, n) = self.rate.entry(player.to_string()).or_insert((now, 0));
        if now.saturating_sub(*start) >= RATE_WINDOW_MS {
            (*start, *n) = (now, 0);
        }
        *n += 1;
        if *n > RATE_LIMIT {
            return fail(settings_error::BUSY, None);
        }
        let Some((form, _)) = host.settings_form(&set.mod_id, Scope::Server) else {
            return fail(settings_error::UNKNOWN_MOD, None);
        };
        let server_key = form["fields"].as_array().is_some_and(|f| f.iter().any(|f| f["key"] == set.key.as_str()));
        if !server_key {
            return fail(settings_error::UNKNOWN_KEY, None);
        }
        // The key exists (checked above), so the only failure left is an invalid value.
        host.set_setting_cbor(&set.mod_id, &set.key, &set.value).map(drop).map_err(|e: SettingsError| SettingsFailure {
            code: settings_error::INVALID_VALUE,
            message: Some(e.to_string()),
        })
    }

    /// Once per poll: fresh `SETTINGS_SCHEMA`s for admins after settings changed (any source), and every schema for
    /// players who became admins. Returns `(player, packet)` pairs.
    pub fn refresh(&mut self, host: &mut ModHost, is_admin: impl Fn(&str) -> bool) -> Vec<(String, Vec<u8>)> {
        let changed = host.take_changed_settings();
        let mut out = Vec::new();
        for p in &self.speakers {
            let admin = is_admin(p);
            let packets = match (admin, self.sent.contains(p)) {
                (true, false) => SettingsNet::all(host),
                (true, true) => changed
                    .iter()
                    .filter_map(|id| schema(host, id))
                    .map(|s| Message::SettingsSchema(s).encode())
                    .collect(),
                (false, _) => Vec::new(),
            };
            out.extend(packets.into_iter().map(|b| (p.clone(), b)));
        }
        self.sent = self.speakers.iter().filter(|p| is_admin(p)).cloned().collect();
        out
    }
}
