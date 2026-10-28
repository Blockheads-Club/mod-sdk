//! The official `voice` mod (`examples/voice-rust`, built by `scripts/build-wasm.sh`) in the real mod host, with a
//! recording voice host: routes, the role matrix with the `perms` mod's groups, commands, settings, channels,
//! players without voice (server) and key bindings, the talk button, the microphone opt-in and icons (client).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use bhmod_core::input::InputCommand;
use bhmod_core::types::{ApiError, CommandSender, Event, LogLevel, Pos, Role, Side};
use bhmod_core::ui::{ButtonAction, OverheadIcon, UiCommand};
use bhmod_core::value::Value as CborValue;
use bhmod_core::voice::{Activation, Caps, Icons, Mode, Route, TransportConfig, VoiceEvent, VoiceState};
use bhmod_core::{GameBridge, HostOptions, ModHost, ModState, RemoteCall, RemoteError, VoiceHost};
use bhmod_wasm::{WasmConfig, WasmRuntime};

fn example(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name)
}

/// Copies an example's package files into `mods/<id>`.
fn install(mods: &Path, example_dir: &str, id: &str) {
    let src = example(example_dir);
    let dst = mods.join(id);
    for f in
        ["mod.toml", "server/mod.wasm", "client/mod.wasm", "server/main.luau", "wit/voice.wit", "wit/permissions.wit"]
    {
        if src.join(f).exists() {
            fs::create_dir_all(dst.join(f).parent().unwrap()).unwrap();
            fs::copy(src.join(f), dst.join(f)).unwrap_or_else(|e| panic!("{f}: {e} (run scripts/build-wasm.sh first)"));
        }
    }
}

#[derive(Default)]
struct Rec {
    states: RefCell<BTreeMap<String, VoiceState>>,
    routes: RefCell<BTreeMap<String, Vec<Route>>>,
    allowed: RefCell<BTreeMap<String, (u8, u8)>>,
    mutes: RefCell<BTreeMap<String, Option<u64>>>,
    gains: RefCell<BTreeMap<String, u16>>,
    caps: RefCell<Option<Caps>>,
    transport: RefCell<Option<TransportConfig>>,
    icons: RefCell<Option<Icons>>,
    overrides: Cell<Option<bool>>,
    /// Client calls, as text.
    calls: RefCell<Vec<String>>,
}

impl Rec {
    fn call(&self, s: String) -> Result<(), ApiError> {
        self.calls.borrow_mut().push(s);
        Ok(())
    }
}

impl VoiceHost for Rec {
    fn state(&self, player: &str) -> Result<VoiceState, ApiError> {
        Ok(self.states.borrow().get(player).copied().unwrap_or(VoiceState::None))
    }
    fn set_routes(&self, speaker: &str, routes: &[Route]) -> Result<(), ApiError> {
        self.routes.borrow_mut().insert(speaker.into(), routes.to_vec());
        Ok(())
    }
    fn mute(&self, player: &str, until_ms: Option<u64>) -> Result<(), ApiError> {
        self.mutes.borrow_mut().insert(player.into(), until_ms);
        Ok(())
    }
    fn unmute(&self, player: &str) -> Result<bool, ApiError> {
        Ok(self.mutes.borrow_mut().remove(player).is_some())
    }
    fn set_speaker_gain(&self, player: &str, percent: u16) -> Result<(), ApiError> {
        self.gains.borrow_mut().insert(player.into(), percent);
        Ok(())
    }
    fn set_caps(&self, caps: Caps) -> Result<(), ApiError> {
        *self.caps.borrow_mut() = Some(caps);
        Ok(())
    }
    fn set_transport(&self, config: TransportConfig) -> Result<(), ApiError> {
        *self.transport.borrow_mut() = Some(config);
        Ok(())
    }
    fn set_allowed(&self, player: &str, modes: u8, activations: u8) -> Result<(), ApiError> {
        self.allowed.borrow_mut().insert(player.into(), (modes, activations));
        Ok(())
    }
    fn set_icons(&self, icons: Icons) -> Result<(), ApiError> {
        *self.icons.borrow_mut() = Some(icons);
        Ok(())
    }
    fn set_broadcast_overrides_deafen(&self, on: bool) -> Result<(), ApiError> {
        self.overrides.set(Some(on));
        Ok(())
    }
    fn set_default_mode(&self, mode: Mode) -> Result<(), ApiError> {
        self.call(format!("default {}", mode.name()))
    }
    fn set_activation(&self, activation: Activation) -> Result<(), ApiError> {
        self.call(format!("activation {}", activation.name()))
    }
    fn talk(&self, mode: Option<Mode>, pressed: bool) -> Result<(), ApiError> {
        self.call(format!("talk {} {pressed}", mode.map_or("default", Mode::name)))
    }
    fn set_mic(&self, on: bool) -> Result<(), ApiError> {
        self.call(format!("mic {on}"))
    }
    fn set_vad_threshold(&self, db: f32) -> Result<(), ApiError> {
        self.call(format!("vad {db}"))
    }
    fn set_master(&self, percent: u16) -> Result<(), ApiError> {
        self.call(format!("master {percent}"))
    }
    fn set_player_volume(&self, player: &str, percent: u16) -> Result<(), ApiError> {
        self.call(format!("volume {player} {percent}"))
    }
}

/// An online player: id, alias, role, tile.
type Online = (String, String, Role, Option<Pos>);

struct Bridge {
    side: Side,
    host: &'static str,
    players: RefCell<Vec<Online>>,
    logs: RefCell<Vec<String>>,
    kicks: RefCell<Vec<String>>,
    ui: RefCell<Vec<UiCommand>>,
    input: RefCell<Vec<InputCommand>>,
    remote: RefCell<Vec<RemoteCall>>,
    voice: Rec,
}

impl Bridge {
    fn new(side: Side, host: &'static str) -> Rc<Bridge> {
        Rc::new(Bridge {
            side,
            host,
            players: RefCell::default(),
            logs: RefCell::default(),
            kicks: RefCell::default(),
            ui: RefCell::default(),
            input: RefCell::default(),
            remote: RefCell::default(),
            voice: Rec::default(),
        })
    }
    fn join(&self, id: &str, alias: &str, role: Role, pos: Option<(i32, i32)>, state: VoiceState) {
        self.players.borrow_mut().push((id.into(), alias.into(), role, pos.map(|(x, y)| Pos { x, y })));
        self.voice.states.borrow_mut().insert(id.into(), state);
    }
    fn log_text(&self) -> String {
        self.logs.borrow().join("\n")
    }
    fn routes(&self, speaker: &str) -> Vec<(String, u8, f32, f32, f32)> {
        let r = self.voice.routes.borrow();
        r.get(speaker)
            .map_or(Vec::new(), |r| r.iter().map(|r| (r.listener.clone(), r.modes, r.gain, r.pan, r.muffle)).collect())
    }
}

impl GameBridge for Bridge {
    fn side(&self) -> Side {
        self.side
    }
    fn host_name(&self) -> String {
        self.host.into()
    }
    fn log(&self, mod_id: Option<&str>, _level: LogLevel, message: &str) {
        self.logs.borrow_mut().push(format!("{}: {message}", mod_id.unwrap_or("-")));
    }
    fn online_players(&self) -> Vec<String> {
        self.players.borrow().iter().map(|p| p.0.clone()).collect()
    }
    fn player_alias(&self, id: &str) -> Option<String> {
        self.players.borrow().iter().find(|p| p.0 == id).map(|p| p.1.clone())
    }
    fn player_role(&self, id: &str) -> Role {
        self.players.borrow().iter().find(|p| p.0 == id).map_or(Role::Player, |p| p.2)
    }
    fn send_chat(&self, _id: &str, _text: &str) -> bool {
        true
    }
    fn broadcast(&self, _text: &str) {}
    fn kick(&self, id: &str, _reason: &str) -> bool {
        self.kicks.borrow_mut().push(id.into());
        true
    }
    fn is_builtin_command(&self, _name: &str) -> bool {
        false
    }
    fn player_pos(&self, id: &str) -> Option<Pos> {
        self.players.borrow().iter().find(|p| p.0 == id).and_then(|p| p.3)
    }
    fn service_send(&self, call: &RemoteCall) -> Result<(), RemoteError> {
        self.remote.borrow_mut().push(call.clone());
        Ok(())
    }
    fn ui(&self, _mod_id: &str, command: &UiCommand) -> Result<(), ApiError> {
        self.ui.borrow_mut().push(command.clone());
        Ok(())
    }
    fn voice(&self) -> Option<&dyn VoiceHost> {
        Some(&self.voice)
    }
    fn tile_solid(&self, x: i32, y: i32) -> Option<bool> {
        Some(x == 5 && y == 0)
    }
    fn world_width(&self) -> Option<i32> {
        Some(1024)
    }
    fn input(&self, _mod_id: &str, command: &InputCommand) -> Result<(), ApiError> {
        self.input.borrow_mut().push(command.clone());
        Ok(())
    }
}

fn host(dir: &Path, bridge: Rc<Bridge>, side: Side) -> ModHost {
    let mut opts = HostOptions::new(dir, side);
    opts.runtimes.push(Box::new(WasmRuntime::new(WasmConfig::default()).unwrap()));
    opts.budgets.hard = std::time::Duration::from_secs(3);
    opts.budgets.soft = opts.budgets.hard;
    opts.budgets.step = opts.budgets.hard;
    if side == Side::Client {
        opts.client_settings = Some(dir.join("client-settings.toml"));
    }
    ModHost::load(opts, bridge)
}

const PROX: u8 = 1;
const GLOBAL: u8 = 2;
const CHANNEL: u8 = 4;
const BROADCAST: u8 = 8;

#[test]
fn server_half() {
    let tmp = tempfile::tempdir().unwrap();
    install(tmp.path(), "voice-rust", "voice");
    install(tmp.path(), "perms", "perms");
    fs::write(
        tmp.path().join("mods.toml"),
        "[voice]\ngrant = [\"voice\", \"channels\", \"players:kick\"]\n[voice.settings]\nmode_global = true\ntalk_global = [\"vip\"]\n",
    )
    .unwrap();
    let b = Bridge::new(Side::Server, "fake-server 1.0");
    // Trivial bridge calls the mod never makes.
    assert!(b.send_chat("x", "y") && !b.is_builtin_command("voice"));
    b.broadcast("z");
    b.join("a", "ALICE", Role::Admin, Some((0, 0)), VoiceState::Ready);
    b.join("b", "BOB", Role::Player, Some((10, 0)), VoiceState::Ready);
    b.join("c", "CAROL", Role::Player, Some((100, 0)), VoiceState::Ready);
    b.join("v", "VAN", Role::Player, Some((1, 1)), VoiceState::None);
    let mut h = host(tmp.path(), b.clone(), Side::Server);
    h.enable("World", "wid");
    assert!(h.mods().iter().all(|m| m.state == ModState::Enabled), "{}", b.log_text());
    // Global settings reach the host.
    assert_eq!(b.voice.caps.borrow().unwrap().max_bitrate, 24000);
    assert_eq!(b.voice.transport.borrow().unwrap().udp_port, None);
    assert!(b.voice.icons.borrow().unwrap().no_voice);
    assert_eq!(b.voice.overrides.get(), Some(false));
    h.tick(0.2);
    // Alice (admin: every perms node, so also group "vip"): every mode; Bob: proximity and channel.
    assert_eq!(b.voice.allowed.borrow()["a"], (PROX | GLOBAL | CHANNEL | BROADCAST, 0b111));
    assert_eq!(b.voice.allowed.borrow()["b"], (PROX | CHANNEL, 0b111));
    assert!(!b.voice.allowed.borrow().contains_key("v"), "no voice, nothing allowed");
    // Alice to Bob: global + broadcast, then proximity (10 tiles, muffled by the solid tile at x 5).
    let r = b.routes("a");
    assert_eq!(r.len(), 3, "{r:?}");
    assert_eq!((r[0].0.as_str(), r[0].1), ("b", GLOBAL | BROADCAST));
    assert_eq!((r[1].0.as_str(), r[1].1), ("c", GLOBAL | BROADCAST));
    let (who, modes, gain, pan, muffle) = r[2].clone();
    assert_eq!((who.as_str(), modes), ("b", PROX));
    assert!(gain > 0.0 && gain < 1.0 && pan < 0.0 && (muffle - 0.15).abs() < 1e-6, "{:?}", r[2]);
    // Bob: only Alice nearby.
    assert_eq!(b.routes("b").iter().map(|r| (r.0.as_str(), r.1)).collect::<Vec<_>>(), [("a", PROX)]);
    assert!(b.routes("c").is_empty() && !b.voice.routes.borrow().contains_key("v"));

    // Bob joins group vip through the perms mod: global after the next group refresh (5 s).
    let reply = h.command(CommandSender::Console, "/perm grant BOB group.vip").unwrap();
    assert!(reply[0].contains("Granted"), "{reply:?}");
    for _ in 0..25 {
        h.tick(0.2);
    }
    assert_eq!(b.voice.allowed.borrow()["b"].0, PROX | GLOBAL | CHANNEL);

    // Commands.
    let say = |h: &mut ModHost, who: &str, line: &str| h.command(CommandSender::Player(who.into()), line).unwrap();
    assert!(say(&mut h, "b", "/voice mute CAROL")[0].contains("may not"));
    assert!(h.command(CommandSender::Console, "/voice mute CAROL 10m").unwrap()[0].contains("for 10m"));
    assert!(b.voice.mutes.borrow()["c"].is_some());
    assert!(say(&mut h, "a", "/voice unmute carol")[0].contains("is unmuted"));
    assert!(say(&mut h, "a", "/voice unmute carol")[0].contains("was not muted"));
    assert!(say(&mut h, "a", "/voice gain BOB 50")[0].contains("50"));
    assert_eq!(b.voice.gains.borrow()["b"], 50);
    assert!(say(&mut h, "a", "/voice gain NOBODY 50")[0].contains("not online"));
    assert!(say(&mut h, "b", "/voice channel create Red pw")[0].contains("Created Red"));
    assert!(say(&mut h, "c", "/voice channel join red")[0].contains("Wrong channel password"));
    assert!(say(&mut h, "c", "/voice channel join red pw")[0].contains("joined Red"));
    let list = say(&mut h, "c", "/voice channel list");
    assert_eq!(list, ["Red (2) [password] [temporary] <- you"]);
    assert!(h.command(CommandSender::Console, "/voice channel leave").unwrap()[0].contains("Only players"));
    assert_eq!(say(&mut h, "b", "/voice"), h.command(CommandSender::Console, "/voice help").unwrap());
    h.tick(0.2);
    assert!(b.routes("b").iter().any(|r| r.0 == "c" && r.1 & CHANNEL != 0), "{:?}", b.routes("b"));
    assert!(say(&mut h, "c", "/voice channel leave")[0].contains("left Red"));

    // Dead speakers are silent nearby until they move again.
    h.fire(Event::Death { player: Some("b".into()), blockhead: 1 });
    h.tick(0.2);
    assert!(b.routes("b").iter().all(|r| r.1 & PROX == 0), "{:?}", b.routes("b"));
    h.fire(Event::PlayerMove { player: "b".into(), from: Pos { x: 10, y: 0 }, to: Pos { x: 10, y: 0 } });
    h.tick(0.2);
    assert!(b.routes("b").iter().any(|r| r.1 == PROX));

    // Settings: a new cap reaches the host; players without voice get kicked after 20 s once not allowed.
    h.set_setting_cbor("voice", "max_bitrate", &CborValue::Int(32000).encode()).unwrap();
    h.set_setting_cbor("voice", "allow_vanilla", &CborValue::Bool(false).encode()).unwrap();
    assert_eq!(b.voice.caps.borrow().unwrap().max_bitrate, 32000);
    for _ in 0..100 {
        h.tick(0.2);
    }
    assert_eq!(*b.kicks.borrow(), ["v"]);
    h.disable_all();
    assert!(!b.log_text().contains("voice: saving mutes"), "{}", b.log_text());
}

#[test]
fn client_half() {
    let tmp = tempfile::tempdir().unwrap();
    install(tmp.path(), "voice-rust", "voice");
    fs::write(tmp.path().join("mods.toml"), "[voice]\ngrant = [\"voice\", \"channels\", \"players:kick\"]\n").unwrap();
    let b = Bridge::new(Side::Client, "bhmod-android 0.3");
    b.join("a", "ALICE", Role::Player, None, VoiceState::Ready);
    b.join("v", "VAN", Role::Player, None, VoiceState::None);
    let mut h = host(tmp.path(), b.clone(), Side::Client);
    h.enable("World", "wid");
    assert_eq!(h.mods()[0].state, ModState::Enabled, "{}", b.log_text());
    // Key bindings: one push-to-talk key per mode plus menu, mute and deafen.
    let binds: Vec<(String, String)> = b
        .input
        .borrow()
        .iter()
        .filter_map(|c| match c {
            InputCommand::Bind { action, default_key } => Some((action.clone(), default_key.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(binds[..4].iter().map(|b| b.1.as_str()).collect::<Vec<_>>(), ["v", "g", "b", "j"]);
    assert_eq!(binds.len(), 7);
    // Defaults reach the host; the button shows on Android; the mic is off until the player says yes.
    let calls = b.voice.calls.borrow().clone();
    assert!(calls.contains(&"master 100".into()) && calls.contains(&"default proximity".into()), "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("mic")));
    let ui = b.ui.borrow().clone();
    assert!(ui.iter().any(|c| matches!(c, UiCommand::SetButton { id, .. } if id == "ptt")));
    assert!(ui
        .iter()
        .any(|c| matches!(c, UiCommand::SetOverhead { player, icon: OverheadIcon::NoVoice } if player == "v")));
    let consent = ui
        .iter()
        .find_map(|c| match c {
            UiCommand::ShowDialog { id, dialog } if dialog.title == "Voice chat" => Some(*id),
            _ => None,
        })
        .expect("mic opt-in dialog");
    assert!(h.ui_result(consent, 0, None));
    assert!(b.voice.calls.borrow().contains(&"mic true".into()));
    assert_eq!(h.settings_values("voice").unwrap()["mic"], "on");
    // The client asked the server for its allowed modes.
    assert!(b.remote.borrow().iter().any(|c| c.service == "voice" && c.method == "info"));

    // Keys and the button.
    b.voice.calls.borrow_mut().clear();
    assert!(h.input_event("voice", "talk-global", true));
    h.input_event("voice", "talk-global", false);
    h.ui_button("voice", "ptt", ButtonAction::Press);
    h.ui_button("voice", "ptt", ButtonAction::Release);
    assert_eq!(
        *b.voice.calls.borrow(),
        ["talk global true", "talk global false", "talk default true", "talk default false"]
    );

    // Icons follow the voice events.
    b.ui.borrow_mut().clear();
    h.voice_event(&VoiceEvent::TalkStart { player: "a".into(), mode: Mode::Proximity });
    b.voice.states.borrow_mut().insert("a".into(), VoiceState::AdminMuted);
    h.voice_event(&VoiceEvent::TalkStop { player: "a".into(), mode: Mode::Proximity });
    h.voice_event(&VoiceEvent::State { player: "a".into(), state: VoiceState::Ready });
    let icons: Vec<String> =
        b.ui.borrow()
            .iter()
            .map(|c| match c {
                UiCommand::SetOverhead { icon, .. } => format!("{icon:?}"),
                other => format!("{other:?}").split_whitespace().next().unwrap().to_string(),
            })
            .collect();
    assert_eq!(icons, ["Speaking", "AdminMute", "ClearOverhead"]);

    // The volume dialog and /voicevol.
    let reply = h.command(CommandSender::Player("a".into()), "/voicevol ALICE 40").unwrap();
    assert_eq!(reply, ["ALICE at 40 %."]);
    assert!(b.voice.calls.borrow().contains(&"volume a 40".into()));
}
