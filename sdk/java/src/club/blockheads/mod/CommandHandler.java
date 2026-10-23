// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.commands.CommandOutcome;
import club.blockheads.api.events.CommandSender;

import java.util.List;

/** Runs a registered command. */
public interface CommandHandler {
    /**
     * @param sender who ran it
     * @param label the name or alias that was typed
     * @param args the rest of the line split on spaces
     * @return the outcome; {@code null} means handled without a reply (see {@link Replies})
     */
    CommandOutcome run(CommandSender sender, String label, List<String> args) throws Exception;
}
