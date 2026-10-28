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
    /**
     * True when the loader can keep an online world through the RECORD_AUDIO dialog (its lifecycle guard is hooked
     * into the game); without it the dialog is never shown while connected.
     */
    static native boolean micGuardAvailable();
    /** Arms (true) or drops the lifecycle guard around our RECORD_AUDIO dialog. */
    static native void micGuard(boolean on);
    /** The player touched a loader window: the game's idle pause restarts at the next frame, as for a touch on the game. */
    static native void touched();
    /** Push-to-talk down / up (host seam: talks in the default mode without going through a mod). */
    static native void ptt(boolean down);
    /** An on-screen button of a mod: `action` = press, long-press or release. */
    static native void uiButton(String mod, String id, String action);
    /** The player dropped a button at screen fractions `x`, `y` (stored on the device). */
    static native void buttonMoved(String mod, String id, float x, float y);
    /** A long press on the game at screen fractions `x`, `y` (the blockhead there is picked). */
    static native void tap(float x, float y);
    /** A hardware key went down or up (key name of the `input` interface). */
    static native void key(String name, boolean down);
    /** Opens the settings screen of a running mod. */
    static native void openSettings(String mod);
    /** A setting changed in the settings screen: `scope` client, server, keys or voice; `json` the new value. */
    static native void settingSet(String mod, String scope, String key, String json);

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
