//! WASM glue shared by both sides: the export, conversions between the policy types and the WIT bindings, small
//! helpers. Everything with a decision in it lives in the host-tested modules.

use blockheads_mod::cbor::Value;
use blockheads_mod::events::{self, EventCtx};
use blockheads_mod::prelude::*;
use blockheads_mod::voice as wv;

use crate::model::{Activation, Mode, Role as R};

pub fn wit_mode(m: Mode) -> wv::Mode {
    match m {
        Mode::Proximity => wv::Mode::Proximity,
        Mode::Global => wv::Mode::Global,
        Mode::Channel => wv::Mode::Channel,
        Mode::Broadcast => wv::Mode::Broadcast,
    }
}

pub fn from_wit_mode(m: wv::Mode) -> Mode {
    match m {
        wv::Mode::Proximity => Mode::Proximity,
        wv::Mode::Global => Mode::Global,
        wv::Mode::Channel => Mode::Channel,
        wv::Mode::Broadcast => Mode::Broadcast,
    }
}

pub fn wit_activation(a: Activation) -> wv::Activation {
    match a {
        Activation::Ptt => wv::Activation::Ptt,
        Activation::Vad => wv::Activation::Vad,
        Activation::Toggle => wv::Activation::Toggle,
    }
}

pub fn role_of(p: &Player) -> R {
    match p.role() {
        Role::Player => R::Player,
        Role::Moderator => R::Moderator,
        Role::Admin => R::Admin,
        Role::Owner => R::Owner,
    }
}

/// Unix time in milliseconds (WASI clocks work in the mod sandbox).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The payload of a custom event, decoded.
pub fn payload(ctx: &EventCtx<'_, '_>) -> Option<Value> {
    match ctx.event() {
        events::Event::Custom(c) => Value::decode(&c.payload).ok(),
        _ => None,
    }
}

/// Subscribes `f` to the custom event `name` with its decoded payload.
pub fn on_custom(name: &str, mut f: impl FnMut(&Value) + 'static) -> Result<u32, ApiError> {
    events::on_custom(name, events::On::default(), move |ctx| {
        if let Some(v) = payload(ctx) {
            f(&v);
        }
    })
}

pub fn log_err(what: &str, r: Result<(), ApiError>) {
    if let Err(e) = r {
        log::warn(&format!("{what}: {}", String::from(e)));
    }
}

struct Voice;

impl Mod for Voice {
    fn on_enable() -> Result<(), String> {
        match host::current_side() {
            Side::Server => crate::server::enable(),
            Side::Client => crate::client::enable(),
        }
    }

    fn on_disable() {
        if host::current_side() == Side::Server {
            crate::server::disable();
        }
    }

    fn call_service(name: String, method: String, args: Vec<u8>) -> Result<Vec<u8>, String> {
        if name != "voice" || host::current_side() != Side::Server {
            return Err("this mod provides only the server service `voice`".into());
        }
        let args = Value::decode(&args)?;
        crate::server::call_service(&method, &args).map(|v| v.encode())
    }
}

blockheads_mod::export_mod!(Voice);
