// Blockheads Club mod loader: native methods of libbhmodloader.so. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

/** Calls into libbhmodloader.so. The game loads the library after our listener starts, so every call is guarded. */
final class Native {
    private Native() {}

    static native void decide(boolean accept);
    static native void cancel();
    /** A mod dialog was answered: `button` from 0 (-1 when it went away), `text` null without a field. */
    static native void dialogAnswer(int dialog, int button, String text);
    static native String version();
    static native byte[] font(int id);
    static native void setModsLabel(String label);
    static native String listInstalled();
    static native String inspect(String path, String sha256);
    /** `nativeConfirmed`: the player confirmed the red native-code warning (honoured for index installs only). */
    static native String install(String path, String sha256, String source, boolean nativeConfirmed);
    static native boolean setEnabled(String id, boolean on);
    static native boolean remove(String id);
    /** The answer of the RECORD_AUDIO dialog (voice chat). */
    static native void micPermission(boolean granted);
    /** Push-to-talk down / up (seam for the voice HUD button). */
    static native void ptt(boolean down);

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
