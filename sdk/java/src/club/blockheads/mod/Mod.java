// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

/**
 * The entry point of a JVM mod: a public class with a public no-argument constructor, named in the jar manifest as
 * {@code Bhmod-Main} (or {@code Main-Class}). The host creates one instance per mod and calls these on the game
 * thread, one call at a time.
 */
public interface Mod {
    /**
     * Called once when the mod is enabled. Subscribe to events and register commands here. An exception disables
     * the mod and is logged.
     */
    void onEnable(Api api) throws Exception;

    /** Called once when the mod is disabled (server stop, reload). Flush state here. */
    default void onDisable() throws Exception {
    }
}
