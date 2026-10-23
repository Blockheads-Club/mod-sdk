// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

/** A deliberate error of a service method: the caller sees {@code provider-error} with this message. */
public class ServiceError extends Exception {
    private static final long serialVersionUID = 1L;

    public ServiceError(String message) {
        super(message);
    }
}
