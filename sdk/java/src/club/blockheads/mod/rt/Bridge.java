// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod.rt;

/**
 * The connection to the host: calls a WIT import by its name ({@code interface.function}, resource methods
 * {@code interface.resource.method}) with the arguments as a CBOR list and returns the decoded result.
 */
public interface Bridge {
    /**
     * Calls the host function {@code fn}. A function that returns a WIT {@code result} gives
     * {@code {"ok": v}} or {@code {"err": e}}; others give the plain value.
     */
    Object call(String fn, Object... args);
}
