// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

/** An event handler. {@code T} is the event record of the subscription (for example {@code ChatEvent}). */
public interface Handler<T> {
    /** Handles the event; change its fields (for example {@code cancelled}) to influence the outcome. */
    void handle(T event) throws Exception;
}
