// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod.rt;

/** A host function failed with an error that has no typed exception, or the host itself failed. */
public class HostException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    public HostException(String message) {
        super(message);
    }
}
