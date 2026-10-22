//! The Luau runtime (mlua, vendored Luau): one VM per mod in sandbox mode, a memory limit and an interrupt that
//! stops the VM at the call's deadline. Implements the mapping in `docs/luau-api.md`.
//!
//! The `bh` functions reach the host through a pointer to the [`HostApi`] of the call in progress. It is set when
//! the host enters the VM and cleared before the call returns, so it never outlives the borrow it came from.
#![allow(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use mlua::{AnyUserData, Function, Lua, MetaMethod, Table, UserData, UserDataMethods, Value, Variadic, VmState};

use crate::runtime::TaskId;
use crate::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, ModFiles, Runtime, SubscriptionId};
use crate::types::{ApiError, CommandOutcome, CommandSender, CommandSpec, Event, EventKind, LogLevel, Mutation};
use crate::types::{Outcome, Priority, Role};

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
        Err(e) => Err(CallError::Trap(error_text(&e))),
    }
}

fn error_text(e: &mlua::Error) -> String {
    match e {
        mlua::Error::MemoryError(_) => "out of memory".to_string(),
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
    }
    Ok(t)
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
        for reg in [HANDLERS, COMMANDS, TASKS, LOADED] {
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
        b.func(&players, "online", move |lua, ()| {
            let ids = with_host(&s, |h| h.players_online())?;
            ids.into_iter().map(|id| player(lua, &s, id)).collect::<mlua::Result<Vec<_>>>()
        });
        let s = sh.clone();
        b.func(&players, "broadcast", move |_, text: String| {
            with_host(&s, |h| h.players_broadcast(&text))?.map_err(api_err)
        });
        bh.set("players", players)?;

        let events = lua.create_table()?;
        let s = sh.clone();
        b.func(&events, "on", move |lua, (kind, f, opts): (String, Function, Option<Table>)| {
            let kind =
                EventKind::parse(&kind).ok_or_else(|| rt_err(format!("invalid argument: unknown event {kind:?}")))?;
            let mut prio = Priority::Normal;
            let mut ignore = false;
            if let Some(o) = opts {
                if let Some(p) = o.get::<Option<String>>("priority")? {
                    prio = Priority::parse(&p).ok_or_else(|| rt_err(format!("invalid argument: priority {p:?}")))?;
                }
                ignore = o.get::<Option<bool>>("ignore_cancelled")?.unwrap_or(false);
            }
            let id = with_host(&s, |h| h.subscribe(kind, prio, ignore))?.map_err(api_err)?;
            lua.named_registry_value::<Table>(HANDLERS)?.set(id, f)?;
            Ok(id)
        });
        let s = sh.clone();
        b.func(&events, "off", move |lua, id: SubscriptionId| {
            lua.named_registry_value::<Table>(HANDLERS)?.set(id, Value::Nil)?;
            with_host(&s, |h| h.unsubscribe(id))
        });
        let unsupported =
            lua.create_function(|_, _: Variadic<Value>| -> mlua::Result<()> { Err(api_err(ApiError::Unsupported)) })?;
        events.set("on_custom", unsupported.clone())?;
        events.set("fire_custom", unsupported.clone())?;
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

        for name in ["services", "channels"] {
            let t = lua.create_table()?;
            for f in ["provide", "call", "open", "send"] {
                t.set(f, unsupported.clone())?;
            }
            bh.set(name, t)?;
        }
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
            let mutation = match ev {
                Event::Chat { text, .. } => match t.get::<Value>("text")? {
                    Value::String(s) if s.to_string_lossy() != *text => Some(Mutation::ChatText(s.to_string_lossy())),
                    _ => None,
                },
                _ => None,
            };
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
