//! The `blockheads:api` bindings (wasmtime `bindgen!`, see [`crate::wit`]), the imports implemented on top of
//! [`HostApi`], and the conversions between the generated WIT types and the core types. Public for other hosts of
//! WASM mods (the client loaders); most users only need [`crate::WasmRuntime`].
//!
//! Every import reaches the host through the [`HostApi`] of the guest call in progress. Outside a call (while the
//! component is instantiated) there is none and every import traps.

use wasmtime::component::Resource;

use bhmod_core::types as core;
use bhmod_core::HostApi;

use crate::state::State;

pub use crate::wit::blockheads::api::{
    channels, commands, events, host, log, players, scheduler, services, storage, types,
};
pub use crate::wit::{PlayerRes, ServerMod};

type R<T> = wasmtime::Result<T>;

impl State {
    /// The host API of the call in progress, or a trap outside a call (a start function calling an import).
    fn api(&mut self) -> R<&mut dyn HostApi> {
        Ok(self.host_api().ok_or(wasmtime::Trap::CannotEnterComponent)?)
    }

    /// Wraps a player id in a new resource owned by the guest.
    fn new_player(&mut self, id: String) -> R<Resource<PlayerRes>> {
        Ok(self.table.push(PlayerRes(id))?)
    }

    fn player_id(&self, p: &Resource<PlayerRes>) -> R<String> {
        Ok(self.table.get(p)?.0.clone())
    }
}

/// Core API error to WIT.
pub fn api_error(e: core::ApiError) -> types::ApiError {
    match e {
        core::ApiError::NotPermitted(p) => types::ApiError::NotPermitted(p),
        core::ApiError::InvalidArgument(a) => types::ApiError::InvalidArgument(a),
        core::ApiError::LimitExceeded(l) => types::ApiError::LimitExceeded(l),
        core::ApiError::StaleHandle => types::ApiError::StaleHandle,
        core::ApiError::Unsupported => types::ApiError::Unsupported,
        core::ApiError::Internal(m) => types::ApiError::Internal(m),
    }
}

/// Core side to WIT.
pub fn side_to(s: core::Side) -> types::Side {
    match s {
        core::Side::Server => types::Side::Server,
        core::Side::Client => types::Side::Client,
    }
}

/// Core role to WIT.
pub fn role_to(r: core::Role) -> players::Role {
    match r {
        core::Role::Player => players::Role::Player,
        core::Role::Moderator => players::Role::Moderator,
        core::Role::Admin => players::Role::Admin,
        core::Role::Owner => players::Role::Owner,
    }
}

/// WIT role to core.
pub fn role_from(r: players::Role) -> core::Role {
    match r {
        players::Role::Player => core::Role::Player,
        players::Role::Moderator => core::Role::Moderator,
        players::Role::Admin => core::Role::Admin,
        players::Role::Owner => core::Role::Owner,
    }
}

/// WIT priority to core.
pub fn priority_from(p: types::Priority) -> core::Priority {
    match p {
        types::Priority::Lowest => core::Priority::Lowest,
        types::Priority::Low => core::Priority::Low,
        types::Priority::Normal => core::Priority::Normal,
        types::Priority::High => core::Priority::High,
        types::Priority::Highest => core::Priority::Highest,
        types::Priority::Monitor => core::Priority::Monitor,
    }
}

/// WIT log level to core.
pub fn level_from(l: log::Level) -> core::LogLevel {
    match l {
        log::Level::Trace => core::LogLevel::Trace,
        log::Level::Debug => core::LogLevel::Debug,
        log::Level::Info => core::LogLevel::Info,
        log::Level::Warn => core::LogLevel::Warn,
        log::Level::Error => core::LogLevel::Error,
    }
}

/// WIT event kind to core.
pub fn kind_from(k: events::EventKind) -> core::EventKind {
    match k {
        events::EventKind::PlayerJoin => core::EventKind::PlayerJoin,
        events::EventKind::PlayerQuit => core::EventKind::PlayerQuit,
        events::EventKind::Chat => core::EventKind::Chat,
        events::EventKind::Command => core::EventKind::Command,
        events::EventKind::Tick => core::EventKind::Tick,
        events::EventKind::WorldLoaded => core::EventKind::WorldLoaded,
        events::EventKind::WorldSave => core::EventKind::WorldSave,
    }
}

/// WIT handler outcome to core.
pub fn outcome_from(o: events::Outcome) -> core::Outcome {
    core::Outcome {
        cancelled: o.cancelled,
        mutation: o.mutation.map(|m| match m {
            events::Mutation::ChatText(t) => core::Mutation::ChatText(t),
            events::Mutation::CustomPayload(p) => core::Mutation::CustomPayload(p),
        }),
    }
}

/// WIT command outcome to core.
pub fn command_outcome_from(o: commands::CommandOutcome) -> core::CommandOutcome {
    core::CommandOutcome { handled: o.handled, reply: o.reply }
}

impl types::Host for State {}

impl host::Host for State {
    fn api_version(&mut self) -> R<String> {
        Ok(self.api()?.api_version())
    }
    fn mod_id(&mut self) -> R<String> {
        Ok(self.api()?.mod_id())
    }
    fn mod_version(&mut self) -> R<String> {
        Ok(self.api()?.mod_version())
    }
    fn current_side(&mut self) -> R<types::Side> {
        Ok(side_to(self.api()?.current_side()))
    }
    fn host_name(&mut self) -> R<String> {
        Ok(self.api()?.host_name())
    }
}

impl log::Host for State {
    fn log(&mut self, level: log::Level, message: String) -> R<()> {
        self.api()?.log(level_from(level), &message);
        Ok(())
    }
}

impl players::HostPlayer for State {
    fn id(&mut self, p: Resource<PlayerRes>) -> R<String> {
        self.player_id(&p)
    }
    fn alias(&mut self, p: Resource<PlayerRes>) -> R<String> {
        let id = self.player_id(&p)?;
        Ok(self.api()?.player_alias(&id).unwrap_or_default())
    }
    fn is_online(&mut self, p: Resource<PlayerRes>) -> R<bool> {
        let id = self.player_id(&p)?;
        Ok(self.api()?.player_is_online(&id))
    }
    fn role(&mut self, p: Resource<PlayerRes>) -> R<players::Role> {
        let id = self.player_id(&p)?;
        Ok(role_to(self.api()?.player_role(&id).unwrap_or(core::Role::Player)))
    }
    fn send_chat(&mut self, p: Resource<PlayerRes>, text: String) -> R<Result<(), types::ApiError>> {
        let id = self.player_id(&p)?;
        Ok(self.api()?.player_send_chat(&id, &text).map_err(api_error))
    }
    fn kick(&mut self, p: Resource<PlayerRes>, reason: String) -> R<Result<(), types::ApiError>> {
        let id = self.player_id(&p)?;
        Ok(self.api()?.player_kick(&id, &reason).map_err(api_error))
    }
    fn drop(&mut self, p: Resource<PlayerRes>) -> R<()> {
        self.table.delete(p)?;
        Ok(())
    }
}

impl players::Host for State {
    fn get(&mut self, id: String) -> R<Option<Resource<PlayerRes>>> {
        let found = self.api()?.players_get(&id);
        found.map(|id| self.new_player(id)).transpose()
    }
    fn find_by_alias(&mut self, alias: String) -> R<Option<Resource<PlayerRes>>> {
        let found = self.api()?.players_find_by_alias(&alias);
        found.map(|id| self.new_player(id)).transpose()
    }
    fn online(&mut self) -> R<Vec<Resource<PlayerRes>>> {
        let ids = self.api()?.players_online();
        ids.into_iter().map(|id| self.new_player(id)).collect()
    }
    fn broadcast(&mut self, text: String) -> R<Result<(), types::ApiError>> {
        Ok(self.api()?.players_broadcast(&text).map_err(api_error))
    }
}

impl events::Host for State {
    fn subscribe(
        &mut self,
        kind: events::EventKind,
        priority: types::Priority,
        ignore_cancelled: bool,
    ) -> R<Result<u32, types::ApiError>> {
        Ok(self.api()?.subscribe(kind_from(kind), priority_from(priority), ignore_cancelled).map_err(api_error))
    }
    fn subscribe_custom(
        &mut self,
        _name: String,
        _priority: types::Priority,
        _ignore_cancelled: bool,
    ) -> R<Result<u32, types::ApiError>> {
        self.api()?;
        Ok(Err(types::ApiError::Unsupported))
    }
    fn unsubscribe(&mut self, sub: u32) -> R<bool> {
        Ok(self.api()?.unsubscribe(sub))
    }
    fn fire_custom(&mut self, _name: String, _payload: Vec<u8>) -> R<Result<bool, types::ApiError>> {
        self.api()?;
        Ok(Err(types::ApiError::Unsupported))
    }
}

impl commands::Host for State {
    fn register(&mut self, spec: commands::CommandSpec) -> R<Result<u32, types::ApiError>> {
        let spec = core::CommandSpec {
            name: spec.name,
            aliases: spec.aliases,
            description: spec.description,
            usage: spec.usage,
            min_role: role_from(spec.min_role),
            console: spec.console,
        };
        Ok(self.api()?.register_command(spec).map_err(api_error))
    }
    fn unregister(&mut self, id: u32) -> R<bool> {
        Ok(self.api()?.unregister_command(id))
    }
}

impl scheduler::Host for State {
    fn run_later(&mut self, delay: u32) -> R<Result<u64, types::ApiError>> {
        Ok(self.api()?.run_later(delay).map_err(api_error))
    }
    fn run_every(&mut self, delay: u32, period: u32) -> R<Result<u64, types::ApiError>> {
        Ok(self.api()?.run_every(delay, period).map_err(api_error))
    }
    fn run_next_poll(&mut self) -> R<Result<u64, types::ApiError>> {
        Ok(self.api()?.run_next_poll().map_err(api_error))
    }
    fn cancel(&mut self, id: u64) -> R<bool> {
        Ok(self.api()?.cancel_task(id))
    }
}

impl storage::Host for State {
    fn get(&mut self, key: String) -> R<Option<Vec<u8>>> {
        Ok(self.api()?.storage_get(&key))
    }
    fn set(&mut self, key: String, value: Vec<u8>) -> R<Result<(), types::ApiError>> {
        Ok(self.api()?.storage_set(&key, &value).map_err(api_error))
    }
    fn delete(&mut self, key: String) -> R<bool> {
        Ok(self.api()?.storage_delete(&key))
    }
    fn keys(&mut self, prefix: String) -> R<Vec<String>> {
        Ok(self.api()?.storage_keys(&prefix))
    }
}

// Services and channels arrive with M3; until then they report `unsupported` like the Luau runtime.
impl services::Host for State {
    fn provide(&mut self, _name: String, _version: String) -> R<Result<(), types::ApiError>> {
        self.api()?;
        Ok(Err(types::ApiError::Unsupported))
    }
    fn call(
        &mut self,
        _name: String,
        _version_req: String,
        _method: String,
        _args: Vec<u8>,
    ) -> R<Result<Vec<u8>, services::CallError>> {
        self.api()?;
        Ok(Err(services::CallError::NotFound))
    }
}

impl channels::Host for State {
    fn open(&mut self, _name: String) -> R<Result<u32, types::ApiError>> {
        self.api()?;
        Ok(Err(types::ApiError::Unsupported))
    }
    fn send(&mut self, _channel: u32, _to: channels::Target, _data: Vec<u8>) -> R<Result<(), types::ApiError>> {
        self.api()?;
        Ok(Err(types::ApiError::Unsupported))
    }
    fn close(&mut self, _channel: u32) -> R<bool> {
        self.api()?;
        Ok(false)
    }
}
