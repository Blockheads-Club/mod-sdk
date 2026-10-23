// Blockheads Club mod loader: native methods of libbhmodloader.so. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

/** Calls into libbhmodloader.so. The game loads the library after our listener starts, so every call is guarded. */
final class Native {
    private Native() {}

    static native void decide(boolean accept);
    static native void cancel();
    static native String version();
    static native byte[] font(int id);
    static native void setModsLabel(String label);
    static native String listInstalled();
    static native String inspect(String path, String sha256);
    static native String install(String path, String sha256, String source);
    static native boolean setEnabled(String id, boolean on);
    static native boolean remove(String id);

    /** Runs a native call; false when the library is not loaded (yet). */
    static boolean ok(Runnable r) {
        try {
            r.run();
            return true;
        } catch (UnsatisfiedLinkError e) {
            return false;
        }
    }
}
