//! [`GameBridge`]: what a host program (server, PC client, Android loader) provides to the mod host.
//!
//! The bridge is the only way mod code reaches the game. Every method is called on the game's main thread while a
//! mod call is running, so implementations must not call back into the [`crate::ModHost`].

use crate::types::{LogLevel, Role, Side};

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
}
