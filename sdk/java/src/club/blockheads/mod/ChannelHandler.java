// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.channels.Source;

/** Receives the messages of a channel. */
public interface ChannelHandler {
    void onMessage(Source origin, byte[] data) throws Exception;
}
