//! What the player sets on this phone and keeps on every server (API 0.3.4): where they dragged each on-screen
//! button, their key bindings, the voice preferences (master volume, per-player volumes and local mutes, as the
//! voice driver's text) and the client-scope mod settings (a TOML file the mod host reads and writes itself,
//! [`DeviceStore::client_settings`]). The server never learns any of it.
//!
//! Files live in the loader's data folder (`device/`), written whole through a temporary file and a rename, so a
//! crash leaves the old or the new file, never half of one. A file that does not parse reads as empty.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

const BUTTONS: &str = "buttons.json";
const BINDINGS: &str = "bindings.json";
const VOICE: &str = "voice-prefs.txt";
const SETTINGS: &str = "client-settings.toml";

/// The device files under one folder.
#[derive(Debug, Clone)]
pub struct DeviceStore {
    dir: PathBuf,
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    path.parent().map_or(Ok(()), fs::create_dir_all)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)
}

impl DeviceStore {
    /// The store in `dir` (created on the first write).
    pub fn new(dir: &Path) -> DeviceStore {
        DeviceStore { dir: dir.to_path_buf() }
    }

    /// The client-scope settings file for [`bhmod_core::host::HostOptions`].
    pub fn client_settings(&self) -> PathBuf {
        self.dir.join(SETTINGS)
    }

    fn read_json(&self, name: &str) -> Map<String, Value> {
        fs::read(self.dir.join(name))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    fn write_json(&self, name: &str, v: &Map<String, Value>) -> io::Result<()> {
        write_atomic(&self.dir.join(name), Value::Object(v.clone()).to_string().as_bytes())
    }

    /// Where the player dropped button `id` of `mod_id` (screen fractions), if they moved it.
    pub fn button_pos(&self, mod_id: &str, id: &str) -> Option<(f32, f32)> {
        let all = self.read_json(BUTTONS);
        let p = all.get(&format!("{mod_id}/{id}"))?.as_array()?;
        let f = |i: usize| p.get(i).and_then(Value::as_f64).map(|v| v.clamp(0.0, 1.0) as f32);
        Some((f(0)?, f(1)?))
    }

    /// Stores a dropped button position (clamped to the screen).
    pub fn set_button_pos(&self, mod_id: &str, id: &str, x: f32, y: f32) -> io::Result<()> {
        let mut all = self.read_json(BUTTONS);
        all.insert(format!("{mod_id}/{id}"), json!([x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)]));
        self.write_json(BUTTONS, &all)
    }

    /// The player's key for an action: `None` = the default (not rebound), `Some(None)` = cleared on purpose.
    pub fn binding(&self, mod_id: &str, action: &str) -> Option<Option<String>> {
        let all = self.read_json(BINDINGS);
        let v = all.get(mod_id)?.get(action)?;
        Some(v.as_str().map(str::to_string))
    }

    /// Every rebinding of `mod_id`: action -> key (`None` = cleared).
    pub fn bindings(&self, mod_id: &str) -> BTreeMap<String, Option<String>> {
        let all = self.read_json(BINDINGS);
        let Some(m) = all.get(mod_id).and_then(Value::as_object) else { return BTreeMap::new() };
        m.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_string))).collect()
    }

    /// Rebinds an action (`Some(None)`: no key), or goes back to the default (`None`).
    pub fn set_binding(&self, mod_id: &str, action: &str, key: Option<Option<&str>>) -> io::Result<()> {
        let mut all = self.read_json(BINDINGS);
        let m = all.entry(mod_id.to_string()).or_insert_with(|| json!({}));
        if !m.is_object() {
            *m = json!({});
        }
        let obj = m.as_object_mut().expect("just made an object");
        match key {
            None => {
                obj.remove(action);
            }
            Some(k) => {
                obj.insert(action.to_string(), k.map_or(Value::Null, |s| json!(s)));
            }
        }
        if obj.is_empty() {
            all.remove(mod_id);
        }
        self.write_json(BINDINGS, &all)
    }

    /// The voice preferences text (empty when none were saved).
    pub fn voice_prefs(&self) -> String {
        fs::read_to_string(self.dir.join(VOICE)).unwrap_or_default()
    }

    /// Saves the voice preferences text.
    pub fn set_voice_prefs(&self, text: &str) -> io::Result<()> {
        write_atomic(&self.dir.join(VOICE), text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_round_trip_and_clamp() {
        let t = tempfile::tempdir().unwrap();
        let s = DeviceStore::new(&t.path().join("device"));
        assert_eq!(s.button_pos("voice", "ptt"), None);
        s.set_button_pos("voice", "ptt", 0.25, 1.5).unwrap();
        assert_eq!(s.button_pos("voice", "ptt"), Some((0.25, 1.0)));
        assert_eq!(s.button_pos("voice", "other"), None);
        fs::write(t.path().join("device").join(BUTTONS), r#"{"voice/ptt": [0.5]}"#).unwrap();
        assert_eq!(s.button_pos("voice", "ptt"), None);
        fs::write(t.path().join("device").join(BUTTONS), r#"{"voice/ptt": 3}"#).unwrap();
        assert_eq!(s.button_pos("voice", "ptt"), None);
        fs::write(t.path().join("device").join(BUTTONS), "not json").unwrap();
        assert_eq!(s.button_pos("voice", "ptt"), None);
        s.set_button_pos("voice", "ptt", -1.0, 0.5).unwrap();
        assert_eq!(s.button_pos("voice", "ptt"), Some((0.0, 0.5)));
    }

    #[test]
    fn bindings_default_rebound_and_cleared() {
        let t = tempfile::tempdir().unwrap();
        let s = DeviceStore::new(t.path());
        assert_eq!(s.binding("voice", "talk"), None);
        s.set_binding("voice", "talk", Some(Some("v"))).unwrap();
        s.set_binding("voice", "global", Some(None)).unwrap();
        assert_eq!(s.binding("voice", "talk"), Some(Some("v".into())));
        assert_eq!(s.binding("voice", "global"), Some(None));
        assert_eq!(s.binding("voice", "nope"), None);
        assert_eq!(s.bindings("voice").len(), 2);
        assert!(s.bindings("other").is_empty());
        s.set_binding("voice", "talk", None).unwrap();
        s.set_binding("voice", "global", None).unwrap();
        assert!(s.bindings("voice").is_empty());
        fs::write(t.path().join(BINDINGS), r#"{"voice": 1}"#).unwrap();
        assert!(s.bindings("voice").is_empty());
        s.set_binding("voice", "talk", Some(Some("b"))).unwrap();
        assert_eq!(s.binding("voice", "talk"), Some(Some("b".into())));
    }

    #[test]
    fn voice_prefs_and_settings_path() {
        let t = tempfile::tempdir().unwrap();
        let s = DeviceStore::new(t.path());
        assert_eq!(s.voice_prefs(), "");
        s.set_voice_prefs("master=120\n").unwrap();
        assert_eq!(s.voice_prefs(), "master=120\n");
        assert_eq!(s.client_settings(), t.path().join(SETTINGS));
    }

    #[test]
    fn writes_fail_cleanly_when_the_folder_is_a_file() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("file");
        fs::write(&f, b"x").unwrap();
        let s = DeviceStore::new(&f);
        assert!(s.set_voice_prefs("x").is_err());
        assert!(s.set_button_pos("m", "b", 0.0, 0.0).is_err());
    }
}
