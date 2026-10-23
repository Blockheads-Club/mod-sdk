// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import java.util.List;

/**
 * A service provided to other mods (see {@link Api#provide}). Arguments and the result are plain values: null,
 * booleans, numbers ({@code Long} for integers, {@code Double}), text, {@code byte[]}, lists, text-keyed maps and
 * {@link club.blockheads.api.players.Player}s. Throw a {@link ServiceError} to answer with a provider error.
 */
public interface Service {
    Object call(String method, List<Object> args) throws Exception;
}
