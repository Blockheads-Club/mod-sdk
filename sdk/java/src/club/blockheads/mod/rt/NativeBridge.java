// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod.rt;

import java.util.Arrays;
import java.util.List;

/** The {@link Bridge} of one mod instance: CBOR over the native {@code Glue.host} function. */
final class NativeBridge implements Bridge {
    private final int inst;

    NativeBridge(int inst) {
        this.inst = inst;
    }

    @Override
    public Object call(String fn, Object... args) {
        List<Object> list = Arrays.asList(args);
        return Cbor.decode(Glue.host(inst, fn, Cbor.encode(list)));
    }
}
