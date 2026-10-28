//! The client half: push-to-talk keys and button, mode picker, microphone opt-in, overhead icons, local volumes.

use std::cell::RefCell;

use blockheads_mod::cbor::Value;
use blockheads_mod::prelude::*;
use blockheads_mod::ui::{Answer, ButtonKind, Dialog, DialogButton, Field};
use blockheads_mod::{input, ui as wui, voice as wv};

use crate::command::{parse_percent, parse_volume, VolumeCmd};
use crate::config::{ClientConfig, MicChoice};
use crate::glue::{from_wit_mode, log_err, on_custom, wit_activation, wit_mode};
use crate::model::{Icon, Mode, ModeSet, State};
use crate::payload::{
    button_event, input_event, mic_event, mode_event, pick_event, picker_modes, player_text, state_event, talk_event,
    volume_text, ButtonAction, VolumeLine,
};

const PTT: &str = "ptt";
const MENU: &str = "menu";
const MENU_ACTION: &str = "voice-menu";
const MUTE_ACTION: &str = "self-mute";
const DEAFEN_ACTION: &str = "deafen";

#[derive(Default)]
struct Client {
    cfg: ClientConfig,
    /// The on-screen button is held and talking.
    held: bool,
    consent_dialog: Option<u32>,
    picker_dialog: Option<(u32, Vec<Mode>)>,
    /// The headphones button's menu (talk mode, deafen, volumes).
    hub_dialog: Option<u32>,
    menu_dialog: Option<u32>,
    /// The per-player volume dialog and its player id.
    player_dialog: Option<(u32, String)>,
}

thread_local! {
    static C: RefCell<Client> = RefCell::new(Client::default());
}

fn with<T>(f: impl FnOnce(&mut Client) -> T) -> T {
    C.with(|c| f(&mut c.borrow_mut()))
}

fn wit_icon(i: Icon) -> wui::OverheadIcon {
    match i {
        Icon::Speaking => wui::OverheadIcon::Speaking,
        Icon::NoVoice => wui::OverheadIcon::NoVoice,
        Icon::SelfMute => wui::OverheadIcon::SelfMute,
        Icon::Deafen => wui::OverheadIcon::Deafen,
        Icon::AdminMute => wui::OverheadIcon::AdminMute,
    }
}

fn from_wit_state(s: wv::VoiceState) -> State {
    match s {
        wv::VoiceState::None => State::None,
        wv::VoiceState::Ready => State::Ready,
        wv::VoiceState::Talking => State::Talking,
        wv::VoiceState::SelfMuted => State::SelfMuted,
        wv::VoiceState::Deafened => State::Deafened,
        wv::VoiceState::AdminMuted => State::AdminMuted,
    }
}

/// True for the player of this client (no icon above oneself, no volume row).
fn is_me(id: &str) -> bool {
    players::local_player().is_some_and(|me| me.id() == id)
}

fn show_icon(p: &Player, state: State) {
    if is_me(&p.id()) {
        return;
    }
    match state.icon() {
        Some(i) => log_err("overhead icon", wui::set_overhead(p, wit_icon(i))),
        None => {
            wui::clear_overhead(p);
        }
    }
}

fn refresh_icon(id: &str) {
    if let Some(p) = players::get(id) {
        let s = wv::state(&p).map_or(State::None, from_wit_state);
        show_icon(&p, s);
    }
}

/// Applies the player's settings to the host and the button.
fn apply() {
    let cfg = ClientConfig::from_value(&settings::all());
    log_err("master volume", wv::set_master(cfg.master_volume));
    log_err("vad threshold", wv::set_vad_threshold(cfg.vad_threshold));
    log_err("default mode", wv::set_default_mode(wit_mode(cfg.default_mode)));
    apply_activation(&cfg);
    if cfg.mic == MicChoice::On && !matches!(wv::mic(), Ok(wv::MicState::On)) {
        log_err("microphone", wv::set_mic(true));
    }
    if cfg.shows_button(wui::has_touch()) {
        let ptt = wui::Button {
            icon: "mic".into(),
            label: None,
            x: 0.92,
            y: 0.62,
            size: cfg.button_size,
            opacity: cfg.button_opacity,
        };
        log_err("talk button", wui::set_button(PTT, &ptt));
        let menu = wui::Button {
            icon: "headphones".into(),
            label: None,
            x: 0.92,
            y: 0.45,
            size: (cfg.button_size * 2 / 3).max(32),
            opacity: cfg.button_opacity,
        };
        log_err("menu button", wui::set_button(MENU, &menu));
    } else {
        wui::remove_button(PTT);
        wui::remove_button(MENU);
    }
    with(|c| c.cfg = cfg);
}

/// The player's activation, or push-to-talk when the server does not allow it.
fn apply_activation(cfg: &ClientConfig) {
    if wv::set_activation(wit_activation(cfg.activation)).is_err() {
        let _ = wv::set_activation(wv::Activation::Ptt);
    }
}

fn dialog(title: &str, text: &str, field: Option<Field>, buttons: &[(&str, ButtonKind)]) -> Option<u32> {
    let buttons = buttons.iter().map(|(l, k)| DialogButton { label: l.to_string(), kind: *k }).collect();
    ui::show_dialog(&Dialog { title: title.into(), text: text.into(), field, buttons }).ok()
}

fn ask_consent() {
    let id = dialog(
        "Voice chat",
        "This server has voice chat. Turn on your microphone? You can change this any time in the voice settings. \
         Nothing is recorded.",
        None,
        &[("Turn on mic", ButtonKind::Primary), ("Listen only", ButtonKind::Secondary)],
    );
    with(|c| c.consent_dialog = id);
}

fn set_choice(mic: MicChoice) {
    log_err("mic setting", settings::set("mic", &Value::Text(mic.name().into())));
}

fn open_picker() {
    let allowed = wv::allowed()
        .map_or(ModeSet::EMPTY, |a| a.modes.into_iter().map(from_wit_mode).fold(ModeSet::EMPTY, ModeSet::with));
    let current = with(|c| c.cfg.default_mode);
    let modes = picker_modes(allowed, current);
    if modes.is_empty() {
        let _ = ui::toast("No other talk mode on this server.");
        return;
    }
    let mut buttons: Vec<(&str, ButtonKind)> = modes.iter().map(|m| (m.label(), ButtonKind::Secondary)).collect();
    buttons.push(("Cancel", ButtonKind::Secondary));
    let text = format!("Now: {}. Talk to:", current.label());
    let id = dialog("Talk mode", &text, None, &buttons);
    with(|c| c.picker_dialog = id.map(|id| (id, modes)));
}

/// The headphones button's menu: talk mode, deafen, volumes.
fn open_hub() {
    let deafened = wv::deafened().unwrap_or(false);
    let mode = with(|c| c.cfg.default_mode);
    let text = format!("Talking to: {}.{}", mode.label(), if deafened { " Voice chat is deafened." } else { "" });
    let deafen = if deafened { "Undeafen" } else { "Deafen" };
    let buttons = [
        ("Talk mode", ButtonKind::Secondary),
        (deafen, ButtonKind::Secondary),
        ("Volumes", ButtonKind::Secondary),
        ("Close", ButtonKind::Secondary),
    ];
    let id = dialog("Voice chat", &text, None, &buttons);
    with(|c| c.hub_dialog = id);
}

fn toggle_deafen() {
    let on = !wv::deafened().unwrap_or(false);
    log_err("deafen", wv::set_deafen(on));
    let _ = ui::toast(if on { "Voice chat deafened." } else { "Voice chat audible." });
}

/// The volume dialog of one player (a tapped or clicked blockhead).
fn open_player(id: &str) {
    let Some(p) = players::get(id).filter(|p| !is_me(&p.id())) else { return };
    let (percent, muted) = (wv::player_volume(&p).unwrap_or(100), wv::player_muted(&p).unwrap_or(false));
    let field = Field { placeholder: "0-200".into(), initial: percent.to_string(), max_length: 4 };
    let mute = if muted { "Unmute" } else { "Mute" };
    let buttons = [("Apply", ButtonKind::Primary), (mute, ButtonKind::Danger), ("Cancel", ButtonKind::Secondary)];
    let dialog_id = dialog("Voice volume", &player_text(&p.alias(), percent, muted), Some(field), &buttons);
    with(|c| c.player_dialog = dialog_id.map(|d| (d, p.id())));
}

fn open_menu() {
    let lines: Vec<VolumeLine> = players::online()
        .iter()
        .filter(|p| !is_me(&p.id()))
        .map(|p| VolumeLine {
            alias: p.alias(),
            percent: wv::player_volume(p).unwrap_or(100),
            muted: wv::player_muted(p).unwrap_or(false),
        })
        .collect();
    let master = wv::master().unwrap_or(100);
    let field = Field { placeholder: "Bob 50 | Bob mute | master 120".into(), initial: String::new(), max_length: 64 };
    let id = dialog(
        "Voice volumes",
        &volume_text(master, &lines),
        Some(field),
        &[("Apply", ButtonKind::Primary), ("Settings", ButtonKind::Secondary), ("Close", ButtonKind::Secondary)],
    );
    with(|c| c.menu_dialog = id);
}

/// Applies a volume change; returns the line to show.
fn apply_volume(text: &str) -> String {
    let cmd = match parse_volume(text) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let find = |alias: &str| players::find_by_alias(alias).ok_or(format!("{alias} is not here."));
    let r = match &cmd {
        VolumeCmd::Master(p) => {
            let _ = settings::set("master_volume", &Value::Int(i64::from(*p)));
            wv::set_master(*p).map(|_| format!("Voice volume {p} %."))
        }
        VolumeCmd::Player(a, p) => match find(a) {
            Ok(pl) => wv::set_player_volume(&pl, *p).map(|_| format!("{} at {p} %.", pl.alias())),
            Err(e) => return e,
        },
        VolumeCmd::Mute(a) | VolumeCmd::Unmute(a) => match find(a) {
            Ok(pl) => {
                let on = matches!(cmd, VolumeCmd::Mute(_));
                wv::mute_player(&pl, on).map(|_| format!("{} {}.", pl.alias(), if on { "muted" } else { "unmuted" }))
            }
            Err(e) => return e,
        },
    };
    r.unwrap_or_else(|e| format!("Voice: {}", String::from(e)))
}

fn on_answer(a: Answer) {
    enum Which {
        Consent,
        Picker(Vec<Mode>),
        Hub,
        Menu,
        Player(String),
        Other,
    }
    let which = with(|c| {
        if c.consent_dialog == Some(a.dialog) {
            c.consent_dialog = None;
            Which::Consent
        } else if c.picker_dialog.as_ref().is_some_and(|(id, _)| *id == a.dialog) {
            Which::Picker(c.picker_dialog.take().map(|(_, m)| m).unwrap_or_default())
        } else if c.hub_dialog == Some(a.dialog) {
            c.hub_dialog = None;
            Which::Hub
        } else if c.menu_dialog == Some(a.dialog) {
            c.menu_dialog = None;
            Which::Menu
        } else if c.player_dialog.as_ref().is_some_and(|(id, _)| *id == a.dialog) {
            Which::Player(c.player_dialog.take().map(|(_, p)| p).unwrap_or_default())
        } else {
            Which::Other
        }
    });
    match which {
        Which::Consent => match a.button {
            0 => {
                set_choice(MicChoice::On);
                log_err("microphone", wv::set_mic(true));
            }
            1 => set_choice(MicChoice::Off),
            _ => {} // Closed without an answer: ask again next time.
        },
        Which::Picker(modes) => {
            if let Some(m) = modes.get(a.button as usize) {
                log_err("default mode", settings::set("default_mode", &Value::Text(m.name().into())));
                log_err("default mode", wv::set_default_mode(wit_mode(*m)));
                with(|c| c.cfg.default_mode = *m);
                let _ = ui::toast(&format!("Talking to: {}", m.label()));
            }
        }
        Which::Hub => match a.button {
            0 => open_picker(),
            1 => toggle_deafen(),
            2 => open_menu(),
            _ => {}
        },
        Which::Menu => match a.button {
            0 => {
                let line = apply_volume(a.text.as_deref().unwrap_or_default());
                let _ = ui::toast(&line);
            }
            1 => log_err("settings screen", wui::open_settings(&host::mod_id())),
            _ => {}
        },
        Which::Player(id) => {
            let Some(p) = players::get(&id) else { return };
            let line = match a.button {
                0 => match a.text.as_deref().and_then(parse_percent) {
                    Some(v) => wv::set_player_volume(&p, v).map(|_| format!("{} at {v} %.", p.alias())),
                    None => Ok("Type a volume from 0 to 200.".to_string()),
                },
                1 => {
                    let on = !wv::player_muted(&p).unwrap_or(false);
                    wv::mute_player(&p, on).map(|_| format!("{} {}.", p.alias(), if on { "muted" } else { "unmuted" }))
                }
                _ => return,
            };
            let _ = ui::toast(&line.unwrap_or_else(|e| format!("Voice: {}", String::from(e))));
        }
        Which::Other => {}
    }
}

fn on_input(action: &str, pressed: bool) {
    if let Some(m) = Mode::from_action(action) {
        log_err("talk", wv::talk(Some(wit_mode(m)), pressed));
        return;
    }
    if !pressed {
        return;
    }
    match action {
        MENU_ACTION => open_menu(),
        MUTE_ACTION => {
            let on = !wv::self_muted().unwrap_or(false);
            log_err("self mute", wv::set_self_mute(on));
            let _ = ui::toast(if on { "Microphone muted." } else { "Microphone on." });
        }
        DEAFEN_ACTION => toggle_deafen(),
        _ => {}
    }
}

fn on_button(id: &str, action: ButtonAction) {
    match (id, action) {
        (PTT, ButtonAction::Press) => {
            with(|c| c.held = true);
            log_err("talk", wv::talk(None, true));
        }
        // Push-to-talk is hold-to-talk only: a long hold keeps talking until the release. The talk mode is in the
        // headphones button's menu.
        (PTT, ButtonAction::Release) => {
            if with(|c| std::mem::take(&mut c.held)) {
                log_err("talk", wv::talk(None, false));
            }
        }
        (MENU, ButtonAction::Release) => open_hub(),
        _ => {}
    }
}

pub fn enable() -> Result<(), String> {
    let me = host::mod_id();
    for m in Mode::ALL {
        input::bind(m.action(), m.default_key())?;
    }
    input::bind(MENU_ACTION, "f8")?;
    input::bind(MUTE_ACTION, "f9")?;
    input::bind(DEAFEN_ACTION, "f10")?;
    apply();
    on_custom(&format!("{me}:settings-changed"), |_| apply())?;
    on_custom(&format!("{me}:input"), |v| {
        if let Some((action, pressed)) = input_event(v) {
            on_input(&action, pressed);
        }
    })?;
    on_custom(&format!("{me}:ui-button"), |v| {
        if let Some((id, action)) = button_event(v) {
            on_button(&id, action);
        }
    })?;
    ui::on_answer(on_answer)?;
    on_custom("voice:state", |v| {
        if let Some((id, state)) = state_event(v) {
            if let Some(p) = players::get(&id) {
                show_icon(&p, state);
            }
        }
    })?;
    on_custom("voice:talk-start", |v| {
        if let Some((id, _)) = talk_event(v) {
            if let Some(p) = players::get(&id) {
                show_icon(&p, State::Talking);
            }
        }
    })?;
    on_custom("voice:talk-stop", |v| {
        if let Some((id, _)) = talk_event(v) {
            refresh_icon(&id);
        }
    })?;
    on_custom(&format!("{me}:ui-pick"), |v| {
        if let Some(id) = pick_event(v) {
            open_player(&id);
        }
    })?;
    // The loader's own mode picker may change the default mode: keep the setting in step.
    on_custom("voice:mode", |v| {
        if let Some(m) = mode_event(v) {
            if with(|c| std::mem::replace(&mut c.cfg.default_mode, m)) != m {
                log_err("default mode", settings::set("default_mode", &Value::Text(m.name().into())));
            }
        }
    })?;
    // What the server allows changed: fall back to push-to-talk if the chosen activation is not allowed any more.
    on_custom("voice:allowed", |_| apply_activation(&with(|c| c.cfg.clone())))?;
    // `denied`: the host explains where to allow the microphone (on Android: the loader's Mods screen).
    on_custom("voice:mic", |v| {
        if mic_event(v).as_deref() == Some("unavailable") {
            let _ =
                ui::toast("Voice chat is off: this server's voice UDP port is not open (or there is no microphone).");
        }
    })?;
    commands::register(
        Command::new("voicevol").usage("/voicevol <player> <0-200|mute|unmute> | master <0-200>"),
        |_, _, args| Reply::lines([apply_volume(&args.join(" "))]),
    )?;
    commands::register(Command::new("voicemenu").alias("vm"), |_, _, _| {
        open_menu();
        Reply::ok()
    })?;
    for p in players::online() {
        refresh_icon(&p.id());
    }
    if with(|c| c.cfg.mic) == MicChoice::Ask && !matches!(wv::mic(), Ok(wv::MicState::On)) {
        ask_consent();
    }
    Ok(())
}
