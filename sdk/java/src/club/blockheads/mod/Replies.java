// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.commands.CommandOutcome;

import java.util.ArrayList;
import java.util.Arrays;

/** Shortcuts for command outcomes. */
public final class Replies {
    private Replies() {
    }

    /** Handled, with these reply lines. */
    public static CommandOutcome reply(String... lines) {
        return new CommandOutcome(true, new ArrayList<String>(Arrays.asList(lines)));
    }

    /** Not handled: the host shows the command's usage text to the sender, after these lines (if any). */
    public static CommandOutcome usage(String... lines) {
        return new CommandOutcome(false, new ArrayList<String>(Arrays.asList(lines)));
    }
}
