// Blockheads Club mod loader: overlay over the game view. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.SharedPreferences;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.os.Build;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
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

    private Overlay() {}

    static Activity activity() {
        return Bridge.activity.get();
    }

    static void attach(Activity a) {
        if (root != null && root.getContext() == a && root.getParent() != null) return;
        if (root != null && root.getParent() instanceof ViewGroup) ((ViewGroup) root.getParent()).removeView(root);
        root = new FrameLayout(a);
        root.setClipChildren(false);
        ViewGroup decor = (ViewGroup) a.getWindow().getDecorView();
        decor.addView(root, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        mark = Kit.badge(a, Strings.t("mark", "v", version), Kit.TEXT, Kit.panel(a));
        mark.setTextSize(13);
        mark.setPadding(Kit.u(a, 5), Kit.u(a, 3), Kit.u(a, 5), Kit.u(a, 3));
        mark.setContentDescription(Strings.t("mods"));
        mark.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { ModsScreen.open(); } });
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT,
            Gravity.BOTTOM | Gravity.START);
        int m = Kit.u(a, 4);
        lp.setMargins(m + inset(a, true), m, m, m + inset(a, false));
        root.addView(mark, lp);
        updateMark();
        if (dialog != null) {
            View d = dialog;
            dialog = null;
            if (d.getParent() instanceof ViewGroup) ((ViewGroup) d.getParent()).removeView(d);
            show(d);
        }
        maybeWelcome();
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
        mark.setText(Strings.t("mark", "v", version));
        mark.setVisibility(ready && !inWorld && dialog == null ? View.VISIBLE : View.GONE);
    }

    /** A command from the native side. */
    static void command(JSONObject c) {
        String t = c.optString("t");
        switch (t) {
            case "ready":
                ready = c.optBoolean("hooks", false);
                version = c.optString("version", version);
                updateMark();
                maybeWelcome();
                break;
            case "world":
                inWorld = c.optBoolean("open");
                updateMark();
                break;
            default:
                JoinFlow.command(t, c);
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
        Kit.ellipsize(tv);
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

    /** Shows `panel` as the only dialog: a scrim blocks the game's touches; system back runs `onBack`. */
    static void show(final View panel) {
        Activity a = activity();
        if (a == null || root == null) {
            dialog = panel;
            return;
        }
        dismiss();
        FrameLayout scrim = new FrameLayout(a) {
            @Override public boolean dispatchKeyEvent(KeyEvent e) {
                if (e.getKeyCode() == KeyEvent.KEYCODE_BACK && e.getAction() == KeyEvent.ACTION_UP) {
                    Object back = panel.getTag();
                    if (back instanceof Runnable) ((Runnable) back).run();
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
        root.addView(scrim, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        scrim.requestFocus();
        dialog = scrim;
        updateMark();
    }

    static void dismiss() {
        if (dialog != null && dialog.getParent() instanceof ViewGroup) ((ViewGroup) dialog.getParent()).removeView(dialog);
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
        if (a == null || root == null) return;
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
        if (!ready || welcomeChecked || a == null || root == null) return;
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
