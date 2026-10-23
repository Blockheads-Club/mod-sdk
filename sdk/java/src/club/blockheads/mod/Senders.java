// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.events.CommandSender;
import club.blockheads.api.players.Player;

/** Helpers for {@link CommandSender}. */
public final class Senders {
    private Senders() {
    }

    /** The player who ran the command, or {@code null} for the console. */
    public static Player player(CommandSender sender) {
        return sender instanceof CommandSender.Player ? ((CommandSender.Player) sender).value : null;
    }

    /** The player's alias, or {@code "console"}. */
    public static String name(CommandSender sender) {
        Player p = player(sender);
        return p == null ? "console" : p.alias();
    }
}
