// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

/** The answer of a remote service call ({@link Api#callRemote}); exactly one of the two methods runs, later. */
public interface RemoteCallback {
    /** The provider returned {@code value} (a plain CBOR value). */
    void ok(Object value) throws Exception;

    /**
     * The call failed. {@code kind} is {@code not-found}, {@code version-mismatch}, {@code not-permitted},
     * {@code busy}, {@code provider-error}, {@code trap}, {@code bad-value}, {@code too-large}, {@code internal}
     * (the other side's answer) or {@code unavailable}, {@code timeout}, {@code disconnected} (decided locally).
     */
    void failed(String kind, String message) throws Exception;
}
