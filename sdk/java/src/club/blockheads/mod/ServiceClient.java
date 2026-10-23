// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.services.CallErrorException;
import club.blockheads.api.services.Services;

import java.util.Arrays;

/** Calls the methods of a service provided by another mod. */
public final class ServiceClient {
    private final Services services;
    private final String name;
    private final String requirement;

    ServiceClient(Services services, String name, String requirement) {
        this.services = services;
        this.name = name;
        this.requirement = requirement;
    }

    /**
     * Calls {@code method}.
     *
     * @throws CallErrorException when the call fails (not found, reentrant, provider error, ...)
     */
    public Object call(String method, Object... args) {
        return services.call(name, requirement, method, Arrays.asList(args));
    }
}
