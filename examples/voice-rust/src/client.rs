//! The client half: push-to-talk keys and button, mode picker, microphone opt-in, overhead icons, local volumes.

use std::cell::RefCell;

use blockheads_mod::cbor::Value;
use blockheads_mod::prelude::*;
use blockheads_mod::services::Target;
use blockheads_mod::ui::{Answer, ButtonKind, Dialog, DialogButton, Field};
use blockheads_mod::{input, ui as wui, voice as wv};

use crate::command::{parse_volume, VolumeCmd};
use crate::config::{ClientConfig, MicChoice};
use crate::glue::{log_err, on_custom, wit_activation, wit_mode};
use crate::model::{Icon, Mode, State};
use crate::payload::{
    button_event, input_event, mic_event, picker_modes, state_event, talk_event, volume_text, ButtonAction, Info,
    VolumeLine,
};

const PTT: &str = "ptt";
const MENU: &str = "menu";
const MENU_ACTION: &str = "voice-menu";
const MUTE_ACTION: &str = "self-mute";
const DEAFEN_ACTION: &str = "deafen";

#[derive(Default)]
struct Client {
    cfg: ClientConfig,
    info: Info,
    /// The on-screen button is held and talking.
    held: bool,
    self_mute: bool,
    deafen: bool,
    consent_dialog: Option<u32>,
    picker_dialog: Option<(u32, Vec<Mode>)>,
    menu_dialog: Option<u32>,
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

fn show_icon(p: &Player, state: State) {
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
    if wv::set_activation(wit_activation(cfg.activation)).is_err() {
        // Not allowed here: push-to-talk is always the fallback the server is asked to allow.
        let _ = wv::set_activation(wv::Activation::Ptt);
    }
    if cfg.mic == MicChoice::On {
        log_err("microphone", wv::set_mic(true));
    }
    if cfg.shows_button(&host::host_name()) {
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

/// Asks the server what this player may do, then runs `then`.
fn fetch_info(then: impl FnOnce() + 'static) {
    let sent = services::call_remote(&Target::Server, "voice", "^1", "info", &[], 3000, move |r| {
        if let Ok(v) = r {
            with(|c| c.info = Info::from_value(&v));
        }
        then();
    });
    if let Err(e) = sent {
        log::debug(&format!("voice info: {}", String::from(e)));
    }
}

fn open_picker() {
    fetch_info(|| {
        let (allowed, current) = with(|c| (c.info.modes, c.cfg.default_mode));
        let modes = picker_modes(allowed, current);
        if modes.is_empty() {
            let _ = ui::toast("No other talk mode on this server.");
            return;
        }
        let buttons: Vec<(&str, ButtonKind)> = modes.iter().map(|m| (m.label(), ButtonKind::Secondary)).collect();
        let text = format!("Now: {}. Talk to:", current.label());
        let id = dialog("Talk mode", &text, None, &buttons);
        with(|c| c.picker_dialog = id.map(|id| (id, modes)));
    });
}

fn open_menu() {
    let lines: Vec<VolumeLine> = players::online()
        .iter()
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
        Menu,
        Other,
    }
    let which = with(|c| {
        if c.consent_dialog == Some(a.dialog) {
            c.consent_dialog = None;
            Which::Consent
        } else if c.picker_dialog.as_ref().is_some_and(|(id, _)| *id == a.dialog) {
            Which::Picker(c.picker_dialog.take().map(|(_, m)| m).unwrap_or_default())
        } else if c.menu_dialog == Some(a.dialog) {
            c.menu_dialog = None;
            Which::Menu
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
        Which::Menu => match a.button {
            0 => {
                let line = apply_volume(a.text.as_deref().unwrap_or_default());
                let _ = ui::toast(&line);
            }
            1 => log_err("settings screen", wui::open_settings(&host::mod_id())),
            _ => {}
        },
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
            let on = with(|c| {
                c.self_mute = !c.self_mute;
                c.self_mute
            });
            log_err("self mute", wv::set_self_mute(on));
            let _ = ui::toast(if on { "Microphone muted." } else { "Microphone on." });
        }
        DEAFEN_ACTION => {
            let on = with(|c| {
                c.deafen = !c.deafen;
                c.deafen
            });
            log_err("deafen", wv::set_deafen(on));
            let _ = ui::toast(if on { "Voice chat deafened." } else { "Voice chat audible." });
        }
        _ => {}
    }
}

fn on_button(id: &str, action: ButtonAction) {
    match (id, action) {
        (PTT, ButtonAction::Press) => {
            with(|c| c.held = true);
            log_err("talk", wv::talk(None, true));
        }
        (PTT, ButtonAction::LongPress) => {
            with(|c| c.held = false);
            log_err("talk", wv::talk(None, false));
            open_picker();
        }
        (PTT, ButtonAction::Release) => {
            if with(|c| std::mem::take(&mut c.held)) {
                log_err("talk", wv::talk(None, false));
            }
        }
        (MENU, ButtonAction::Release) => open_menu(),
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
    on_custom("voice:mic", |v| {
        if mic_event(v).as_deref() == Some("denied") {
            let _ = ui::toast("The microphone was refused. Allow it in the system settings to talk.");
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
    if with(|c| c.cfg.mic) == MicChoice::Ask {
        ask_consent();
    }
    fetch_info(|| {});
    Ok(())
}
