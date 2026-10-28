//! The API 0.3.4 half of the loader: the voice client and its pipeline, overhead icons, on-screen buttons, key
//! bindings and the settings screen (client settings, key bindings, voice volume, the server's settings for admins).

use bhmod_core::voice::VoiceHost;
use bhmod_manifest::settings::Scope;
use bhmod_proto::{Message, SettingsResult, SettingsSchema};
use serde_json::Value;

use super::*;
use crate::overhead::Visibility;
use crate::voice::link::LinkDriver;
use crate::voice::pipeline::{MicState, Net, Notice};
use crate::voice::udp::host_of;

/// The voice preferences are compared (and saved when changed) at most this often.
pub const PREFS_MS: u64 = 1000;

/// Where the pipeline's packets go: the session (reliable 0xBC control) or the server's voice UDP port (frames).
struct LoaderNet<'a, P: Platform> {
    p: &'a P,
    session: Option<&'a Session>,
    udp: &'a mut Option<UdpLink>,
    server: &'a str,
    port: Option<u16>,
}

impl<P: Platform> Net for LoaderNet<'_, P> {
    fn reliable(&mut self, packet: &[u8]) {
        if let Some(pkt) = self.session.and_then(|s| s.voice_packet(packet.to_vec())) {
            self.p.send(&pkt);
        }
    }
    fn udp(&mut self, datagram: &[u8]) -> bool {
        if self.udp.is_none() {
            let Some(port) = self.port else { return false };
            match UdpLink::open(host_of(self.server), port) {
                Ok(u) => *self.udp = Some(u),
                Err(e) => {
                    self.p.log(LogLevel::Warn, &format!("voice: UDP port {port} unreachable: {e}"));
                    return false;
                }
            }
        }
        self.udp.as_ref().is_some_and(|u| u.send(datagram))
    }
}

impl<P: Platform + 'static> Loader<P> {
    /// Starts the voice client when the world opened on a server where both sides listed `voice`.
    pub(super) fn start_voice(&mut self) {
        if self.slot.is_set() || !self.session.as_ref().is_some_and(Session::voice_enabled) {
            return;
        }
        let Some(link) = self.voice_factory.as_ref().and_then(|f| f()) else { return };
        let Some(audio) = self.p.audio() else {
            self.p.log(LogLevel::Warn, "voice: no audio backend on this phone, voice chat is off");
            return;
        };
        let Some(key) = link.start() else {
            self.p.log(LogLevel::Warn, "voice: no key could be made, voice chat is off");
            return;
        };
        let prefs = self.device.voice_prefs();
        link.load_prefs(&prefs);
        self.prefs_saved = prefs;
        if let Some(pkt) = self.session.as_ref().and_then(|s| s.voice_packet(key)) {
            self.p.send(&pkt);
        }
        self.pipeline = Some(Pipeline::new(audio));
        self.slot.set(Some(link));
        self.p.log(LogLevel::Info, "voice: started");
    }

    /// Ends the voice client (world closed, disconnected): microphone and playback close, preferences are saved.
    pub(super) fn stop_voice(&mut self) {
        let Some(link) = self.slot.get() else { return };
        if let Some(mut pipe) = self.pipeline.take() {
            let notes = pipe.stop(&mut LinkDriver(&*link));
            self.notices(notes);
        }
        self.voice_events(&*link);
        self.save_prefs(&*link);
        self.slot.set(None);
        self.udp = None;
    }

    fn save_prefs(&mut self, link: &dyn VoiceLink) {
        let now = link.prefs();
        if now != self.prefs_saved {
            if let Err(e) = self.device.set_voice_prefs(&now) {
                self.p.log(LogLevel::Warn, &format!("voice: preferences not saved: {e}"));
            }
            self.prefs_saved = now;
        }
    }

    fn notices(&mut self, notes: Vec<Notice>) {
        for n in notes {
            match n {
                Notice::AskPermission => self.p.ask_mic(),
                Notice::NeedsPermission => {
                    self.p.log(LogLevel::Info, "voice: microphone not allowed; it is allowed from the Mods screen");
                    self.p.ui(ui::mic_needs_menu());
                }
                Notice::Mic(state) => {
                    self.p.log(LogLevel::Info, &format!("voice: microphone {}", state.name()));
                    if state == MicState::Denied {
                        self.p.ui(ui::mic_denied());
                    }
                }
                Notice::Indicator(mode) => self.p.ui(ui::mic_indicator(mode)),
                Notice::Failed(e) => self.p.log(LogLevel::Warn, &format!("voice: {e}")),
            }
        }
    }

    fn voice_events(&mut self, link: &dyn VoiceLink) {
        let events = link.take_events();
        if let Some(h) = self.host.as_mut() {
            for e in &events {
                h.voice_event(e);
            }
        }
    }

    /// Once per frame: UDP in, the pipeline, events to the mods, preferences.
    pub(super) fn voice_frame(&mut self, now: u64) {
        let Some(link) = self.slot.get() else { return };
        if let Some(u) = &self.udp {
            for d in u.poll() {
                link.on_udp(&d);
            }
        }
        if let Some(pipe) = self.pipeline.as_mut() {
            let mut net = LoaderNet {
                p: &*self.p,
                session: self.session.as_ref(),
                udp: &mut self.udp,
                server: &self.server,
                port: link.udp_port(),
            };
            let notes = pipe.frame(now, &mut LinkDriver(&*link), &mut net);
            self.notices(notes);
        }
        self.voice_events(&*link);
        if now >= self.prefs_at + PREFS_MS {
            self.prefs_at = now;
            self.save_prefs(&*link);
        }
    }

    /// The answer of the `RECORD_AUDIO` dialog.
    pub(super) fn on_mic_permission(&mut self, granted: bool) {
        let (Some(link), Some(pipe)) = (self.slot.get(), self.pipeline.as_mut()) else { return };
        let notes = pipe.on_permission(granted, &mut LinkDriver(&*link));
        self.notices(notes);
        self.voice_events(&*link);
    }

    /// `Output::Voice`: to the voice client. Mods own their buttons (no host mode picker).
    pub(super) fn on_voice_message(&mut self, m: &Message) {
        if let Some(l) = self.slot.get() {
            l.on_message(m);
        }
    }

    /// Once per frame: settings screens mods asked for, overhead icons.
    pub(super) fn hud_frame(&mut self, now: u64) {
        let open: Vec<String> = std::mem::take(&mut self.hud.borrow_mut().open_settings);
        for id in open {
            self.open_settings(&id);
        }
        let icons = self.slot.icons().unwrap_or_default();
        let mut hud = self.hud.borrow_mut();
        hud.board.set_visibility(Visibility {
            speaking: icons.speaking,
            no_voice: icons.no_voice,
            self_mute: icons.self_mute,
            deafen: icons.deafen,
            admin_mute: icons.admin_mute,
        });
        let active = hud.board.active();
        // Blockheads are tracked for the icons and, while mods run, for long presses on them (`ui-pick`).
        self.tracker.set_active(active || self.host.is_some());
        if active {
            let players: BTreeSet<&str> = hud.board.players().into_iter().collect();
            let heads = self.p.camera().map(|c| self.tracker.heads(now, &players, &c)).unwrap_or_default();
            if let Some(json) = hud.board.frame(now, &heads) {
                self.p.ui(json);
                self.overhead_shown = true;
            }
        } else if self.overhead_shown {
            self.overhead_shown = false;
            hud.board.reset_post();
            self.p.ui(serde_json::json!({"t": "overhead", "items": []}).to_string());
        }
    }

    /// A tap on the game: the blockhead under it, if any, is picked for the mods.
    pub(super) fn on_tap(&mut self, x: f32, y: f32) {
        let Some(cam) = self.p.camera() else { return };
        let Some(player) = self.tracker.pick(self.p.now_ms(), &cam, x, y) else { return };
        if let Some(h) = self.host.as_mut() {
            h.ui_pick(&player);
        }
    }

    /// An on-screen button was used.
    pub(super) fn on_button(&mut self, mod_id: &str, id: &str, action: &str) {
        let (Some(h), Some(a)) = (self.host.as_mut(), ButtonAction::parse(action)) else { return };
        h.ui_button(mod_id, id, a);
    }

    /// A hardware key went down or up.
    pub(super) fn on_key(&mut self, name: &str, down: bool) {
        let hits = self.hud.borrow_mut().key(&self.device, name, down);
        if let Some(h) = self.host.as_mut() {
            for (m, a) in hits {
                h.input_event(&m, &a, down);
            }
        }
    }

    /// The mods whose settings screen can open: every running mod and every mod the server sent a form of.
    pub(super) fn settings_mods(&self) -> Vec<String> {
        let mut ids: BTreeSet<String> = self
            .host
            .as_ref()
            .map(|h| h.mods().into_iter().filter(|m| m.state == ModState::Enabled).map(|m| m.id).collect())
            .unwrap_or_default();
        ids.extend(self.server_forms.keys().cloned());
        ids.into_iter().collect()
    }

    /// Shows (or refreshes) the settings screen of `mod_id`.
    pub(super) fn open_settings(&mut self, mod_id: &str) {
        let client = self.host.as_ref().and_then(|h| h.settings_form(mod_id, Scope::Client));
        let keys: Vec<ui::KeyRow> = self
            .hud
            .borrow()
            .actions(mod_id)
            .into_iter()
            .map(|(action, default_key)| ui::KeyRow {
                key: Hud::key_of(&self.device, mod_id, &action, &default_key),
                is_default: self.device.binding(mod_id, &action).is_none(),
                action,
                default_key,
            })
            .collect();
        let voice = self.slot.get().filter(|_| self.voice_mods.contains(mod_id)).map(|l| ui::VoiceSection {
            master: l.master().unwrap_or(100),
            players: l
                .speakers()
                .into_iter()
                .map(|id| ui::PlayerVolume {
                    name: l.alias(&id).unwrap_or_else(|| id.clone()),
                    volume: l.player_volume(&id).unwrap_or(100),
                    muted: l.player_muted(&id).unwrap_or(false),
                    id,
                })
                .collect(),
        });
        let server = self.server_forms.get(mod_id);
        self.settings_open = Some(mod_id.to_string());
        let name = self.mod_names.get(mod_id);
        self.p.ui(ui::settings_screen(mod_id, name, client.as_ref(), &keys, voice.as_ref(), server));
    }

    fn setting_failed(&self, mod_id: &str, message: &str) {
        self.p.ui(ui::settings_result(mod_id, None, false, Some(message), false));
    }

    /// A change in the settings screen.
    pub(super) fn set_setting(&mut self, mod_id: &str, scope: &str, key: &str, json: &str) {
        let Ok(value) = serde_json::from_str::<Value>(json) else {
            return self.setting_failed(mod_id, "not a value");
        };
        match scope {
            "client" => {
                let Some(h) = self.host.as_mut() else { return self.setting_failed(mod_id, "no mods are running") };
                match h.set_setting(mod_id, key, &value) {
                    Ok(_) => {
                        self.p.ui(ui::settings_result(mod_id, Some(key), true, None, false));
                        self.open_settings(mod_id);
                    }
                    Err(e) => self.setting_failed(mod_id, &e.to_string()),
                }
            }
            "server" => {
                let cbor = bhmod_core::settings::to_cbor(&value);
                match self.session.as_ref().and_then(|s| s.settings_set(mod_id, key, cbor)) {
                    Some(pkt) => self.p.send(&pkt),
                    None => self.setting_failed(mod_id, "not connected to a server with settings"),
                }
            }
            "keys" => {
                let choice = match &value {
                    Value::String(k) if bhmod_core::input::is_key(k) => Some(Some(k.as_str())),
                    Value::Null => Some(None),
                    Value::Bool(false) => None,
                    _ => return self.setting_failed(mod_id, "not a key"),
                };
                match self.device.set_binding(mod_id, key, choice) {
                    Ok(()) => self.open_settings(mod_id),
                    Err(e) => self.setting_failed(mod_id, &e.to_string()),
                }
            }
            "voice" => match self.set_voice(key, &value) {
                Ok(()) => self.open_settings(mod_id),
                Err(e) => self.setting_failed(mod_id, &e.to_string()),
            },
            _ => self.setting_failed(mod_id, "unknown section"),
        }
    }

    /// A voice volume from the settings screen: `master`, `player:<id>` (0-200) or `mute:<id>` (bool).
    fn set_voice(&self, key: &str, value: &Value) -> Result<(), ApiError> {
        let percent = || {
            value
                .as_u64()
                .filter(|n| *n <= u64::from(bhmod_core::voice::MAX_PERCENT))
                .map(|n| n as u16)
                .ok_or(ApiError::InvalidArgument("volume must be 0 to 200".into()))
        };
        if let Some(p) = key.strip_prefix("player:") {
            return self.slot.set_player_volume(p, percent()?);
        }
        if let Some(p) = key.strip_prefix("mute:") {
            let on = value.as_bool().ok_or(ApiError::InvalidArgument("mute is true or false".into()))?;
            return self.slot.mute_player(p, on);
        }
        self.slot.set_master(percent()?)
    }

    /// `SETTINGS_SCHEMA`: the server's settings of a mod (this player is an admin).
    pub(super) fn on_settings_schema(&mut self, s: &SettingsSchema) {
        let form = bhmod_core::settings::from_cbor(&s.schema)
            .and_then(|schema| Ok((schema, bhmod_core::settings::from_cbor(&s.values)?)));
        match form {
            Ok(f) => {
                self.server_forms.insert(s.mod_id.clone(), f);
                if self.settings_open.as_deref() == Some(s.mod_id.as_str()) {
                    self.open_settings(&s.mod_id.clone());
                }
            }
            Err(e) => self.p.log(LogLevel::Warn, &format!("settings of {}: unreadable form: {e}", s.mod_id)),
        }
    }

    /// `SETTINGS_RESULT`: the answer to the last server setting change.
    pub(super) fn on_settings_result(&mut self, r: &SettingsResult) {
        // Servers before API 0.3.4 do not name the setting: the answer then belongs to the open screen.
        let m = r.mod_id.clone().or_else(|| self.settings_open.clone()).unwrap_or_default();
        let (ok, text) = match &r.outcome {
            Ok(()) => (true, None),
            Err(f) => (false, Some(f.message.clone().unwrap_or_else(|| format!("refused (code {})", f.code)))),
        };
        self.p.ui(ui::settings_result(&m, r.key.as_deref(), ok, text.as_deref(), false));
    }
}
