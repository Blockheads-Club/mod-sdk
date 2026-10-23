//! [`GameBridge`]: what a host program (server, PC client, Android loader) provides to the mod host.
//!
//! The bridge is the only way mod code reaches the game. Every method is called on the game's main thread while a
//! mod call is running, so implementations must not call back into the [`crate::ModHost`].

use crate::types::{ApiError, ChannelTarget, ContentKind, LogLevel, Pos, Role, Side};

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

    /// The numeric id this world gave a content key (API 0.3). `None` for an unknown key and on hosts that keep no
    /// registry (the clients).
    fn content_id(&self, kind: ContentKind, key: &str) -> Option<u64> {
        let _ = (kind, key);
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
}
