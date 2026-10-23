// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.commands.CommandSpec;
import club.blockheads.api.players.Role;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** Builds a {@link CommandSpec}: {@code Spec.command("home").alias("h").usage("/home [name]").build()}. */
public final class Spec {
    private final String name;
    private final List<String> aliases = new ArrayList<String>();
    private String description = "";
    private String usage;
    private Role minRole = Role.PLAYER;
    private boolean console = true;

    private Spec(String name) {
        this.name = name;
        this.usage = "/" + name;
    }

    /** A command named {@code name} (without the slash). */
    public static Spec command(String name) {
        return new Spec(name);
    }

    public Spec alias(String... names) {
        aliases.addAll(Arrays.asList(names));
        return this;
    }

    public Spec description(String text) {
        this.description = text;
        return this;
    }

    public Spec usage(String text) {
        this.usage = text;
        return this;
    }

    /** The lowest role allowed to run it from chat (default {@link Role#PLAYER}). */
    public Spec minRole(Role role) {
        this.minRole = role;
        return this;
    }

    /** Whether the console may run it (default true). */
    public Spec console(boolean allowed) {
        this.console = allowed;
        return this;
    }

    public CommandSpec build() {
        return new CommandSpec(name, new ArrayList<String>(aliases), description, usage, minRole, console);
    }
}
