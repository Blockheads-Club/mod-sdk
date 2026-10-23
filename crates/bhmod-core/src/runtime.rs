//! The runtime abstraction: [`Runtime`] loads a mod into an [`Instance`]; the instance exposes the WIT `guest`
//! export and calls back into the host through [`HostApi`] (the WIT imports).
//!
//! The core never knows which language a mod is written in. A WASM runtime (M2) implements [`Instance`] by calling
//! the component's `guest` export and implements the WIT imports by forwarding to [`HostApi`]; a JVM runtime (M9)
//! does the same through JNI. Deadlines are wall-clock instants: a runtime must stop guest code at
//! [`HostApi::deadline`] (Luau interrupt, wasmtime epoch) and report [`CallError::Timeout`].

use std::collections::BTreeMap;
use std::time::Instant;

use crate::channels::ChannelId;
use crate::remote::{RemoteCaller, RemoteTarget};
use crate::types::{
    ApiError, ChannelSource, ChannelTarget, CommandOutcome, CommandSender, CommandSpec, ContentKind, CosmeticInfo,
    Event, EventKind, LogLevel, Outcome, Pos, Priority, ServiceError,
};
use crate::types::{Role, Side};
use crate::ui::{Dialog, NoticeLevel, Widget};

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

    /// `call-service` (API 0.2): `args` and the result are canonical CBOR. A deliberate error from the provider is
    /// [`CallError::Failed`] (reported as `provider-error`, not a strike). The default reports that the runtime does
    /// not implement services.
    fn call_service(
        &mut self,
        host: &mut dyn HostApi,
        name: &str,
        method: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, CallError> {
        let _ = (host, name, method, args);
        Err(CallError::Failed("this runtime does not implement services".into()))
    }

    /// `on-channel-message` (API 0.2). The default ignores the message.
    fn on_channel_message(
        &mut self,
        host: &mut dyn HostApi,
        channel: ChannelId,
        source: &ChannelSource,
        data: &[u8],
    ) -> Result<(), CallError> {
        let _ = (host, channel, source, data);
        Ok(())
    }
}

/// The WIT imports, bound to the calling mod. The API 0.2 and 0.3 additions have defaults that report
/// [`ApiError::Unsupported`], so a host API written for 0.1 keeps compiling; [`crate::ModHost`] implements them all.
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

    /// `player.pos` (API 0.2): the tile of the player's blockhead, `None` when it is not in the world.
    fn player_pos(&mut self, id: &str) -> Result<Option<Pos>, ApiError> {
        let _ = id;
        Err(ApiError::Unsupported)
    }
    /// The host handle of a player, for `{"$h": n}` values.
    fn handle_of_player(&mut self, id: &str) -> u64 {
        let _ = id;
        0
    }
    /// The player behind a host handle.
    fn player_of_handle(&mut self, handle: u64) -> Result<String, ApiError> {
        let _ = handle;
        Err(ApiError::StaleHandle)
    }

    /// `events.subscribe-custom`.
    fn subscribe_custom(
        &mut self,
        name: &str,
        priority: Priority,
        ignore_cancelled: bool,
    ) -> Result<SubscriptionId, ApiError> {
        let _ = (name, priority, ignore_cancelled);
        Err(ApiError::Unsupported)
    }
    /// `events.fire-custom`: true when the event ended up cancelled.
    fn fire_custom(&mut self, name: &str, payload: &[u8]) -> Result<bool, ApiError> {
        let _ = (name, payload);
        Err(ApiError::Unsupported)
    }

    /// `services.provide`.
    fn provide_service(&mut self, name: &str, version: &str) -> Result<(), ApiError> {
        let _ = (name, version);
        Err(ApiError::Unsupported)
    }
    /// `services.call`.
    fn call_service(
        &mut self,
        name: &str,
        version_req: &str,
        method: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, ServiceError> {
        let _ = (name, version_req, method, args);
        Err(ServiceError::NotFound)
    }

    /// `services.call-remote` (API 0.3.1): the id of the call. The answer arrives as the custom event
    /// `<mod-id>:service-result`; see [`crate::remote`].
    fn call_remote(
        &mut self,
        to: RemoteTarget,
        name: &str,
        version_req: &str,
        method: &str,
        args: &[u8],
        timeout_ms: u32,
    ) -> Result<u32, ApiError> {
        let _ = (to, name, version_req, method, args, timeout_ms);
        Err(ApiError::Unsupported)
    }
    /// `services.cancel-remote` (API 0.3.1).
    fn cancel_remote(&mut self, id: u32) -> bool {
        let _ = id;
        false
    }
    /// `services.caller` (API 0.3.1).
    fn remote_caller(&mut self) -> Option<RemoteCaller> {
        None
    }

    /// `content.id-of` (API 0.3).
    fn content_id(&mut self, kind: ContentKind, key: &str) -> Option<u64> {
        let _ = (kind, key);
        None
    }
    /// `content.key-of` (API 0.3).
    fn content_key(&mut self, kind: ContentKind, id: u64) -> Option<String> {
        let _ = (kind, id);
        None
    }
    /// `content.cosmetics` (API 0.3).
    fn content_cosmetics(&mut self) -> Vec<CosmeticInfo> {
        Vec::new()
    }
    /// `content.equipped` (API 0.3): `(slot, key)` pairs.
    fn content_equipped(&mut self, id: &str) -> Result<Vec<(String, String)>, ApiError> {
        let _ = id;
        Err(ApiError::Unsupported)
    }
    /// `content.equip` (API 0.3).
    fn content_equip(&mut self, id: &str, key: &str) -> Result<(), ApiError> {
        let _ = (id, key);
        Err(ApiError::Unsupported)
    }
    /// `content.unequip` (API 0.3).
    fn content_unequip(&mut self, id: &str, slot: &str) -> Result<(), ApiError> {
        let _ = (id, slot);
        Err(ApiError::Unsupported)
    }

    /// `channels.open`.
    fn channel_open(&mut self, name: &str) -> Result<ChannelId, ApiError> {
        let _ = name;
        Err(ApiError::Unsupported)
    }
    /// `channels.send`.
    fn channel_send(&mut self, channel: ChannelId, to: ChannelTarget, data: &[u8]) -> Result<(), ApiError> {
        let _ = (channel, to, data);
        Err(ApiError::Unsupported)
    }
    /// `channels.close`.
    fn channel_close(&mut self, channel: ChannelId) -> bool {
        let _ = channel;
        false
    }

    /// `ui.set-widget` (API 0.3).
    fn ui_set_widget(&mut self, id: &str, widget: Widget) -> Result<(), ApiError> {
        let _ = (id, widget);
        Err(ApiError::Unsupported)
    }
    /// `ui.remove-widget` (API 0.3).
    fn ui_remove_widget(&mut self, id: &str) -> bool {
        let _ = id;
        false
    }
    /// `ui.toast` (API 0.3).
    fn ui_toast(&mut self, text: &str) -> Result<(), ApiError> {
        let _ = text;
        Err(ApiError::Unsupported)
    }
    /// `ui.notice` (API 0.3).
    fn ui_notice(&mut self, text: &str, level: NoticeLevel) -> Result<(), ApiError> {
        let _ = (text, level);
        Err(ApiError::Unsupported)
    }
    /// `ui.show-dialog` (API 0.3): the dialog id.
    fn ui_show_dialog(&mut self, dialog: Dialog) -> Result<u32, ApiError> {
        let _ = dialog;
        Err(ApiError::Unsupported)
    }
    /// `ui.close-dialog` (API 0.3).
    fn ui_close_dialog(&mut self, id: u32) -> bool {
        let _ = id;
        false
    }
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
