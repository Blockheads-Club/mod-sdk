//! The Luau runtime (mlua, vendored Luau): one VM per mod in sandbox mode, a memory limit and an interrupt that
//! stops the VM at the call's deadline. Implements the mapping in `docs/luau-api.md`.
//!
//! The `bh` functions reach the host through a pointer to the [`HostApi`] of the call in progress. It is set when
//! the host enters the VM and cleared before the call returns, so it never outlives the borrow it came from.
#![allow(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use mlua::{
    AnyUserData, Function, Lua, MetaMethod, MultiValue, Table, UserData, UserDataMethods, Value, Variadic, VmState,
};

use crate::channels::ChannelId;
use crate::runtime::TaskId;
use crate::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, ModFiles, Runtime, SubscriptionId};
use crate::types::{ApiError, CommandOutcome, CommandSender, CommandSpec, Event, EventKind, LogLevel, Mutation};
use crate::types::{ChannelSource, ChannelTarget, ContentKind, Outcome, Pos, Priority, RejectReason, Role};
use crate::value::{self, Value as Cbor};

/// The Luau runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct LuauRuntime;

impl Runtime for LuauRuntime {
    fn kind(&self) -> bhmod_manifest::manifest::Runtime {
        bhmod_manifest::manifest::Runtime::Lua
    }

    fn instantiate(&self, mod_id: &str, files: &ModFiles, limits: InstanceLimits) -> Result<Box<dyn Instance>, String> {
        let inst = LuaInstance::new(mod_id, files.clone(), limits).map_err(|e| e.to_string())?;
        Ok(Box::new(inst))
    }
}

type HostPtr = *mut (dyn HostApi + 'static);

struct Shared {
    host: Cell<Option<HostPtr>>,
    deadline: Cell<Option<Instant>>,
    timed_out: Cell<bool>,
    files: ModFiles,
}

fn rt_err(msg: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(msg)
}

fn api_err(e: ApiError) -> mlua::Error {
    rt_err(e)
}

fn outside_call() -> mlua::Error {
    rt_err("the bh API can only be used while the host runs the mod")
}

/// Runs `f` with the host API of the call in progress.
fn with_host<R>(sh: &Shared, f: impl FnOnce(&mut dyn HostApi) -> R) -> mlua::Result<R> {
    let Some(p) = sh.host.get() else { return Err(outside_call()) };
    // SAFETY: `p` was created in `enter` from a `&mut dyn HostApi` that is alive for the whole guest call and is
    // reset to `None` before `enter` returns. Lua runs on this thread only, and no host function calls back into
    // Lua, so this is the only live reference derived from it.
    Ok(f(unsafe { &mut *p }))
}

/// Clears the host pointer even if the guest call unwinds.
struct Entered<'a>(&'a Shared);

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        self.0.host.set(None);
        self.0.deadline.set(None);
    }
}

/// Enters the VM for one guest call.
fn enter<R>(sh: &Shared, host: &mut dyn HostApi, f: impl FnOnce() -> mlua::Result<R>) -> Result<R, CallError> {
    enter_as(sh, host, false, f)
}

fn is_memory_error(e: &mlua::Error) -> bool {
    match e {
        mlua::Error::MemoryError(_) => true,
        mlua::Error::CallbackError { cause, .. } => is_memory_error(cause),
        _ => false,
    }
}

/// Enters the VM; with `soft`, a Luau error (not a timeout or out of memory) is [`CallError::Failed`], not a fault.
fn enter_as<R>(
    sh: &Shared,
    host: &mut dyn HostApi,
    soft: bool,
    f: impl FnOnce() -> mlua::Result<R>,
) -> Result<R, CallError> {
    sh.deadline.set(Some(host.deadline()));
    sh.timed_out.set(false);
    let ptr: *mut (dyn HostApi + '_) = host;
    // SAFETY: only the lifetime is erased; see `with_host` and `Entered` for why the pointer never dangles in use.
    let ptr: HostPtr = unsafe { std::mem::transmute::<*mut (dyn HostApi + '_), HostPtr>(ptr) };
    sh.host.set(Some(ptr));
    let _guard = Entered(sh);
    let r = f();
    if sh.timed_out.get() {
        return Err(CallError::Timeout);
    }
    match r {
        Ok(v) => Ok(v),
        Err(e) if soft && !is_memory_error(&e) => Err(CallError::Failed(error_text(&e))),
        Err(e) => Err(CallError::Trap(error_text(&e))),
    }
}

fn error_text(e: &mlua::Error) -> String {
    match e {
        mlua::Error::MemoryError(_) => "out of memory".to_string(),
        mlua::Error::RuntimeError(m) => m.clone(),
        mlua::Error::CallbackError { cause, .. } => error_text(cause),
        other => other.to_string(),
    }
}

/// A player handle (`players.player` resource) as userdata.
struct LuaPlayer {
    id: String,
    sh: Rc<Shared>,
}

impl UserData for LuaPlayer {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("id", |_, this, ()| Ok(this.id.clone()));
        m.add_method("alias", |_, this, ()| with_host(&this.sh, |h| h.player_alias(&this.id).unwrap_or_default()));
        m.add_method("is_online", |_, this, ()| with_host(&this.sh, |h| h.player_is_online(&this.id)));
        m.add_method("role", |_, this, ()| {
            with_host(&this.sh, |h| h.player_role(&this.id).unwrap_or(Role::Player).name())
        });
        m.add_method("send_chat", |_, this, text: String| {
            with_host(&this.sh, |h| h.player_send_chat(&this.id, &text))?.map_err(api_err)
        });
        m.add_method("kick", |_, this, reason: String| {
            with_host(&this.sh, |h| h.player_kick(&this.id, &reason))?.map_err(api_err)
        });
        m.add_method("value_handle", |_, this, ()| with_host(&this.sh, |h| h.handle_of_player(&this.id)));
        m.add_method("pos", |lua, this, ()| match with_host(&this.sh, |h| h.player_pos(&this.id))? {
            Ok(Some(p)) => Ok(Value::Table(pos_table(lua, p)?)),
            _ => Ok(Value::Nil),
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("player({})", this.id)));
        m.add_meta_method(MetaMethod::Eq, |_, this, other: AnyUserData| {
            Ok(other.borrow::<LuaPlayer>().is_ok_and(|o| o.id == this.id))
        });
    }
}

const HANDLERS: &str = "bh.handlers";
const COMMANDS: &str = "bh.commands";
const TASKS: &str = "bh.tasks";
const LOADED: &str = "bh.loaded";
const SERVICES: &str = "bh.services";
const CHANNELS: &str = "bh.channels";

/// One Luau mod.
pub struct LuaInstance {
    lua: Lua,
    sh: Rc<Shared>,
    module: Option<Table>,
}

/// Resolves `require` names (`./x`, `../x`, relative to the entry's directory, never above the side directory).
pub fn resolve_require(entry: &str, name: &str) -> Option<String> {
    if !(name.starts_with("./") || name.starts_with("../")) {
        return None;
    }
    let base = entry.rsplit_once('/').map_or("", |(d, _)| d);
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in name.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.len() <= 1 {
                    return None;
                }
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut p = parts.join("/");
    if !p.ends_with(".luau") {
        p.push_str(".luau");
    }
    Some(p)
}

/// Adds Rust functions to tables and keeps the first error (so call sites need no `?`).
struct Builder<'a> {
    lua: &'a Lua,
    err: Option<mlua::Error>,
}

impl Builder<'_> {
    fn func<F, A, R>(&mut self, t: &Table, name: &str, f: F)
    where
        F: Fn(&Lua, A) -> mlua::Result<R> + 'static,
        A: mlua::FromLuaMulti,
        R: mlua::IntoLuaMulti,
    {
        let r = self.lua.create_function(f).and_then(|f| t.set(name, f));
        self.err = self.err.take().or(r.err());
    }

    fn finish(self) -> mlua::Result<()> {
        self.err.map_or(Ok(()), Err)
    }
}

fn player(lua: &Lua, sh: &Rc<Shared>, id: String) -> mlua::Result<AnyUserData> {
    lua.create_userdata(LuaPlayer { id, sh: sh.clone() })
}

fn sender_table(lua: &Lua, sh: &Rc<Shared>, s: &CommandSender) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    match s {
        CommandSender::Console => t.set("console", true)?,
        CommandSender::Player(id) => t.set("player", player(lua, sh, id.clone())?)?,
    }
    Ok(t)
}

fn pos_table(lua: &Lua, p: Pos) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("x", p.x)?;
    t.set("y", p.y)?;
    Ok(t)
}

fn opt_player(lua: &Lua, sh: &Rc<Shared>, p: &Option<String>) -> mlua::Result<Value> {
    match p {
        Some(id) => Ok(Value::UserData(player(lua, sh, id.clone())?)),
        None => Ok(Value::Nil),
    }
}

fn bad_value(msg: impl std::fmt::Display) -> mlua::Error {
    rt_err(format!("bad value: {msg}"))
}

/// A Luau integer as i64 (`mlua::Integer` is 32 bits wide on 32-bit targets).
#[allow(clippy::useless_conversion)]
fn widen(i: mlua::Integer) -> i64 {
    i.into()
}

/// Integers up to 2^53 stay exact in a Luau number.
const MAX_SAFE: f64 = 9_007_199_254_740_992.0;

/// Converts a Luau value to a [`Cbor`] value (players become host handles).
fn to_cbor(sh: &Shared, v: &Value, depth: usize) -> mlua::Result<Cbor> {
    if depth > value::MAX_DEPTH {
        return Err(bad_value("nested too deeply"));
    }
    Ok(match v {
        Value::Nil => Cbor::Null,
        Value::Boolean(b) => Cbor::Bool(*b),
        Value::Integer(i) => Cbor::Int(widen(*i)),
        Value::Number(n) if n.fract() == 0.0 && n.abs() <= MAX_SAFE => Cbor::Int(*n as i64),
        Value::Number(n) => Cbor::Float(*n),
        Value::String(s) => match std::str::from_utf8(&s.as_bytes()) {
            Ok(t) => Cbor::Text(t.to_string()),
            Err(_) => Cbor::Bytes(s.as_bytes().to_vec()),
        },
        Value::Table(t) => {
            let n = t.raw_len();
            let mut count = 0;
            let mut all_str = true;
            for pair in t.pairs::<Value, Value>() {
                let (k, _) = pair?;
                count += 1;
                all_str &= matches!(k, Value::String(_));
            }
            if count == n {
                let items: mlua::Result<Vec<Cbor>> =
                    (1..=n).map(|i| to_cbor(sh, &t.raw_get::<Value>(i)?, depth + 1)).collect();
                Cbor::List(items?)
            } else if all_str {
                let mut entries = Vec::with_capacity(count);
                for pair in t.pairs::<mlua::LuaString, Value>() {
                    let (k, v) = pair?;
                    entries.push((k.to_str()?.to_string(), to_cbor(sh, &v, depth + 1)?));
                }
                Cbor::Map(entries)
            } else {
                return Err(bad_value("a table must be a list (keys 1..n) or have only string keys"));
            }
        }
        Value::UserData(u) => {
            // Players are the only userdata the API hands out.
            let p = u.borrow::<LuaPlayer>().map_err(|_| bad_value("unsupported userdata"))?;
            Cbor::handle(with_host(sh, |h| h.handle_of_player(&p.id))?)
        }
        other => return Err(bad_value(format!("cannot convert a {}", other.type_name()))),
    })
}

/// Converts a [`Cbor`] value to Luau (host handles become players; a stale handle raises).
fn from_cbor(lua: &Lua, sh: &Rc<Shared>, v: &Cbor) -> mlua::Result<Value> {
    if let Some(h) = v.as_handle() {
        let id = with_host(sh, |host| host.player_of_handle(h))?.map_err(api_err)?;
        return Ok(Value::UserData(player(lua, sh, id)?));
    }
    Ok(match v {
        Cbor::Null => Value::Nil,
        Cbor::Bool(b) => Value::Boolean(*b),
        Cbor::Int(i) => Value::Number(*i as f64),
        Cbor::Float(f) => Value::Number(*f),
        Cbor::Text(t) => Value::String(lua.create_string(t)?),
        Cbor::Bytes(b) => Value::String(lua.create_string(b)?),
        Cbor::List(items) => {
            let t = lua.create_table_with_capacity(items.len(), 0)?;
            for (i, item) in items.iter().enumerate() {
                t.raw_set(i + 1, from_cbor(lua, sh, item)?)?;
            }
            Value::Table(t)
        }
        Cbor::Map(entries) => {
            let t = lua.create_table_with_capacity(0, entries.len())?;
            for (k, item) in entries {
                t.raw_set(k.as_str(), from_cbor(lua, sh, item)?)?;
            }
            Value::Table(t)
        }
    })
}

fn decode_cbor(lua: &Lua, sh: &Rc<Shared>, bytes: &[u8]) -> mlua::Result<Value> {
    from_cbor(lua, sh, &Cbor::decode(bytes).map_err(bad_value)?)
}

/// Calls a service and converts the result.
fn call_service(
    lua: &Lua,
    sh: &Rc<Shared>,
    name: &str,
    req: &str,
    method: &str,
    args: MultiValue,
) -> mlua::Result<Value> {
    let args = args.iter().map(|a| to_cbor(sh, a, 1)).collect::<mlua::Result<Vec<_>>>()?;
    let bytes = Cbor::List(args).encode();
    let out = with_host(sh, |h| h.call_service(name, req, method, &bytes))?
        .map_err(|e| rt_err(format!("service {name}.{method}: {e}")))?;
    decode_cbor(lua, sh, &out)
}

fn channel_target(v: Value) -> mlua::Result<ChannelTarget> {
    match &v {
        Value::String(s) => match &*s.to_str()? {
            "local" => Ok(ChannelTarget::Local),
            "server" => Ok(ChannelTarget::Server),
            "all-players" => Ok(ChannelTarget::AllPlayers),
            other => Err(rt_err(format!("invalid argument: channel target {other:?}"))),
        },
        Value::UserData(u) => {
            let p = u.borrow::<LuaPlayer>().map_err(|_| rt_err("invalid argument: channel target"))?;
            Ok(ChannelTarget::Player(p.id.clone()))
        }
        _ => Err(rt_err("invalid argument: channel target")),
    }
}

fn source_table(lua: &Lua, sh: &Rc<Shared>, src: &ChannelSource) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    match src {
        ChannelSource::Local(m) => t.set("mod", m.as_str())?,
        ChannelSource::Server => t.set("server", true)?,
        ChannelSource::Player(id) => t.set("player", player(lua, sh, id.clone())?)?,
    }
    Ok(t)
}

fn event_table(lua: &Lua, sh: &Rc<Shared>, ev: &Event) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("kind", ev.kind().name())?;
    match ev {
        Event::PlayerJoin { player: p } => t.set("player", player(lua, sh, p.clone())?)?,
        Event::PlayerQuit { player: p, was_kick } => {
            t.set("player", player(lua, sh, p.clone())?)?;
            t.set("was_kick", *was_kick)?;
        }
        Event::Chat { player: p, text, cancelled } => {
            t.set("player", player(lua, sh, p.clone())?)?;
            t.set("text", text.as_str())?;
            t.set("cancelled", *cancelled)?;
        }
        Event::Command { sender, name, args, raw, cancelled } => {
            t.set("sender", sender_table(lua, sh, sender)?)?;
            t.set("name", name.as_str())?;
            t.set("args", args.clone())?;
            t.set("raw", raw.as_str())?;
            t.set("cancelled", *cancelled)?;
        }
        Event::Tick { tick, dt } => {
            t.set("tick", *tick)?;
            t.set("dt", *dt)?;
        }
        Event::WorldLoaded { world_name, world_id } => {
            t.set("world_name", world_name.as_str())?;
            t.set("world_id", world_id.as_str())?;
        }
        Event::WorldSave { phase } => t.set("phase", phase.name())?,
        Event::PlayerPreLogin { player: p, alias, cancelled, reason } => {
            t.set("player_id", p.as_str())?;
            t.set("alias", alias.as_str())?;
            t.set("cancelled", *cancelled)?;
            t.set("reason", reason.name())?;
        }
        Event::BlockPlace { player: p, pos, item, data_a, data_b, cancelled } => {
            t.set("player", player(lua, sh, p.clone())?)?;
            t.set("pos", pos_table(lua, *pos)?)?;
            t.set("item", *item)?;
            t.set("data_a", *data_a)?;
            t.set("data_b", *data_b)?;
            t.set("cancelled", *cancelled)?;
        }
        Event::BlockBreak { player: p, pos, layer, tile, cancelled } => {
            t.set("player", player(lua, sh, p.clone())?)?;
            t.set("pos", pos_table(lua, *pos)?)?;
            t.set("layer", layer.name())?;
            t.set("tile", *tile)?;
            t.set("cancelled", *cancelled)?;
        }
        Event::BlockChanged { pos, tile, player: p } => {
            t.set("pos", pos_table(lua, *pos)?)?;
            t.set("tile", *tile)?;
            t.set("player", opt_player(lua, sh, p)?)?;
        }
        Event::ObjectPlace { player: p, pos, object, item, cancelled } => {
            t.set("player", player(lua, sh, p.clone())?)?;
            t.set("pos", pos_table(lua, *pos)?)?;
            t.set("object", object.name())?;
            t.set("item", *item)?;
            t.set("cancelled", *cancelled)?;
        }
        Event::Damage { player: p, blockhead, amount, attacker, cancelled } => {
            t.set("player", opt_player(lua, sh, p)?)?;
            t.set("blockhead", *blockhead)?;
            t.set("amount", *amount)?;
            t.set("attacker", opt_player(lua, sh, attacker)?)?;
            t.set("cancelled", *cancelled)?;
        }
        Event::Death { player: p, blockhead } => {
            t.set("player", opt_player(lua, sh, p)?)?;
            t.set("blockhead", *blockhead)?;
        }
        Event::PlayerMove { player: p, from, to } => {
            t.set("player", player(lua, sh, p.clone())?)?;
            t.set("from", pos_table(lua, *from)?)?;
            t.set("to", pos_table(lua, *to)?)?;
        }
        Event::Custom { name, source, payload, cancelled } => {
            t.set("name", name.as_str())?;
            t.set("source", source.as_str())?;
            t.set("payload", decode_cbor(lua, sh, payload)?)?;
            t.set("cancelled", *cancelled)?;
        }
    }
    Ok(t)
}

/// The change a handler made to the event table.
fn mutation_of(sh: &Shared, t: &Table, ev: &Event) -> mlua::Result<Option<Mutation>> {
    Ok(match ev {
        Event::Chat { text, .. } => match t.get::<Value>("text")? {
            Value::String(s) if s.to_string_lossy() != *text => Some(Mutation::ChatText(s.to_string_lossy())),
            _ => None,
        },
        Event::PlayerPreLogin { reason, .. } => {
            let r: String = t.get("reason")?;
            match RejectReason::parse(&r) {
                Some(r) if r != *reason => Some(Mutation::RejectReason(r)),
                Some(_) => None,
                None => return Err(rt_err(format!("invalid argument: reject reason {r:?}"))),
            }
        }
        Event::Damage { amount, .. } => {
            let a: f64 = t.get("amount")?;
            (a as f32 != *amount).then_some(Mutation::DamageAmount(a as f32))
        }
        Event::Custom { payload, .. } => {
            let now = to_cbor(sh, &t.get::<Value>("payload")?, 1)?.encode();
            (now != *payload).then_some(Mutation::CustomPayload(now))
        }
        _ => None,
    })
}

fn sub_opts(opts: Option<Table>) -> mlua::Result<(Priority, bool)> {
    let mut prio = Priority::Normal;
    let mut ignore = false;
    if let Some(o) = opts {
        if let Some(p) = o.get::<Option<String>>("priority")? {
            prio = Priority::parse(&p).ok_or_else(|| rt_err(format!("invalid argument: priority {p:?}")))?;
        }
        ignore = o.get::<Option<bool>>("ignore_cancelled")?.unwrap_or(false);
    }
    Ok((prio, ignore))
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Nil | Value::Boolean(false))
}

fn string_list(v: Value) -> mlua::Result<Vec<String>> {
    match v {
        Value::Nil => Ok(Vec::new()),
        Value::String(s) => Ok(vec![s.to_string_lossy()]),
        Value::Table(t) => t.sequence_values::<String>().collect(),
        _ => Err(rt_err("expected a string or a list of strings")),
    }
}

fn command_outcome(v: Value) -> mlua::Result<CommandOutcome> {
    match v {
        Value::Nil => Ok(CommandOutcome { handled: true, reply: Vec::new() }),
        Value::String(s) => Ok(CommandOutcome { handled: true, reply: vec![s.to_string_lossy()] }),
        Value::Table(t) => {
            let handled = t.get::<Value>("handled")?;
            Ok(CommandOutcome {
                handled: matches!(handled, Value::Nil) || truthy(&handled),
                reply: string_list(t.get("reply")?)?,
            })
        }
        _ => Err(rt_err("a command handler must return nil, a string or { handled, reply }")),
    }
}

fn command_spec(t: &Table) -> mlua::Result<CommandSpec> {
    let name: String =
        t.get::<Option<String>>("name")?.ok_or_else(|| rt_err("invalid argument: spec.name is required"))?;
    let mut spec = CommandSpec::new(&name);
    spec.aliases = string_list(t.get("aliases")?)?;
    if let Some(d) = t.get::<Option<String>>("description")? {
        spec.description = d;
    }
    if let Some(u) = t.get::<Option<String>>("usage")? {
        spec.usage = u;
    }
    if let Some(r) = t.get::<Option<String>>("min_role")? {
        spec.min_role = Role::parse(&r).ok_or_else(|| rt_err(format!("invalid argument: unknown role {r:?}")))?;
    }
    if let Some(c) = t.get::<Option<bool>>("console")? {
        spec.console = c;
    }
    Ok(spec)
}

impl LuaInstance {
    /// Creates the VM, installs `bh` and compiles nothing yet (the entry runs in `on_enable`).
    pub fn new(mod_id: &str, files: ModFiles, limits: InstanceLimits) -> mlua::Result<LuaInstance> {
        if !files.files.contains_key(&files.entry) {
            return Err(rt_err(format!("{mod_id}: entry point {} is missing", files.entry)));
        }
        let lua = Lua::new();
        let sh =
            Rc::new(Shared { host: Cell::new(None), deadline: Cell::new(None), timed_out: Cell::new(false), files });
        let g = lua.globals();
        for name in ["debug", "loadstring", "getfenv", "setfenv"] {
            g.set(name, Value::Nil)?;
        }
        for reg in [HANDLERS, COMMANDS, TASKS, LOADED, SERVICES, CHANNELS] {
            lua.set_named_registry_value(reg, lua.create_table()?)?;
        }
        let bh = Self::build_bh(&lua, &sh)?;
        let mut b = Builder { lua: &lua, err: None };
        g.set("bh", bh)?;
        let s = sh.clone();
        b.func(&g, "print", move |_, args: Variadic<Value>| {
            let line: Vec<String> = args.iter().map(|v| v.to_string().unwrap_or_default()).collect();
            with_host(&s, |h| h.log(LogLevel::Info, &line.join("\t")))
        });
        let s = sh.clone();
        b.func(&g, "require", move |lua, name: String| {
            let path = resolve_require(&s.files.entry, &name)
                .ok_or_else(|| rt_err(format!("require({name:?}): only ./ and ../ paths inside the package")))?;
            let loaded: Table = lua.named_registry_value(LOADED)?;
            let cached: Value = loaded.get(path.as_str())?;
            if !cached.is_nil() {
                return Ok(cached);
            }
            let src = s.files.files.get(&path).ok_or_else(|| rt_err(format!("require: {path} not found")))?;
            let v: Value = lua.load(&src[..]).set_name(format!("={path}")).eval()?;
            let v = if v.is_nil() { Value::Boolean(true) } else { v };
            loaded.set(path, v.clone())?;
            Ok(v)
        });
        b.finish()?;
        lua.sandbox(true)?;
        let s = sh.clone();
        lua.set_interrupt(move |_| {
            if s.deadline.get().is_some_and(|d| Instant::now() >= d) {
                s.timed_out.set(true);
                return Err(rt_err("time budget exceeded"));
            }
            Ok(VmState::Continue)
        });
        lua.set_memory_limit(limits.memory_bytes)?;
        Ok(LuaInstance { lua, sh, module: None })
    }

    fn build_bh(lua: &Lua, sh: &Rc<Shared>) -> mlua::Result<Table> {
        let mut b = Builder { lua, err: None };
        let bh = lua.create_table()?;

        let host = lua.create_table()?;
        let s = sh.clone();
        host.set("api_version", lua.create_function(move |_, ()| with_host(&s, |h| h.api_version()))?)?;
        let s = sh.clone();
        host.set("mod_id", lua.create_function(move |_, ()| with_host(&s, |h| h.mod_id()))?)?;
        let s = sh.clone();
        host.set("mod_version", lua.create_function(move |_, ()| with_host(&s, |h| h.mod_version()))?)?;
        let s = sh.clone();
        host.set("current_side", lua.create_function(move |_, ()| with_host(&s, |h| h.current_side().name()))?)?;
        let s = sh.clone();
        host.set("host_name", lua.create_function(move |_, ()| with_host(&s, |h| h.host_name()))?)?;
        bh.set("host", host)?;

        let log = lua.create_table()?;
        let s = sh.clone();
        b.func(&log, "log", move |_, (level, msg): (String, String)| {
            let level = LogLevel::parse(&level).ok_or_else(|| rt_err(format!("invalid argument: level {level:?}")))?;
            with_host(&s, |h| h.log(level, &msg))
        });
        for level in LogLevel::ALL {
            let s = sh.clone();
            log.set(level.name(), lua.create_function(move |_, msg: String| with_host(&s, |h| h.log(level, &msg)))?)?;
        }
        bh.set("log", log)?;

        let players = lua.create_table()?;
        let s = sh.clone();
        b.func(&players, "get", move |lua, id: String| match with_host(&s, |h| h.players_get(&id))? {
            Some(id) => Ok(Some(player(lua, &s, id)?)),
            None => Ok(None),
        });
        let s = sh.clone();
        b.func(&players, "find_by_alias", move |lua, alias: String| {
            match with_host(&s, |h| h.players_find_by_alias(&alias))? {
                Some(id) => Ok(Some(player(lua, &s, id)?)),
                None => Ok(None),
            }
        });
        let s = sh.clone();
        b.func(&players, "from_value_handle", move |lua, handle: u64| {
            match with_host(&s, |h| h.player_of_handle(handle))? {
                Ok(id) => Ok(Some(player(lua, &s, id)?)),
                Err(_) => Ok(None),
            }
        });
        let s = sh.clone();
        b.func(&players, "online", move |lua, ()| {
            let ids = with_host(&s, |h| h.players_online())?;
            ids.into_iter().map(|id| player(lua, &s, id)).collect::<mlua::Result<Vec<_>>>()
        });
        let s = sh.clone();
        b.func(&players, "broadcast", move |_, text: String| {
            with_host(&s, |h| h.players_broadcast(&text))?.map_err(api_err)
        });
        bh.set("players", players)?;

        let content = lua.create_table()?;
        fn kind_of(k: &str) -> mlua::Result<ContentKind> {
            ContentKind::parse(k).ok_or_else(|| rt_err(format!("invalid argument: unknown content kind {k:?}")))
        }
        let s = sh.clone();
        b.func(&content, "id_of", move |_, (kind, key): (String, String)| {
            let kind = kind_of(&kind)?;
            with_host(&s, |h| h.content_id(kind, &key))
        });
        let s = sh.clone();
        b.func(&content, "key_of", move |_, (kind, id): (String, u64)| {
            let kind = kind_of(&kind)?;
            with_host(&s, |h| h.content_key(kind, id))
        });
        let s = sh.clone();
        b.func(&content, "cosmetics", move |lua, ()| {
            let list = with_host(&s, |h| h.content_cosmetics())?;
            let out = lua.create_table()?;
            for c in list {
                let t = lua.create_table()?;
                t.set("key", c.key)?;
                t.set("mod_id", c.mod_id)?;
                t.set("slot", c.slot)?;
                t.set("id", c.id)?;
                out.push(t)?;
            }
            Ok(out)
        });
        let s = sh.clone();
        b.func(&content, "equipped", move |lua, who: AnyUserData| {
            let id = who.borrow::<LuaPlayer>().map_err(|_| rt_err("invalid argument: player"))?.id.clone();
            let pairs = with_host(&s, |h| h.content_equipped(&id))?.map_err(api_err)?;
            let out = lua.create_table()?;
            for (slot, key) in pairs {
                out.set(slot, key)?;
            }
            Ok(out)
        });
        let s = sh.clone();
        b.func(&content, "equip", move |_, (who, key): (AnyUserData, String)| {
            let id = who.borrow::<LuaPlayer>().map_err(|_| rt_err("invalid argument: player"))?.id.clone();
            with_host(&s, |h| h.content_equip(&id, &key))?.map_err(api_err)
        });
        let s = sh.clone();
        b.func(&content, "unequip", move |_, (who, slot): (AnyUserData, String)| {
            let id = who.borrow::<LuaPlayer>().map_err(|_| rt_err("invalid argument: player"))?.id.clone();
            with_host(&s, |h| h.content_unequip(&id, &slot))?.map_err(api_err)
        });
        bh.set("content", content)?;

        let events = lua.create_table()?;
        let s = sh.clone();
        b.func(&events, "on", move |lua, (kind, f, opts): (String, Function, Option<Table>)| {
            let kind =
                EventKind::parse(&kind).ok_or_else(|| rt_err(format!("invalid argument: unknown event {kind:?}")))?;
            let (prio, ignore) = sub_opts(opts)?;
            let id = with_host(&s, |h| h.subscribe(kind, prio, ignore))?.map_err(api_err)?;
            lua.named_registry_value::<Table>(HANDLERS)?.set(id, f)?;
            Ok(id)
        });
        let s = sh.clone();
        b.func(&events, "off", move |lua, id: SubscriptionId| {
            lua.named_registry_value::<Table>(HANDLERS)?.set(id, Value::Nil)?;
            with_host(&s, |h| h.unsubscribe(id))
        });
        let s = sh.clone();
        b.func(&events, "on_custom", move |lua, (name, f, opts): (String, Function, Option<Table>)| {
            let (prio, ignore) = sub_opts(opts)?;
            let id = with_host(&s, |h| h.subscribe_custom(&name, prio, ignore))?.map_err(api_err)?;
            lua.named_registry_value::<Table>(HANDLERS)?.set(id, f)?;
            Ok(id)
        });
        let s = sh.clone();
        b.func(&events, "fire_custom", move |_, (name, payload): (String, Value)| {
            let bytes = to_cbor(&s, &payload, 1)?.encode();
            with_host(&s, |h| h.fire_custom(&name, &bytes))?.map_err(api_err)
        });
        bh.set("events", events)?;

        let commands = lua.create_table()?;
        let s = sh.clone();
        b.func(&commands, "register", move |lua, (spec, f): (Table, Function)| {
            let spec = command_spec(&spec)?;
            let id = with_host(&s, |h| h.register_command(spec))?.map_err(api_err)?;
            lua.named_registry_value::<Table>(COMMANDS)?.set(id, f)?;
            Ok(id)
        });
        let s = sh.clone();
        b.func(&commands, "unregister", move |lua, id: CommandId| {
            lua.named_registry_value::<Table>(COMMANDS)?.set(id, Value::Nil)?;
            with_host(&s, |h| h.unregister_command(id))
        });
        bh.set("commands", commands)?;

        let sched = lua.create_table()?;
        fn store_task(lua: &Lua, id: TaskId, f: Function, repeat: bool) -> mlua::Result<TaskId> {
            let entry = lua.create_table()?;
            entry.set("f", f)?;
            entry.set("r", repeat)?;
            lua.named_registry_value::<Table>(TASKS)?.set(id, entry)?;
            Ok(id)
        }
        let s = sh.clone();
        b.func(&sched, "run_later", move |lua, (delay, f): (u32, Function)| {
            let id = with_host(&s, |h| h.run_later(delay))?.map_err(api_err)?;
            store_task(lua, id, f, false)
        });
        let s = sh.clone();
        b.func(&sched, "run_every", move |lua, (delay, period, f): (u32, u32, Function)| {
            let id = with_host(&s, |h| h.run_every(delay, period))?.map_err(api_err)?;
            store_task(lua, id, f, true)
        });
        let s = sh.clone();
        b.func(&sched, "run_next_poll", move |lua, f: Function| {
            let id = with_host(&s, |h| h.run_next_poll())?.map_err(api_err)?;
            store_task(lua, id, f, false)
        });
        let s = sh.clone();
        b.func(&sched, "cancel", move |lua, id: TaskId| {
            lua.named_registry_value::<Table>(TASKS)?.set(id, Value::Nil)?;
            with_host(&s, |h| h.cancel_task(id))
        });
        bh.set("scheduler", sched)?;

        let storage = lua.create_table()?;
        let s = sh.clone();
        b.func(&storage, "get", move |lua, key: String| match with_host(&s, |h| h.storage_get(&key))? {
            Some(v) => Ok(Some(lua.create_string(v)?)),
            None => Ok(None),
        });
        let s = sh.clone();
        b.func(&storage, "set", move |_, (key, value): (String, mlua::LuaString)| {
            with_host(&s, |h| h.storage_set(&key, &value.as_bytes()))?.map_err(api_err)
        });
        let s = sh.clone();
        storage.set("delete", lua.create_function(move |_, key: String| with_host(&s, |h| h.storage_delete(&key)))?)?;
        let s = sh.clone();
        b.func(&storage, "keys", move |_, prefix: Option<String>| {
            with_host(&s, |h| h.storage_keys(prefix.as_deref().unwrap_or("")))
        });
        bh.set("storage", storage)?;

        let services = lua.create_table()?;
        let s = sh.clone();
        b.func(&services, "provide", move |lua, (name, version, imp): (String, String, Table)| {
            with_host(&s, |h| h.provide_service(&name, &version))?.map_err(api_err)?;
            lua.named_registry_value::<Table>(SERVICES)?.set(name, imp)
        });
        let s = sh.clone();
        b.func(&services, "call", move |lua, (name, req, method, args): (String, String, String, MultiValue)| {
            call_service(lua, &s, &name, &req, &method, args)
        });
        let s = sh.clone();
        b.func(&services, "get", move |lua, (name, req): (String, String)| {
            let proxy = lua.create_table()?;
            let meta = lua.create_table()?;
            let s = s.clone();
            let index = move |lua: &Lua, (_, method): (Table, String)| {
                let (s, name, req) = (s.clone(), name.clone(), req.clone());
                lua.create_function(move |lua, args: MultiValue| call_service(lua, &s, &name, &req, &method, args))
            };
            meta.set("__index", lua.create_function(index)?)?;
            proxy.set_metatable(Some(meta))?;
            Ok(proxy)
        });
        bh.set("services", services)?;

        let channels = lua.create_table()?;
        let s = sh.clone();
        b.func(&channels, "open", move |lua, (name, f): (String, Option<Function>)| {
            let id = with_host(&s, |h| h.channel_open(&name))?.map_err(api_err)?;
            if let Some(f) = f {
                lua.named_registry_value::<Table>(CHANNELS)?.set(id, f)?;
            }
            Ok(id)
        });
        let s = sh.clone();
        b.func(&channels, "send", move |_, (id, to, data): (ChannelId, Value, mlua::LuaString)| {
            let to = channel_target(to)?;
            with_host(&s, |h| h.channel_send(id, to, &data.as_bytes()))?.map_err(api_err)
        });
        let s = sh.clone();
        b.func(&channels, "close", move |lua, id: ChannelId| {
            lua.named_registry_value::<Table>(CHANNELS)?.set(id, Value::Nil)?;
            with_host(&s, |h| h.channel_close(id))
        });
        bh.set("channels", channels)?;

        let val = lua.create_table()?;
        let s = sh.clone();
        b.func(&val, "encode", move |lua, v: Value| lua.create_string(to_cbor(&s, &v, 1)?.encode()));
        let s = sh.clone();
        b.func(&val, "decode", move |lua, bytes: mlua::LuaString| decode_cbor(lua, &s, &bytes.as_bytes()));
        bh.set("value", val)?;
        for (_, v) in bh.pairs::<String, Table>().collect::<mlua::Result<Vec<_>>>()? {
            v.set_readonly(true);
        }
        b.finish()?;
        Ok(bh)
    }
}

impl Instance for LuaInstance {
    fn on_enable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        let lua = &self.lua;
        let sh = &self.sh;
        let module = enter(sh, host, || {
            let src = &sh.files.files[&sh.files.entry];
            let v: Value = lua.load(&src[..]).set_name(format!("={}", sh.files.entry)).eval()?;
            let module = match v {
                Value::Table(t) => Some(t),
                Value::Nil => None,
                _ => return Err(rt_err(format!("{} must return a table or nil", sh.files.entry))),
            };
            if let Some(f) = module.as_ref().map(|m| m.get::<Option<Function>>("on_enable")).transpose()?.flatten() {
                f.call::<()>(())?;
            }
            Ok(module)
        })
        .map_err(|e| match e {
            CallError::Trap(m) => CallError::Failed(m),
            other => other,
        })?;
        self.module = module;
        Ok(())
    }

    fn on_disable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        let module = self.module.clone();
        enter(&self.sh, host, || {
            let f = module.map(|m| m.get::<Option<Function>>("on_disable")).transpose()?.flatten();
            match f {
                Some(f) => f.call::<()>(()),
                None => Ok(()),
            }
        })
    }

    fn handle_event(&mut self, host: &mut dyn HostApi, sub: SubscriptionId, ev: &Event) -> Result<Outcome, CallError> {
        let lua = &self.lua;
        let sh = &self.sh;
        enter(sh, host, || {
            let f: Option<Function> = lua.named_registry_value::<Table>(HANDLERS)?.get(sub)?;
            let Some(f) = f else { return Ok(Outcome { cancelled: ev.cancelled(), mutation: None }) };
            let t = event_table(lua, sh, ev)?;
            f.call::<()>(t.clone())?;
            let cancelled = truthy(&t.get::<Value>("cancelled")?);
            let mutation = mutation_of(sh, &t, ev)?;
            Ok(Outcome { cancelled: ev.kind().cancellable() && cancelled, mutation })
        })
    }

    fn run_command(
        &mut self,
        host: &mut dyn HostApi,
        id: CommandId,
        sender: &CommandSender,
        label: &str,
        args: &[String],
    ) -> Result<CommandOutcome, CallError> {
        let lua = &self.lua;
        let sh = &self.sh;
        enter(sh, host, || {
            let f: Option<Function> = lua.named_registry_value::<Table>(COMMANDS)?.get(id)?;
            let Some(f) = f else { return Err(rt_err(format!("command {id} has no handler"))) };
            command_outcome(f.call((sender_table(lua, sh, sender)?, label, args.to_vec()))?)
        })
    }

    fn run_task(&mut self, host: &mut dyn HostApi, id: TaskId) -> Result<(), CallError> {
        let lua = &self.lua;
        enter(&self.sh, host, || {
            let tasks: Table = lua.named_registry_value(TASKS)?;
            let Some(entry) = tasks.get::<Option<Table>>(id)? else { return Ok(()) };
            if !entry.get::<bool>("r")? {
                tasks.set(id, Value::Nil)?;
            }
            entry.get::<Function>("f")?.call::<()>(())
        })
    }

    fn call_service(
        &mut self,
        host: &mut dyn HostApi,
        name: &str,
        method: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, CallError> {
        let lua = &self.lua;
        let sh = &self.sh;
        enter_as(sh, host, true, || {
            let imp: Option<Table> = lua.named_registry_value::<Table>(SERVICES)?.get(name)?;
            let f: Option<Function> = imp.map(|t| t.get(method)).transpose()?.flatten();
            let Some(f) = f else { return Err(rt_err(format!("{name} has no method {method:?}"))) };
            // The host passes a list (checked); anything else counts as one argument.
            let v = Cbor::decode(args).map_err(bad_value)?;
            let items = if let Cbor::List(i) = v { i } else { Vec::from([v]) };
            let args = items.iter().map(|a| from_cbor(lua, sh, a)).collect::<mlua::Result<Vec<_>>>()?;
            let out: MultiValue = f.call(MultiValue::from_vec(args))?;
            Ok(to_cbor(sh, out.front().unwrap_or(&Value::Nil), 1)?.encode())
        })
    }

    fn on_channel_message(
        &mut self,
        host: &mut dyn HostApi,
        channel: ChannelId,
        source: &ChannelSource,
        data: &[u8],
    ) -> Result<(), CallError> {
        let lua = &self.lua;
        let sh = &self.sh;
        enter(sh, host, || {
            let f: Option<Function> = lua.named_registry_value::<Table>(CHANNELS)?.get(channel)?;
            let Some(f) = f else { return Ok(()) };
            f.call::<()>((source_table(lua, sh, source)?, lua.create_string(data)?))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bh_outside_a_host_call_raises() {
        let files =
            ModFiles { entry: "server/main.luau".into(), files: [("server/main.luau".into(), b"".to_vec())].into() };
        let inst = LuaInstance::new("m", files, InstanceLimits { memory_bytes: 1 << 24 }).unwrap();
        let err = inst.lua.load("bh.log.info('x')").exec().unwrap_err();
        assert!(err.to_string().contains("only be used while the host runs the mod"), "{err}");
    }

    #[test]
    fn require_paths() {
        assert_eq!(resolve_require("server/main.luau", "./a/b").as_deref(), Some("server/a/b.luau"));
        assert_eq!(resolve_require("server/main.luau", "./a/./b.luau").as_deref(), Some("server/a/b.luau"));
        assert_eq!(resolve_require("server/main.luau", "../x"), None);
        assert_eq!(resolve_require("main.luau", "./x").as_deref(), Some("x.luau"));
        assert_eq!(resolve_require("server/main.luau", "x"), None);
    }
}
