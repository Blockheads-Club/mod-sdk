// Blockheads Club mod loader: on-screen buttons, overhead icons and the mic-on indicator (API 0.3.4).
// Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.drawable.Drawable;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * The voice-era HUD. Buttons are round, draggable, each in a small touchable window of its own (the HUD window does
 * not take touches, so the game keeps every touch outside a button). A touch sends `press`; holding 500 ms without
 * moving also sends `long-press` (the mod decides what it means: a push-to-talk button keeps talking until `release`);
 * moving starts a drag (`release` first) and the drop position is stored on the device. The loader has no menus of
 * its own on mod buttons. Overhead icons and the mic-on indicator live in the HUD window and never take touches.
 */
final class VoiceUi {
    static final long LONG_PRESS_MS = 500;

    private VoiceUi() {}

    static boolean handles(String t) {
        return t.startsWith("btn-") || "overhead".equals(t) || t.startsWith("voice-");
    }

    static void command(String t, JSONObject c) {
        Activity a = Overlay.activity();
        if (a == null) return;
        switch (t) {
            case "btn-set": setButton(a, c); break;
            case "btn-remove": removeButton(c.optString("mod") + "/" + c.optString("id")); break;
            case "overhead": overhead(a, c.optJSONArray("items")); break;
            case "voice-mic": indicator(a, c.optBoolean("on"), c.optString("mode")); break;
            case "voice-denied": Overlay.toast(Strings.t("mic_denied")); break;
            default: break;
        }
    }

    static String modeName(String m) {
        return Strings.t("mode_" + m);
    }

    // ---- buttons -----------------------------------------------------------------------------------------------

    private static final Map<String, Btn> buttons = new HashMap<>();

    static void clearMod(String mod) {
        for (String k : new ArrayList<>(buttons.keySet())) {
            if (k.startsWith(mod + "/")) removeButton(k);
        }
    }

    /** The world closed or the overlay is rebuilt: everything goes. */
    static void clearAll() {
        watchTaps(false);
        for (String k : new ArrayList<>(buttons.keySet())) removeButton(k);
        overhead(Overlay.activity(), null);
        if (indicator != null && Overlay.root != null) Overlay.root.removeView(indicator);
        indicator = null;
    }

    /** The overlay moved to a new activity: the buttons are made again there (their windows belonged to the old one). */
    static void reattach() {
        Activity a = Overlay.activity();
        List<JSONObject> again = new ArrayList<>();
        for (String k : new ArrayList<>(buttons.keySet())) {
            Btn b = buttons.get(k);
            if (b.last != null) again.add(b.last);
            removeButton(k);
        }
        if (a == null) return;
        for (JSONObject c : again) setButton(a, c);
    }

    /** Re-adds the button windows above game windows created after them (with the HUD, see Overlay.raise). */
    static void raise() {
        for (Btn b : buttons.values()) {
            if (b.lp == null || !Overlay.added.contains(b.win)) continue;
            Overlay.remove(b.win);
            Overlay.add(b.win, b.lp);
        }
    }

    private static void removeButton(String key) {
        Btn b = buttons.remove(key);
        if (b != null) Overlay.remove(b.win);
    }

    private static void setButton(Activity a, JSONObject c) {
        String mod = c.optString("mod"), id = c.optString("id");
        String key = mod + "/" + id;
        Btn b = buttons.get(key);
        if (b == null || !Overlay.added.contains(b.win)) {
            if (b != null) removeButton(key);
            b = new Btn(a, mod, id);
            buttons.put(key, b);
        }
        b.update(a, c);
    }

    private static int screenW(Activity a) {
        return Overlay.root != null && Overlay.root.getWidth() > 0 ? Overlay.root.getWidth() : a.getResources().getDisplayMetrics().widthPixels;
    }

    private static int screenH(Activity a) {
        return Overlay.root != null && Overlay.root.getHeight() > 0 ? Overlay.root.getHeight() : a.getResources().getDisplayMetrics().heightPixels;
    }

    /** One round button in its own window. */
    static final class Btn implements View.OnTouchListener {
        final String mod, id;
        final FrameLayout win;
        final Round face;
        WindowManager.LayoutParams lp;
        String icon = "";
        int px;
        float cx, cy;
        float downX, downY;
        int startX, startY;
        boolean dragging, held, longFired;
        final Runnable longPress = new Runnable() {
            public void run() {
                if (!held || dragging) return;
                longFired = true;
                send("long-press");
            }
        };

        Btn(Activity a, String mod, String id) {
            this.mod = mod;
            this.id = id;
            win = new FrameLayout(a);
            face = new Round(a);
            win.addView(face);
            win.setOnTouchListener(this);
        }

        JSONObject last;

        void update(Activity a, JSONObject c) {
            last = c;
            icon = c.optString("icon");
            String label = c.isNull("label") ? "" : c.optString("label");
            px = Math.round(c.optInt("size", 64) * a.getResources().getDisplayMetrics().density);
            face.set(icon, label, px);
            win.setAlpha(Math.max(10, Math.min(100, c.optInt("opacity", 80))) / 100f);
            win.setContentDescription(label.isEmpty() ? icon : label);
            JSONArray saved = c.optJSONArray("saved");
            cx = (float) (saved != null ? saved.optDouble(0, c.optDouble("x")) : c.optDouble("x", 0.9));
            cy = (float) (saved != null ? saved.optDouble(1, c.optDouble("y")) : c.optDouble("y", 0.7));
            boolean fresh = lp == null;
            if (fresh) {
                lp = Overlay.params(px, px, WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL);
                lp.gravity = Gravity.TOP | Gravity.START;
            }
            lp.width = px;
            lp.height = px;
            place(a);
            if (fresh) Overlay.add(win, lp);
            else if (Overlay.added.contains(win)) a.getWindowManager().updateViewLayout(win, lp);
        }

        void place(Activity a) {
            int w = screenW(a), h = screenH(a);
            lp.x = Math.max(0, Math.min(w - px, Math.round(cx * w - px / 2f)));
            lp.y = Math.max(0, Math.min(h - px, Math.round(cy * h - px / 2f)));
        }

        void send(final String action) {
            Native.ok(new Runnable() { public void run() { Native.uiButton(mod, id, action); } });
        }

        @Override public boolean onTouch(View v, MotionEvent e) {
            Activity a = Overlay.activity();
            if (a == null) return false;
            int slop = ViewConfiguration.get(a).getScaledTouchSlop() * 2;
            switch (e.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    downX = e.getRawX();
                    downY = e.getRawY();
                    startX = lp.x;
                    startY = lp.y;
                    dragging = false;
                    longFired = false;
                    held = true;
                    face.setPressed(true);
                    send("press");
                    Bridge.ui.postDelayed(longPress, LONG_PRESS_MS);
                    return true;
                case MotionEvent.ACTION_MOVE: {
                    float dx = e.getRawX() - downX, dy = e.getRawY() - downY;
                    if (!dragging && !longFired && Math.hypot(dx, dy) > slop) {
                        Bridge.ui.removeCallbacks(longPress);
                        dragging = true;
                        send("release");
                    }
                    if (dragging) {
                        int w = screenW(a), h = screenH(a);
                        lp.x = Math.max(0, Math.min(w - px, startX + Math.round(dx)));
                        lp.y = Math.max(0, Math.min(h - px, startY + Math.round(dy)));
                        if (Overlay.added.contains(win)) a.getWindowManager().updateViewLayout(win, lp);
                    }
                    return true;
                }
                case MotionEvent.ACTION_UP:
                case MotionEvent.ACTION_CANCEL:
                    Bridge.ui.removeCallbacks(longPress);
                    held = false;
                    face.setPressed(false);
                    if (dragging) {
                        int w = screenW(a), h = screenH(a);
                        cx = (lp.x + px / 2f) / w;
                        cy = (lp.y + px / 2f) / h;
                        final float fx = cx, fy = cy;
                        Native.ok(new Runnable() { public void run() { Native.buttonMoved(mod, id, fx, fy); } });
                    } else {
                        send("release");
                    }
                    dragging = false;
                    return true;
                default:
                    return true;
            }
        }
    }

    /** The round face of a button: a disc in the kit's colours, the icon in the middle, a label under it. */
    static final class Round extends View {
        private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG), edge = new Paint(Paint.ANTI_ALIAS_FLAG), text = new Paint(Paint.ANTI_ALIAS_FLAG);
        private Drawable icon;
        private String label = "";
        private int px;

        Round(Context c) {
            super(c);
            edge.setStyle(Paint.Style.STROKE);
            text.setTextAlign(Paint.Align.CENTER);
            text.setColor(Kit.TEXT);
            setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_YES);
        }

        void set(String name, String label, int px) {
            this.px = px;
            this.label = label;
            int s = px / 2;
            icon = Icons.has(name) ? Icons.drawable(name, Kit.TEXT, s) : null;
            if (icon != null) icon.setBounds(0, 0, s, s);
            text.setTextSize(Math.max(8f, px / 7f));
            text.setTypeface(Kit.title);
            setLayoutParams(new FrameLayout.LayoutParams(px, px));
            invalidate();
        }

        @Override public void setPressed(boolean p) {
            super.setPressed(p);
            invalidate();
        }

        @Override protected void onDraw(Canvas c) {
            float r = px / 2f, stroke = Math.max(2f, px / 24f);
            fill.setColor(isPressed() ? Kit.SUN : Kit.PANEL);
            edge.setColor(isPressed() ? Kit.INK : Kit.PANEL_HI);
            edge.setStrokeWidth(stroke);
            c.drawCircle(r, r, r - stroke, fill);
            c.drawCircle(r, r, r - stroke, edge);
            boolean withLabel = !label.isEmpty() && px >= 56;
            if (icon != null) {
                int s = px / 2;
                c.save();
                c.translate((px - s) / 2f, withLabel ? px / 6f : (px - s) / 2f);
                icon.draw(c);
                c.restore();
            }
            if (withLabel) c.drawText(label, r, px * 0.86f, text);
        }
    }

    // ---- taps on blockheads (ui-pick) --------------------------------------------------------------------------

    private static View watcher;

    /**
     * While a world is open, a 1-pixel window with {@code FLAG_WATCH_OUTSIDE_TOUCH} sees where every touch on the
     * game starts (same app, so the coordinates are kept) without taking it; the native side picks the blockhead
     * under it.
     */
    static void watchTaps(boolean on) {
        Activity a = Overlay.activity();
        if (watcher != null) Overlay.remove(watcher);
        watcher = null;
        if (!on || a == null) return;
        View v = new View(a);
        v.setOnTouchListener(new View.OnTouchListener() {
            public boolean onTouch(View w, MotionEvent e) {
                if (e.getActionMasked() != MotionEvent.ACTION_OUTSIDE) return false;
                Activity act = Overlay.activity();
                if (act == null) return false;
                final float x = e.getRawX() / screenW(act), y = e.getRawY() / screenH(act);
                Native.ok(new Runnable() { public void run() { Native.tap(x, y); } });
                return false;
            }
        });
        WindowManager.LayoutParams lp = Overlay.params(1, 1, WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
            | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL | WindowManager.LayoutParams.FLAG_WATCH_OUTSIDE_TOUCH);
        lp.gravity = Gravity.TOP | Gravity.START;
        watcher = v;
        Overlay.add(v, lp);
    }

    // ---- overhead icons ----------------------------------------------------------------------------------------

    private static Heads heads;

    private static void overhead(Activity a, JSONArray items) {
        if (a == null || Overlay.root == null) return;
        if (heads == null || heads.getParent() != Overlay.root) {
            heads = new Heads(a);
            Overlay.root.addView(heads, 0, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        }
        heads.set(items);
    }

    /** Draws every player's icons centred on their head position (screen fractions). */
    static final class Heads extends View {
        private final List<float[]> at = new ArrayList<>();
        private final List<String[]> icons = new ArrayList<>();
        private final Map<String, Drawable> cache = new HashMap<>();
        private final Paint bg = new Paint(Paint.ANTI_ALIAS_FLAG);

        Heads(Context c) {
            super(c);
            setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);
            bg.setColor(0xB0000000);
        }

        void set(JSONArray items) {
            at.clear();
            icons.clear();
            for (int i = 0; items != null && i < items.length(); i++) {
                JSONObject it = items.optJSONObject(i);
                if (it == null) continue;
                JSONArray names = it.optJSONArray("icons");
                String[] n = new String[names == null ? 0 : names.length()];
                for (int k = 0; k < n.length; k++) n[k] = names.optString(k);
                at.add(new float[] {(float) it.optDouble("x"), (float) it.optDouble("y")});
                icons.add(n);
            }
            invalidate();
        }

        private Drawable icon(String name, int s) {
            Drawable d = cache.get(name);
            if (d == null) {
                int color = "admin-mute".equals(name) || "deafen".equals(name) ? 0xFFE0563D : "speaking".equals(name) ? Kit.SUN : Kit.TEXT;
                d = Icons.drawable(name, color, s);
                d.setBounds(0, 0, s, s);
                cache.put(name, d);
            }
            return d;
        }

        @Override protected void onDraw(Canvas c) {
            int s = Kit.u(getContext(), 6), gap = Kit.u(getContext(), 1);
            boolean blink = (System.currentTimeMillis() / 300) % 2 == 0;
            boolean anySpeaking = false;
            for (int i = 0; i < at.size(); i++) {
                String[] n = icons.get(i);
                if (n.length == 0) continue;
                float w = n.length * s + (n.length - 1) * gap;
                float x = at.get(i)[0] * getWidth() - w / 2, y = at.get(i)[1] * getHeight() - s;
                c.drawRoundRect(x - gap * 2, y - gap * 2, x + w + gap * 2, y + s + gap * 2, gap * 2, gap * 2, bg);
                for (String name : n) {
                    boolean speaking = "speaking".equals(name);
                    anySpeaking |= speaking;
                    if (Icons.has(name) && !(speaking && !blink)) {
                        c.save();
                        c.translate(x, y);
                        icon(name, s).draw(c);
                        c.restore();
                    }
                    x += s + gap;
                }
            }
            // The speaking icon pulses while it is shown.
            if (anySpeaking) postInvalidateDelayed(150);
        }
    }

    // ---- mic-on indicator --------------------------------------------------------------------------------------

    private static TextView indicator;

    private static void indicator(Activity a, boolean on, String mode) {
        if (Overlay.root == null) return;
        if (!on) {
            if (indicator != null) Overlay.root.removeView(indicator);
            indicator = null;
            return;
        }
        if (indicator == null || indicator.getParent() != Overlay.root) {
            indicator = Kit.badge(a, "", Kit.INK, new Kit.Box(Kit.SUN, 0xFFFFE08A, 0xFF9C6A00, Kit.u(a, 1), 0, false));
            indicator.setTextSize(13);
            indicator.setTypeface(Kit.title);
            int s = Kit.u(a, 5);
            Drawable d = Icons.drawable("mic", Kit.INK, s);
            d.setBounds(0, 0, s, s);
            indicator.setCompoundDrawables(d, null, null, null);
            indicator.setCompoundDrawablePadding(Kit.u(a, 1));
            indicator.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
            FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP | Gravity.CENTER_HORIZONTAL);
            lp.setMargins(0, Kit.u(a, 18), 0, 0);
            Overlay.root.addView(indicator, lp);
        }
        indicator.setText(Strings.t("mic_on", "mode", modeName(mode.isEmpty() ? "proximity" : mode)));
    }
}
