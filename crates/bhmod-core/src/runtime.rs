//! The runtime abstraction: [`Runtime`] loads a mod into an [`Instance`]; the instance exposes the WIT `guest`
//! export and calls back into the host through [`HostApi`] (the WIT imports).
//!
//! The core never knows which language a mod is written in. A WASM runtime (M2) implements [`Instance`] by calling
//! the component's `guest` export and implements the WIT imports by forwarding to [`HostApi`]; a JVM runtime (M9)
//! does the same through JNI. Deadlines are wall-clock instants: a runtime must stop guest code at
//! [`HostApi::deadline`] (Luau interrupt, wasmtime epoch) and report [`CallError::Timeout`].

use std::collections::BTreeMap;
use std::time::Instant;

use crate::types::{
    ApiError, CommandOutcome, CommandSender, CommandSpec, Event, EventKind, LogLevel, Outcome, Priority,
};
use crate::types::{Role, Side};

/// A subscription id (`events.subscription`).
pub type SubscriptionId = u32;
/// A command id (`commands.command-id`).
pub type CommandId = u32;
/// A task id (`scheduler.task-id`).
pub type TaskId = u64;

/// Why a guest call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The guest raised an error, trapped, ran out of memory or panicked. Counts as a strike.
    Trap(String),
    /// The guest ran past the hard deadline. Counts as a strike.
    Timeout,
    /// The guest returned an error on purpose (`on-enable` returning `err`). Not a strike.
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Trap(m) => write!(f, "trap: {m}"),
            CallError::Timeout => f.write_str("hard time budget exceeded"),
            CallError::Failed(m) => write!(f, "failed: {m}"),
        }
    }
}

/// The files of one mod side, read into memory at load time (`server/**` or `client/**`, and `data/**`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModFiles {
    /// The entry point path, for example `server/main.luau`.
    pub entry: String,
    /// Package path -> contents.
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Limits a runtime must enforce for one instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceLimits {
    /// Most bytes of guest memory (Luau heap, WASM linear memory).
    pub memory_bytes: usize,
}

/// A language runtime.
pub trait Runtime {
    /// The manifest runtime this implementation serves.
    fn kind(&self) -> bhmod_manifest::manifest::Runtime;

    /// Creates an instance (compile / validate the code). Must not run guest code yet: the first guest code runs in
    /// [`Instance::on_enable`], when a [`HostApi`] is available.
    fn instantiate(&self, mod_id: &str, files: &ModFiles, limits: InstanceLimits) -> Result<Box<dyn Instance>, String>;
}

/// One loaded mod: the WIT `guest` export. Every call gets the host API of the calling mod.
pub trait Instance {
    /// `on-enable` (runs the entry point first if the runtime needs that).
    fn on_enable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError>;
    /// `on-disable`.
    fn on_disable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError>;
    /// `handle-event`.
    fn handle_event(&mut self, host: &mut dyn HostApi, sub: SubscriptionId, ev: &Event) -> Result<Outcome, CallError>;
    /// `run-command`.
    fn run_command(
        &mut self,
        host: &mut dyn HostApi,
        id: CommandId,
        sender: &CommandSender,
        label: &str,
        args: &[String],
    ) -> Result<CommandOutcome, CallError>;
    /// `run-task`.
    fn run_task(&mut self, host: &mut dyn HostApi, id: TaskId) -> Result<(), CallError>;
}

/// The WIT imports, bound to the calling mod. Services and channels (M3) report [`ApiError::Unsupported`].
pub trait HostApi {
    /// When the current call must end (hard budget).
    fn deadline(&self) -> Instant;

    /// `host.api-version`.
    fn api_version(&self) -> String;
    /// `host.mod-id`.
    fn mod_id(&self) -> String;
    /// `host.mod-version`.
    fn mod_version(&self) -> String;
    /// `host.current-side`.
    fn current_side(&self) -> Side;
    /// `host.host-name`.
    fn host_name(&self) -> String;

    /// `log.log` (messages longer than 4096 bytes are truncated).
    fn log(&mut self, level: LogLevel, message: &str);

    /// `player.alias` (stale handle when offline).
    fn player_alias(&mut self, id: &str) -> Result<String, ApiError>;
    /// `player.is-online`.
    fn player_is_online(&mut self, id: &str) -> bool;
    /// `player.role` (stale handle when offline).
    fn player_role(&mut self, id: &str) -> Result<Role, ApiError>;
    /// `player.send-chat`.
    fn player_send_chat(&mut self, id: &str, text: &str) -> Result<(), ApiError>;
    /// `player.kick` (needs `players:kick`).
    fn player_kick(&mut self, id: &str, reason: &str) -> Result<(), ApiError>;
    /// `players.get`.
    fn players_get(&mut self, id: &str) -> Option<String>;
    /// `players.find-by-alias` (case-insensitive).
    fn players_find_by_alias(&mut self, alias: &str) -> Option<String>;
    /// `players.online`.
    fn players_online(&mut self) -> Vec<String>;
    /// `players.broadcast` (needs `players:chat`).
    fn players_broadcast(&mut self, text: &str) -> Result<(), ApiError>;

    /// `events.subscribe`.
    fn subscribe(
        &mut self,
        kind: EventKind,
        priority: Priority,
        ignore_cancelled: bool,
    ) -> Result<SubscriptionId, ApiError>;
    /// `events.unsubscribe`.
    fn unsubscribe(&mut self, sub: SubscriptionId) -> bool;

    /// `commands.register`.
    fn register_command(&mut self, spec: CommandSpec) -> Result<CommandId, ApiError>;
    /// `commands.unregister`.
    fn unregister_command(&mut self, id: CommandId) -> bool;

    /// `scheduler.run-later`.
    fn run_later(&mut self, delay: u32) -> Result<TaskId, ApiError>;
    /// `scheduler.run-every`.
    fn run_every(&mut self, delay: u32, period: u32) -> Result<TaskId, ApiError>;
    /// `scheduler.run-next-poll`.
    fn run_next_poll(&mut self) -> Result<TaskId, ApiError>;
    /// `scheduler.cancel`.
    fn cancel_task(&mut self, id: TaskId) -> bool;

    /// `storage.get`.
    fn storage_get(&mut self, key: &str) -> Option<Vec<u8>>;
    /// `storage.set`.
    fn storage_set(&mut self, key: &str, value: &[u8]) -> Result<(), ApiError>;
    /// `storage.delete`.
    fn storage_delete(&mut self, key: &str) -> bool;
    /// `storage.keys`.
    fn storage_keys(&mut self, prefix: &str) -> Vec<String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_error_display() {
        assert_eq!(CallError::Trap("boom".into()).to_string(), "trap: boom");
        assert_eq!(CallError::Timeout.to_string(), "hard time budget exceeded");
        assert_eq!(CallError::Failed("no".into()).to_string(), "failed: no");
    }
}
