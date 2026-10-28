// Blockheads Club mod loader: overlay over the game view. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.SharedPreferences;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.PixelFormat;
import android.os.Build;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import org.json.JSONObject;

/**
 * Android views over the game (IG-Spec decision for Android: the game draws its own UIKit in GL; our UI uses the
 * platform view system so it gets TalkBack roles, the system keyboard, text scaling and safe-area insets, and it
 * runs on the UI thread so the game thread never waits). Holds the title mark, one dialog at a time and toasts.
 */
final class Overlay {
    static FrameLayout root;
    static TextView mark;
    static View dialog;
    static boolean ready, inWorld, welcomeChecked;
    static String version = "?";
    /** Running mods that have a settings screen (ids), from the last `world` command. */
    static org.json.JSONArray running;

    private Overlay() {}

    static Activity activity() {
        return Bridge.activity.get();
    }

    static Activity attachedTo;
    /** Views currently added to the window manager (isAttachedToWindow lags behind addView). */
    static final java.util.Set<View> added = new java.util.HashSet<>();
    static FrameLayout markWin, hudWin;

    /**
     * The game draws in its own dialog window above the activity window, so the loader UI uses windows of its own
     * (added with the activity token after the game's, so they stack above it): the title mark (touchable, not
     * focusable), a HUD layer for toasts (not touchable) and, while one is open, a dialog window (focusable).
     */
    static void attach(Activity a) {
        if (attachedTo == a && added.contains(markWin)) return;
        detach();
        attachedTo = a;
        Fonts.load(a);
        root = new FrameLayout(a);
        markWin = new FrameLayout(a);
        mark = Kit.badge(a, Strings.t("mark", "v", version), Kit.TEXT, Kit.panel(a));
        mark.setTextSize(13);
        mark.setPadding(Kit.u(a, 5), Kit.u(a, 3), Kit.u(a, 5), Kit.u(a, 3));
        mark.setContentDescription(Strings.t("mods"));
        mark.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { ModsScreen.open(); } });
        markWin.addView(mark);
        int m = Kit.u(a, 4);
        WindowManager.LayoutParams lp = params(WindowManager.LayoutParams.WRAP_CONTENT, WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL);
        lp.gravity = Gravity.BOTTOM | Gravity.START;
        lp.x = m;
        lp.y = m;
        add(markWin, lp);
        hudWin = root;
        add(hudWin, params(-1, -1, WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE));
        updateMark();
        if (dialog != null) {
            View d = dialog;
            dialog = null;
            show(d);
        }
        VoiceUi.reattach();
        maybeWelcome();
    }

    /** Re-adds the mark and HUD windows so they stack above game windows created after them. */
    static void raise() {
        Activity a = attachedTo;
        if (a == null || markWin == null) return;
        WindowManager.LayoutParams ml = (WindowManager.LayoutParams) markWin.getLayoutParams();
        WindowManager.LayoutParams hl = (WindowManager.LayoutParams) hudWin.getLayoutParams();
        if (ml == null || hl == null) return;
        remove(markWin);
        remove(hudWin);
        add(markWin, ml);
        add(hudWin, hl);
        VoiceUi.raise();
        if (dialog != null) {
            WindowManager.LayoutParams dl = (WindowManager.LayoutParams) dialog.getLayoutParams();
            remove(dialog);
            add(dialog, dl);
            dialog.requestFocus();
        }
    }

    static void detach() {
        for (View v : new View[] {markWin, hudWin, dialog}) remove(v);
        markWin = null;
        hudWin = null;
    }

    static WindowManager.LayoutParams params(int w, int h, int flags) {
        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(w, h, WindowManager.LayoutParams.TYPE_APPLICATION,
            flags | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN | WindowManager.LayoutParams.FLAG_FULLSCREEN, PixelFormat.TRANSLUCENT);
        if (Build.VERSION.SDK_INT >= 28) lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        return lp;
    }

    @SuppressWarnings("deprecation")
    static void add(View v, WindowManager.LayoutParams lp) {
        Activity a = attachedTo;
        if (a == null) return;
        v.setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_FULLSCREEN
            | View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
        try {
            a.getWindowManager().addView(v, lp);
            added.add(v);
        } catch (RuntimeException e) {
            android.util.Log.w(Bridge.TAG, "overlay window not added", e);
        }
    }

    static void remove(View v) {
        if (v == null || !added.remove(v)) return;
        try {
            ((WindowManager) v.getContext().getSystemService(Context.WINDOW_SERVICE)).removeViewImmediate(v);
        } catch (RuntimeException ignored) {
        }
    }

    /** Cutout / gesture inset: the left (true) or bottom (false) side, plus nothing else; 0 before API 28. */
    static int inset(Activity a, boolean left) {
        if (Build.VERSION.SDK_INT < 28) return 0;
        WindowInsets w = a.getWindow().getDecorView().getRootWindowInsets();
        if (w == null || w.getDisplayCutout() == null) return 0;
        return left ? w.getDisplayCutout().getSafeInsetLeft() : w.getDisplayCutout().getSafeInsetBottom();
    }

    static void updateMark() {
        if (mark == null) return;
        mark.setTypeface(Kit.title);
        mark.setText(Strings.t("mark", "v", version));
        // The game's main and pause menus carry a Mods button; the title mark only duplicated it.
        mark.setVisibility(View.GONE);
    }

    /** A command from the native side. */
    static void command(JSONObject c) {
        String t = c.optString("t");
        switch (t) {
            case "ready":
                if (activity() != null) Fonts.load(activity());
                Native.ok(new Runnable() { public void run() { Native.setModsLabel(Strings.t("mods").toUpperCase(java.util.Locale.getDefault())); } });
                if (!added.contains(markWin)) { Activity a = activity(); if (a != null) { attachedTo = null; attach(a); } }
                ready = c.optBoolean("hooks", false);
                version = c.optString("version", version);
                updateMark();
                maybeWelcome();
                Bridge.ui.postDelayed(new Runnable() { public void run() { raise(); } }, 5000);
                break;
            case "skipped": {
                org.json.JSONArray mods = c.optJSONArray("mods");
                StringBuilder names = new StringBuilder();
                for (int i = 0; mods != null && i < mods.length(); i++) {
                    JSONObject m = mods.optJSONObject(i);
                    if (m == null) continue;
                    if (names.length() > 0) names.append(", ");
                    names.append("voice".equals(m.optString("id")) ? Strings.t("voice_chat") : Strings.pick(m.optJSONObject("name"), m.optString("id")));
                }
                if (names.length() > 0) toast(Strings.t("skipped_mods", "mods", names));
                break;
            }
            case "open-mods":
                ModsScreen.open();
                break;
            case "main-menu":
                Voice.mainMenu();
                break;
            case "world":
                inWorld = c.optBoolean("open");
                running = c.optJSONArray("settings");
                if (inWorld) VoiceUi.watchTaps(true);
                if (!inWorld) {
                    VoiceUi.clearAll();
                    if (SettingsScreen.open) SettingsScreen.close();
                }
                updateMark();
                raise();
                break;
            default:
                if (ModUi.handles(t)) ModUi.command(t, c);
                else if (VoiceUi.handles(t)) VoiceUi.command(t, c);
                else if (SettingsScreen.handles(t)) SettingsScreen.command(t, c);
                else JoinFlow.command(t, c);
        }
    }

    // ---- dialogs ---------------------------------------------------------------------------------------------

    /** A dialog panel: banner title (with an optional back arrow), scrolling body and a fixed footer. */
    static View frame(String title, View.OnClickListener back, View body, View footer) {
        Activity a = activity();
        LinearLayout panel = Kit.column(a);
        panel.setBackground(Kit.panel(a));
        int p = Kit.u(a, 2);
        panel.setPadding(p, p, p, p);
        LinearLayout head = Kit.row(a);
        head.setBackground(Kit.banner(a));
        head.setPadding(Kit.u(a, 4), Kit.u(a, 2), Kit.u(a, 4), Kit.u(a, 2));
        if (back != null) {
            TextView arrow = Kit.title(a, "◀");
            arrow.setPadding(0, 0, Kit.u(a, 4), 0);
            arrow.setContentDescription(Strings.t("back"));
            arrow.setOnClickListener(back);
            head.addView(arrow);
        }
        TextView tv = Kit.title(a, title);
        if (Build.VERSION.SDK_INT >= 28) tv.setAccessibilityHeading(true);
        // A long title (a server name in a narrow portrait window) wraps to up to three lines instead of being cut off.
        tv.setMaxLines(3);
        tv.setEllipsize(android.text.TextUtils.TruncateAt.END);
        head.addView(tv, Kit.weight(1));
        panel.addView(head, Kit.fill());
        ScrollView sv = new ScrollView(a);
        sv.setFillViewport(false);
        int bp = Kit.u(a, 4);
        sv.setPadding(bp, bp, bp, bp);
        sv.addView(body);
        panel.addView(sv, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
        if (footer != null) {
            footer.setPadding(bp, 0, bp, bp);
            panel.addView(footer, Kit.fill());
        }
        return panel;
    }

    /** Shows `panel` as the only dialog, in a focusable window over the game: system back runs `onBack`. */
    static void show(final View panel) {
        show(panel, true);
    }

    /**
     * Shows `panel` as the only dialog. A focusable window takes the window focus from the game, which then resigns
     * active (it shows its PAUSE menu); with `focusable` false the dialog takes touches only (system back goes to the
     * game) until {@link #focusDialog()}, e.g. when the player starts typing in a field.
     */
    static void show(final View panel, boolean focusable) {
        Activity a = activity();
        if (a == null || attachedTo == null) {
            dialog = panel;
            return;
        }
        dismiss();
        FrameLayout scrim = new FrameLayout(a) {
            /** The panel width follows the window: after a rotation (landscape to portrait on the Fold cover screen) a width fixed at show time would run off the screen. */
            @Override protected void onMeasure(int wSpec, int hSpec) {
                int avail = MeasureSpec.getSize(wSpec);
                View p = getChildCount() > 0 ? getChildAt(0) : null;
                if (p != null && avail > 0) {
                    ViewGroup.LayoutParams pl = p.getLayoutParams();
                    int w = Math.min(avail - Kit.u(getContext(), 8), Kit.u(getContext(), 250));
                    if (pl.width != w) {
                        pl.width = w;
                        p.setLayoutParams(pl);
                    }
                }
                super.onMeasure(wSpec, hSpec);
            }

            /** A touch here never reaches the game: tell it the player is active (its idle PAUSE menu stays away). */
            @Override public boolean dispatchTouchEvent(android.view.MotionEvent e) {
                int act = e.getActionMasked();
                if (act == android.view.MotionEvent.ACTION_DOWN || act == android.view.MotionEvent.ACTION_UP) touched();
                return super.dispatchTouchEvent(e);
            }

            @Override public boolean dispatchKeyEvent(KeyEvent e) {
                if (e.getKeyCode() == KeyEvent.KEYCODE_BACK) {
                    if (e.getAction() == KeyEvent.ACTION_UP) {
                        Object back = panel.getTag();
                        if (back instanceof Runnable) ((Runnable) back).run();
                    }
                    return true;
                }
                return super.dispatchKeyEvent(e);
            }
        };
        scrim.setBackgroundColor(0x99000000);
        scrim.setClickable(true);
        scrim.setFocusableInTouchMode(true);
        int w = Math.min(a.getResources().getDisplayMetrics().widthPixels - Kit.u(a, 8), Kit.u(a, 250));
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(w, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER);
        lp.setMargins(0, Kit.u(a, 4), 0, Kit.u(a, 4));
        if (panel.getParent() instanceof ViewGroup) ((ViewGroup) panel.getParent()).removeView(panel);
        scrim.addView(panel, lp);
        WindowManager.LayoutParams wlp = params(-1, -1, focusable ? 0 : WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE);
        wlp.softInputMode = WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE;
        add(scrim, wlp);
        scrim.requestFocus();
        dialog = scrim;
        updateMark();
    }

    /** Lets the open dialog take the window focus (keyboard and system back). */
    static void focusDialog() {
        View d = dialog;
        if (d == null || !added.contains(d)) return;
        WindowManager.LayoutParams lp = (WindowManager.LayoutParams) d.getLayoutParams();
        if ((lp.flags & WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE) == 0) return;
        lp.flags &= ~WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE;
        try {
            ((WindowManager) d.getContext().getSystemService(Context.WINDOW_SERVICE)).updateViewLayout(d, lp);
        } catch (RuntimeException ignored) {
        }
        d.requestFocus();
    }

    /** The player touched a loader window (counts as activity for the game's idle pause). */
    static void touched() {
        Native.ok(new Runnable() { public void run() { Native.touched(); } });
    }

    static void dismiss() {
        remove(dialog);
        dialog = null;
        updateMark();
    }

    /** Sets what system back does for a panel. */
    static View onBack(View panel, Runnable r) {
        panel.setTag(r);
        return panel;
    }

    static void toast(String text) {
        Activity a = activity();
        if (a == null || hudWin == null) return;
        final TextView t = Kit.badge(a, text, Kit.TEXT, Kit.panel(a));
        t.setTextSize(13);
        t.setTypeface(Kit.body);
        t.setMaxWidth(Kit.u(a, 120));
        t.setSingleLine(false);
        t.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT,
            Gravity.BOTTOM | Gravity.END);
        lp.setMargins(0, 0, Kit.u(a, 4), Kit.u(a, 30));
        root.addView(t, lp);
        Bridge.ui.postDelayed(new Runnable() { public void run() { root.removeView(t); } }, 4000);
    }

    static void copy(String text) {
        Activity a = activity();
        if (a == null) return;
        ClipboardManager cm = (ClipboardManager) a.getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm != null) cm.setPrimaryClip(ClipData.newPlainText("Blockheads mods", text));
        toast(Strings.t("copied"));
    }

    // ---- welcome (once) ---------------------------------------------------------------------------------------

    static void maybeWelcome() {
        final Activity a = activity();
        if (!ready || welcomeChecked || a == null || attachedTo == null) return;
        welcomeChecked = true;
        final SharedPreferences prefs = a.getSharedPreferences("bhmods", Context.MODE_PRIVATE);
        if (prefs.getBoolean("welcomed", false)) return;
        Bridge.ui.postDelayed(new Runnable() {
            public void run() {
                prefs.edit().putBoolean("welcomed", true).apply();
                welcome(0);
            }
        }, 1500);
    }

    static void welcome(final int page) {
        Activity a = activity();
        if (a == null) return;
        LinearLayout body = Kit.column(a);
        String title;
        if (page == 0) {
            title = Strings.t("w1_title");
            body.addView(Kit.body(a, Strings.t("w1_body", "v", version)), Kit.fill());
            body.addView(Kit.small(a, Strings.t("w1_body2")), Kit.margins(Kit.fill(), 0, Kit.u(a, 4), 0, 0));
        } else if (page == 1) {
            title = Strings.t("w2_title");
            for (String k : new String[] {"w2_l1", "w2_l2", "w2_l3"}) {
                body.addView(Kit.body(a, "• " + Strings.t(k)), Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 2)));
            }
        } else {
            title = Strings.t("w3_title");
            body.addView(trust(a, Kit.badge(a, Strings.t("reviewed"), Kit.INK, new Kit.Box(Kit.OK, 0xFF46B070, 0xFF1F6A3E, Kit.u(a, 1), 0, false)), Strings.t("w3_reviewed")));
            body.addView(trust(a, Kit.badge(a, Strings.t("not_reviewed"), Kit.TEXT, Kit.well(a)), Strings.t("w3_not")));
            body.addView(trust(a, Kit.nativeBadge(a), Strings.t("w3_native")));
            body.addView(Kit.small(a, Strings.t("w3_note")), Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
        }
        View.OnClickListener skip = new View.OnClickListener() { public void onClick(View v) { dismiss(); } };
        View footer = page < 2
            ? Kit.footer(a, Kit.button(a, Strings.t("skip"), Kit.Kind.SEC, skip),
                Kit.button(a, Strings.t("next"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { welcome(page + 1); } }))
            : Kit.footer(a, Kit.button(a, Strings.t("play"), Kit.Kind.SEC, skip),
                Kit.button(a, Strings.t("open_mods"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { ModsScreen.open(); } }));
        show(onBack(frame(title, null, body, footer), new Runnable() { public void run() { dismiss(); } }));
    }

    private static View trust(Activity a, View badge, String text) {
        LinearLayout r = Kit.row(a);
        r.addView(badge, Kit.margins(Kit.wrap(), 0, 0, Kit.u(a, 3), 0));
        TextView t = Kit.small(a, text);
        r.addView(t, Kit.weight(1));
        r.setPadding(0, 0, 0, Kit.u(a, 2));
        return r;
    }

    /** A thin progress bar in a well; `fraction < 0` draws the busy stripes. */
    static final class Bar extends View {
        float fraction;
        private final Paint p = new Paint();

        Bar(Context c) {
            super(c);
            setBackground(Kit.well(c));
            setMinimumHeight(Kit.u(c, 6));
            setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_YES);
        }

        void set(float f) {
            fraction = f;
            setContentDescription(f < 0 ? "…" : Math.round(f * 100) + "%");
            invalidate();
        }

        @Override protected void onDraw(Canvas c) {
            super.onDraw(c);
            int px = Kit.u(getContext(), 1);
            p.setColor(Kit.SUN);
            int w = getWidth() - 2 * px;
            if (fraction >= 0) {
                c.drawRect(px, px, px + w * Math.min(1f, fraction), getHeight() - px, p);
            } else {
                long t = (System.currentTimeMillis() / 40) % (px * 8);
                for (int x = (int) -t; x < w; x += px * 8) c.drawRect(px + Math.max(0, x), px, px + Math.max(0, Math.min(w, x + px * 4)), getHeight() - px, p);
                postInvalidateDelayed(80);
            }
        }
    }
}
