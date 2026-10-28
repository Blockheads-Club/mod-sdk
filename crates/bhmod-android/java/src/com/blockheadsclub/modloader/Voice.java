// Blockheads Club mod loader: microphone permission and push-to-talk seam for voice chat.
// Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.Manifest;
import android.app.Activity;
import android.app.Fragment;
import android.app.FragmentManager;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Process;
import android.util.Log;

/**
 * The {@code RECORD_AUDIO} runtime permission (the game targets SDK 27, so Android 6+ asks at run time; Blockheads
 * Patcher's "record-audio" patch declares it in the manifest). The loader asks only when the player first turns voice
 * on ({@link #request()} from {@code Bridge.requestMic}); the answer goes to {@code Native.micPermission}. The game's
 * activity is not ours, so a headless {@link Fragment} receives {@code onRequestPermissionsResult}.
 *
 * <p>{@link #ptt(boolean)} is the seam for the push-to-talk HUD button (a later milestone): touch down / up.
 */
final class Voice {
    static final String PERMISSION = Manifest.permission.RECORD_AUDIO;
    static final int REQUEST = 0xB0C;
    static final String TAG = "bhmod-voice";

    private Voice() {}

    /** True when the app holds the permission (always true before Android 6 when declared). */
    static boolean granted() {
        if (Bridge.app == null) return false;
        return Bridge.app.checkPermission(PERMISSION, Process.myPid(), Process.myUid()) == PackageManager.PERMISSION_GRANTED;
    }

    /** Shows the system dialog (UI thread); answers at once when no dialog is needed or possible. */
    static void request() {
        Bridge.ui.post(new Runnable() {
            public void run() {
                try {
                    if (granted() || Build.VERSION.SDK_INT < 23) {
                        answer(granted());
                        return;
                    }
                    Activity a = Bridge.activity.get();
                    if (a == null || a.isFinishing()) {
                        answer(false);
                        return;
                    }
                    FragmentManager fm = a.getFragmentManager();
                    Asker f = (Asker) fm.findFragmentByTag(TAG);
                    if (f == null) {
                        f = new Asker();
                        fm.beginTransaction().add(f, TAG).commitAllowingStateLoss();
                        fm.executePendingTransactions();
                    }
                    f.ask();
                } catch (Throwable t) {
                    Log.w(Bridge.TAG, "microphone permission request failed", t);
                    answer(false);
                }
            }
        });
    }

    static void answer(final boolean ok) {
        Native.ok(new Runnable() { public void run() { Native.micPermission(ok); } });
    }

    /** Push-to-talk pressed ({@code true}) or released. */
    static void ptt(final boolean down) {
        Native.ok(new Runnable() { public void run() { Native.ptt(down); } });
    }

    /** Headless fragment that asks and receives the answer. */
    public static final class Asker extends Fragment {
        private boolean asking;

        void ask() {
            if (asking) return;
            asking = true;
            requestPermissions(new String[] {PERMISSION}, REQUEST);
        }

        @Override
        public void onRequestPermissionsResult(int code, String[] perms, int[] results) {
            if (code != REQUEST) return;
            asking = false;
            answer(results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED);
        }
    }
}
