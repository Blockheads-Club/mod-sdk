// Blockheads Club mod loader: Java side of the bridge. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import java.io.File;
import java.lang.ref.WeakReference;

/**
 * Static entry points for the native library: {@link #dataDir()} at load time and {@link #post(String)} for UI
 * commands (JSON). Everything UI runs on the Android main (UI) thread; the game thread never waits for it.
 */
public final class Bridge {
    static final String TAG = "BHModLoader";
    static Context app;
    static WeakReference<Activity> activity = new WeakReference<>(null);
    static final Handler ui = new Handler(Looper.getMainLooper());
    static volatile String loaderVersion = null;

    private Bridge() {}

    static synchronized void init(Context c) {
        if (app == null) app = c.getApplicationContext();
    }

    static void activity(final Activity a) {
        activity = new WeakReference<>(a);
        ui.post(new Runnable() { public void run() { Overlay.attach(a); } });
    }

    static void activityGone(final Activity a) {
        if (activity.get() == a) activity = new WeakReference<>(null);
        ui.post(new Runnable() {
            public void run() {
                if (Overlay.attachedTo == a) {
                    Overlay.detach();
                    Overlay.attachedTo = null;
                }
            }
        });
    }

    /** Where the loader keeps its files (called from JNI_OnLoad). */
    public static String dataDir() {
        File d = app != null ? new File(app.getFilesDir(), "bhmods") : new File("/data/local/tmp/bhmods");
        d.mkdirs();
        return d.getAbsolutePath();
    }

    /** Voice: asks for RECORD_AUDIO (first opt-in); the answer comes back through Native.micPermission. */
    public static void requestMic() {
        Voice.request();
    }

    /** Voice: true when RECORD_AUDIO is granted already (no dialog needed). */
    public static boolean micGranted() {
        return Voice.granted();
    }

    /** A UI command from the game thread; handled later on the UI thread. */
    public static void post(final String json) {
        ui.post(new Runnable() {
            public void run() {
                try {
                    Overlay.command(new org.json.JSONObject(json));
                } catch (Throwable t) {
                    Log.w(TAG, "UI command failed: " + json, t);
                }
            }
        });
    }
}
