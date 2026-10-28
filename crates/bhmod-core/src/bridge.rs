//! [`GameBridge`]: what a host program (server, PC client, Android loader) provides to the mod host.
//!
//! The bridge is the only way mod code reaches the game. Every method is called on the game's main thread while a
//! mod call is running, so implementations must not call back into the [`crate::ModHost`].

use crate::input::InputCommand;
use crate::remote::{RemoteCall, RemoteError};
use crate::types::{ApiError, ChannelTarget, ContentKind, LogLevel, Pos, Role, Side};
use crate::ui::UiCommand;
use crate::voice::VoiceHost;

/// The game side of the mod host. Players are identified by their player id string (the vanilla `playerID`).
pub trait GameBridge {
    /// Which side this host is.
    fn side(&self) -> Side;

    /// A short host name and version for logs (`blockheads-server 0.1.1`).
    fn host_name(&self) -> String;

    /// Writes one log line. `mod_id` is `None` for messages of the mod host itself.
    fn log(&self, mod_id: Option<&str>, level: LogLevel, message: &str);

    /// Connected (approved) players in join order.
    fn online_players(&self) -> Vec<String>;

    /// The alias of a connected player, `None` when the player is not connected.
    fn player_alias(&self, id: &str) -> Option<String>;

    /// The role of a player.
    fn player_role(&self, id: &str) -> Role;

    /// Sends a chat line to one player. False when the player is not connected.
    fn send_chat(&self, id: &str, text: &str) -> bool;

    /// Sends a chat line to every player as a server message.
    fn broadcast(&self, text: &str);

    /// Disconnects a player. False when the player is not connected.
    fn kick(&self, id: &str, reason: &str) -> bool;

    /// True when `name` is a built-in command that mods may not register.
    fn is_builtin_command(&self, name: &str) -> bool;

    /// The tile of a connected player's blockhead (API 0.2). `None` when the player has no blockhead in the world.
    fn player_pos(&self, id: &str) -> Option<Pos> {
        let _ = id;
        None
    }

    /// Sends a channel message over the network (`target` is never [`ChannelTarget::Local`]). The 0xBC `CHANNEL`
    /// packets are sent by hosts that implement the 0xBC protocol (the server since milestone M5); the default reports
    /// [`ApiError::Unsupported`].
    fn channel_send(&self, mod_id: &str, name: &str, target: &ChannelTarget, data: &[u8]) -> Result<(), ApiError> {
        let _ = (mod_id, name, target, data);
        Err(ApiError::Unsupported)
    }

    /// Sends a remote service call to the other side (API 0.3.1, 0xBC `SERVICE_CALL`). The mod host has already
    /// validated it and keeps it open until the answer arrives ([`crate::ModHost::remote_result`]) or it times out.
    /// An error is delivered to the calling mod at once (`unavailable` when there is nobody to ask); the default,
    /// for hosts without the 0xBC protocol, is [`RemoteError::Unavailable`].
    fn service_send(&self, call: &RemoteCall) -> Result<(), RemoteError> {
        let _ = call;
        Err(RemoteError::Unavailable)
    }

    /// The numeric id this world gave a content key (API 0.3). `None` for an unknown key and on hosts that keep no
    /// registry (the clients).
    fn content_id(&self, kind: ContentKind, key: &str) -> Option<u64> {
        let _ = (kind, key);
        None
    }

    /// The key of a content id (API 0.3); the reverse of [`GameBridge::content_id`].
    fn content_key(&self, kind: ContentKind, id: u64) -> Option<String> {
        let _ = (kind, id);
        None
    }

    /// Puts the cosmetic `id` on a player in `slot` (API 0.3); `None` takes the slot's cosmetic off. The host
    /// tells every modded client. The mod host has already checked that the cosmetic belongs to the calling mod and
    /// fits the slot. The default reports [`ApiError::Unsupported`].
    fn set_cosmetic(&self, player: &str, slot: &str, id: Option<u64>) -> Result<(), ApiError> {
        let _ = (player, slot, id);
        Err(ApiError::Unsupported)
    }

    /// What a player wears as `(slot, cosmetic id)` pairs (API 0.3).
    fn cosmetics_of(&self, player: &str) -> Vec<(String, u64)> {
        let _ = player;
        Vec::new()
    }
    /// Gives a player `count` of item `item` (an `ItemType` of the registry's `items`, API 0.3.2). The mod host has
    /// already checked that the item belongs to the calling mod. The default reports [`ApiError::Unsupported`].
    fn give_item(&self, player: &str, item: u64, count: u32) -> Result<(), ApiError> {
        let _ = (player, item, count);
        Err(ApiError::Unsupported)
    }

    /// Replaces the block at `(x, y)` with variant block `block` (registry `blocks`, API 0.3.2). The mod host has
    /// already checked that the block belongs to the calling mod and has a base. The default reports
    /// [`ApiError::Unsupported`].
    fn place_block(&self, x: i32, y: i32, block: u64) -> Result<(), ApiError> {
        let _ = (x, y, block);
        Err(ApiError::Unsupported)
    }

    /// The variant block id at `(x, y)` (API 0.3.2); `None` for a plain block or an unloaded tile.
    fn block_at(&self, x: i32, y: i32) -> Option<u64> {
        let _ = (x, y);
        None
    }

    /// Draws mod UI (API 0.3 `ui`; clients only). The command is already validated and limited
    /// ([`crate::ui`]); the default, for hosts without a UI such as the server, reports [`ApiError::Unsupported`].
    fn ui(&self, mod_id: &str, command: &UiCommand) -> Result<(), ApiError> {
        let _ = (mod_id, command);
        Err(ApiError::Unsupported)
    }

    /// The voice side of the host (API 0.3.4 `voice`); `None` (the default) = no voice: every `voice` function
    /// reports `unsupported`.
    fn voice(&self) -> Option<&dyn VoiceHost> {
        None
    }

    /// Whether tile `(x, y)` is solid, for `terrain.solid-count` (API 0.3.4); `None` for a tile that is not loaded
    /// (counted as not solid) and on hosts without a world (the default).
    fn tile_solid(&self, x: i32, y: i32) -> Option<bool> {
        let _ = (x, y);
        None
    }

    /// The world width in tiles (`x` wraps around it), when known (API 0.3.4).
    fn world_width(&self) -> Option<i32> {
        None
    }

    /// Key bindings of client mods (API 0.3.4 `input`). The command is already validated and limited
    /// ([`crate::input`]); the default, for hosts without a keyboard path such as the server, reports
    /// [`ApiError::Unsupported`].
    fn input(&self, mod_id: &str, command: &InputCommand) -> Result<(), ApiError> {
        let _ = (mod_id, command);
        Err(ApiError::Unsupported)
    }

    /// The key an action of `mod_id` is bound to now (API 0.3.4 `input.key-of`): the player's choice, or the
    /// declared default (`default_key`, which the default implementation returns). `None` = the player cleared it.
    fn input_key(&self, mod_id: &str, action: &str, default_key: &str) -> Option<String> {
        let _ = (mod_id, action);
        Some(default_key.to_string())
    }

    /// The platform (API 0.3.4 `host.platform`): `<host>-<os>-<arch>` such as `android-armv7` or
    /// `server-linux-x86_64`. The default is `unknown`.
    fn platform(&self) -> String {
        "unknown".into()
    }

    /// True while a player's blockhead sleeps (API 0.3.4). The default (hosts that do not track it) is false.
    fn player_sleeping(&self, id: &str) -> bool {
        let _ = id;
        false
    }

    /// True while a player's blockhead is alive (API 0.3.4). The default (hosts that do not track it) is true, so a
    /// host without death tracking never silences proximity voice.
    fn player_alive(&self, id: &str) -> bool {
        let _ = id;
        true
    }

    /// The player of this client (API 0.3.4 `players.local-player`); `None` on the server (the default).
    fn local_player(&self) -> Option<String> {
        None
    }

    /// True when player `id`'s client runs mod `mod_id` (API 0.3.4 `players.has-mod`, servers). The default is false.
    fn player_has_mod(&self, id: &str, mod_id: &str) -> bool {
        let _ = (id, mod_id);
        false
    }

    /// True when the device has a touch screen (API 0.3.4 `ui.has-touch`). The default is false.
    fn has_touch(&self) -> bool {
        false
    }
}
