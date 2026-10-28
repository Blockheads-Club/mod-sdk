// Blockheads Club mod loader: microphone permission and push-to-talk seam for voice chat.
// Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.Manifest;
import android.app.Activity;
import android.app.Fragment;
import android.app.FragmentManager;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Process;
import android.provider.Settings;
import android.util.Log;
import android.view.View;
import android.widget.LinearLayout;

/**
 * The {@code RECORD_AUDIO} runtime permission (the game targets SDK 27, so Android 6+ asks at run time; Blockheads
 * Patcher's "record-audio" patch declares it in the manifest).
 *
 * <p>The system dialog is an activity: it pauses the game, and the game leaves an online world when it is paused. So
 * the loader asks <b>outside any connection</b>: once on the first launch at the main menu (after a short explanation,
 * remembered, never again) and from the "Microphone for voice chat" switch on the Mods screen. In a world it asks only
 * when the native lifecycle guard is available ({@link Native#micGuardAvailable}): the guard keeps the game in its
 * background mode (network still polled) while our dialog is up. Without it a mod's {@code set-mic(true)} gets
 * {@code denied} and the loader says where to allow the microphone. The game's activity is not ours, so a headless
 * {@link Fragment} receives {@code onRequestPermissionsResult}.
 */
final class Voice {
    static final String PERMISSION = Manifest.permission.RECORD_AUDIO;
    static final int REQUEST = 0xB0C;
    static final String TAG = "bhmod-voice";
    static final String ASKED = "mic_first_asked", REQUESTED = "mic_requested";

    /** Called back on the UI thread with the answer of a request started from the Java side. */
    interface Answer { void on(boolean granted); }

    private static Answer pending;
    private static int firstTries;

    private Voice() {}

    /** True when the app holds the permission (always true before Android 6 when declared). */
    static boolean granted() {
        if (Bridge.app == null) return false;
        return Bridge.app.checkPermission(PERMISSION, Process.myPid(), Process.myUid()) == PackageManager.PERMISSION_GRANTED;
    }

    /** True when the manifest declares RECORD_AUDIO (the record-audio patch). */
    static boolean declared() {
        if (Bridge.app == null) return false;
        try {
            PackageInfo pi = Bridge.app.getPackageManager().getPackageInfo(Bridge.app.getPackageName(), PackageManager.GET_PERMISSIONS);
            if (pi.requestedPermissions == null) return false;
            for (String p : pi.requestedPermissions) if (PERMISSION.equals(p)) return true;
        } catch (Throwable t) {
            Log.w(Bridge.TAG, "manifest permissions unreadable", t);
        }
        return false;
    }

    /** No game connection: a world or a join in progress would be ended by the pause the system dialog causes. */
    static boolean safe() {
        return !Overlay.inWorld && !JoinFlow.active;
    }

    /** The system dialog may be shown now (outside a connection, or in one with the lifecycle guard). */
    static boolean mayAsk() {
        if (safe()) return true;
        final boolean[] g = {false};
        Native.ok(new Runnable() { public void run() { g[0] = Native.micGuardAvailable(); } });
        return g[0];
    }

    static SharedPreferences prefs() {
        return Bridge.app.getSharedPreferences("bhmods", Context.MODE_PRIVATE);
    }

    /** A request from the native side (a mod turned the microphone on); the answer goes to Native.micPermission. */
    static void request() {
        ask(null);
    }

    /** Shows the system dialog (UI thread) unless granted already; `done` (may be null) gets the answer too. */
    static void ask(final Answer done) {
        Bridge.ui.post(new Runnable() {
            public void run() {
                try {
                    if (granted() || Build.VERSION.SDK_INT < 23) {
                        answer(granted(), done);
                        return;
                    }
                    Activity a = Bridge.activity.get();
                    if (a == null || a.isFinishing() || !declared() || !mayAsk()) {
                        answer(false, done);
                        return;
                    }
                    // Android stops showing the dialog after "Don't ask again": the app's settings page is the way.
                    if (prefs().getBoolean(REQUESTED, false) && !a.shouldShowRequestPermissionRationale(PERMISSION) && safe()) {
                        openAppSettings(a);
                        answer(false, done);
                        return;
                    }
                    pending = done;
                    if (!safe()) Native.ok(new Runnable() { public void run() { Native.micGuard(true); } });
                    prefs().edit().putBoolean(REQUESTED, true).putBoolean(ASKED, true).apply();
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
                    answer(false, done);
                }
            }
        });
    }

    static void answer(final boolean ok, Answer done) {
        Log.i(Bridge.TAG, "voice: RECORD_AUDIO " + (ok ? "granted" : "not granted"));
        Native.ok(new Runnable() { public void run() { Native.micPermission(ok); } });
        if (done != null) done.on(ok);
        // The guard drops when the game is active again; this is the backstop if that never comes.
        Bridge.ui.postDelayed(new Runnable() {
            public void run() { Native.ok(new Runnable() { public void run() { Native.micGuard(false); } }); }
        }, 3000);
    }

    static void openAppSettings(Activity a) {
        try {
            a.startActivity(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", a.getPackageName(), null)));
            Overlay.toast(Strings.t("mic_settings"));
        } catch (Throwable t) {
            Log.w(Bridge.TAG, "app settings unavailable", t);
        }
    }

    // ---- first launch -----------------------------------------------------------------------------------------

    /**
     * The main menu was built (no connection): on the first launch with the record-audio patch, explain voice chat
     * once and offer the system dialog. Waits for the loader's own welcome and any game dialog to go away.
     */
    static void mainMenu() {
        firstTries = 0;
        Bridge.ui.postDelayed(new Runnable() { public void run() { tryFirstAsk(); } }, 2000);
    }

    static void tryFirstAsk() {
        Activity a = Bridge.activity.get();
        if (a == null || Bridge.app == null) return;
        if (prefs().getBoolean(ASKED, false) || granted() || !declared() || !safe()) {
            Log.i(Bridge.TAG, "voice: no first-launch question (asked " + prefs().getBoolean(ASKED, false) + ", granted " + granted()
                + ", declared " + declared() + ", outside a world " + safe() + ")");
            return;
        }
        // The game draws in its own dialog window, so the activity's focus says nothing; wait for our own dialogs.
        if (Overlay.dialog != null) {
            if (++firstTries < 40) Bridge.ui.postDelayed(new Runnable() { public void run() { tryFirstAsk(); } }, 3000);
            return;
        }
        Log.i(Bridge.TAG, "voice: first launch, offering the microphone");
        prefs().edit().putBoolean(ASKED, true).apply();
        LinearLayout body = Kit.column(a);
        body.addView(Kit.body(a, Strings.t("mic_ask_body")), Kit.fill());
        View.OnClickListener later = new View.OnClickListener() { public void onClick(View v) { Overlay.dismiss(); } };
        View footer = Kit.footer(a, Kit.button(a, Strings.t("not_now"), Kit.Kind.SEC, later),
            Kit.button(a, Strings.t("mic_allow"), Kit.Kind.PRI, new View.OnClickListener() {
                public void onClick(View v) {
                    Overlay.dismiss();
                    ask(null);
                }
            }));
        // Not focusable: taking the window focus would make the game resign active.
        Overlay.show(Overlay.onBack(Overlay.frame(Strings.t("mic_title"), null, body, footer), new Runnable() { public void run() { Overlay.dismiss(); } }), false);
    }

    // ---- Mods screen switch -------------------------------------------------------------------------------------

    /** The "Microphone for voice chat" box of the Mods screen. */
    static View modsBox(final Activity a, final Runnable refresh) {
        LinearLayout box = Kit.column(a);
        box.setBackground(Kit.well(a));
        box.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
        box.addView(Kit.text(a, Strings.t("mic_title"), Kit.TEXT, 15, true), Kit.fill());
        boolean declared = declared(), on = granted();
        box.addView(Kit.body(a, Strings.t(declared ? "mic_body" : "mic_no_patch")), Kit.fill());
        // A plain kit button (a Switch draws no visible track on the game's theme): "Allow the microphone" while it is
        // not granted, "On" (opens Android's app settings, where it is turned off) when it is.
        final boolean granted = declared && on;
        View s = Kit.button(a, granted ? Strings.t("mic_is_on") : Strings.t("mic_turn_on"), granted ? Kit.Kind.SEC : Kit.Kind.PRI,
            new View.OnClickListener() {
                public void onClick(View v) {
                    if (granted) {
                        // Android has no "un-grant" for apps: its app settings page does that.
                        if (safe()) openAppSettings(a);
                        else Overlay.toast(Strings.t("mic_in_world"));
                        return;
                    }
                    if (!mayAsk()) {
                        Overlay.toast(Strings.t("mic_in_world"));
                        return;
                    }
                    ask(new Answer() { public void on(boolean ok) { if (refresh != null) refresh.run(); } });
                }
            });
        s.setEnabled(declared);
        s.setContentDescription(Strings.t("mic_title"));
        box.addView(s, Kit.margins(Kit.wrap(), 0, Kit.u(a, 2), 0, 0));
        return box;
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
            Answer done = pending;
            pending = null;
            answer(results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED, done);
        }
    }
}
