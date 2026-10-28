// Blockheads Club mod loader: the UI of client mods, drawn with the kit (IG-HUD, IG-ModUI). Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.graphics.Typeface;
import android.text.InputFilter;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Widgets stack in two slots under the game's own HUD, notices sit top centre (one at a time), toasts bottom right
 * (three at most, four seconds). Mods send data only: the kit's colours and faces apply, nothing takes focus from
 * the game except a dialog the mod opened. All calls arrive on the UI thread through {@link Overlay#command}.
 */
final class ModUi {
    private static final int MAX_TOASTS = 3;
    static final int CLOSED = -1;

    /** Widget views by `mod/id`, with their slot column. */
    private static final Map<String, View> widgets = new HashMap<>();
    private static final Map<String, LinearLayout> slotOf = new HashMap<>();
    private static LinearLayout left, right;
    private static FrameLayout attachedRoot;
    private static TextView notice;
    private static Runnable noticeHide;
    private static final List<View> toasts = new ArrayList<>();
    /** The open mod dialog: its id and mod, 0/null when none. */
    static int dialogId;
    static String dialogMod;

    private ModUi() {}

    static boolean handles(String t) {
        return t.startsWith("ui-");
    }

    static void command(String t, JSONObject c) {
        Activity a = Overlay.activity();
        if (a == null) return;
        String mod = c.optString("mod");
        switch (t) {
            case "ui-set": set(a, mod, c); break;
            case "ui-remove": remove(mod + "/" + c.optString("id")); break;
            case "ui-toast": toast(a, c.optString("text")); break;
            case "ui-notice": notice(a, c.optString("text"), c.optString("level")); break;
            case "ui-dialog": dialog(a, mod, c); break;
            case "ui-close": if (dialogId == c.optInt("id") && mod.equals(dialogMod)) closeDialog(false); break;
            case "ui-clear": clear(mod); break;
            default: break;
        }
    }

    /** The HUD columns live in the overlay's HUD window; they are rebuilt when that window is. */
    private static void ensureColumns(Activity a) {
        if (attachedRoot == Overlay.root && left != null) return;
        attachedRoot = Overlay.root;
        widgets.clear();
        slotOf.clear();
        toasts.clear();
        notice = null;
        left = Kit.column(a);
        right = Kit.column(a);
        int top = Kit.u(a, 30), side = Kit.u(a, 4);
        FrameLayout.LayoutParams l = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP | Gravity.START);
        l.setMargins(side + Overlay.inset(a, true), top, 0, 0);
        FrameLayout.LayoutParams r = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP | Gravity.END);
        r.setMargins(0, top, side, 0);
        Overlay.root.addView(left, l);
        Overlay.root.addView(right, r);
    }

    private static View build(Activity a, JSONObject c) {
        String kind = c.optString("kind");
        String icon = c.isNull("icon") ? "" : c.optString("icon");
        String title = c.isNull("title") ? "" : c.optString("title");
        String text = c.optString("text");
        LinearLayout box = Kit.column(a);
        box.setBackground(Kit.panel(a));
        box.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
        if ("panel".equals(kind)) {
            if (!title.isEmpty()) box.addView(Kit.text(a, title, Kit.TEXT, 13, true), Kit.fill());
            String[] lines = text.split("\n");
            for (int i = 0; i < lines.length && i < 4; i++) {
                TextView t = Kit.small(a, lines[i]);
                Kit.ellipsize(t);
                box.addView(t, Kit.fill());
            }
            return box;
        }
        LinearLayout row = Kit.row(a);
        if (Icons.has(icon)) {
            ImageView iv = new ImageView(a);
            int s = Kit.u(a, 6);
            iv.setImageDrawable(Icons.drawable(icon, Kit.SUN, s));
            iv.setContentDescription(icon);
            row.addView(iv, Kit.margins(new LinearLayout.LayoutParams(s, s), 0, 0, Kit.u(a, 2), 0));
        }
        TextView t = "counter".equals(kind) ? Kit.text(a, text, Kit.TEXT, 15, true) : Kit.small(a, text);
        Kit.ellipsize(t);
        row.addView(t);
        box.addView(row, Kit.wrap());
        return box;
    }

    private static void set(Activity a, String mod, JSONObject c) {
        if (Overlay.root == null) return;
        ensureColumns(a);
        String key = mod + "/" + c.optString("id");
        LinearLayout column = "top-right".equals(c.optString("slot")) ? right : left;
        View old = widgets.get(key);
        View v = build(a, c);
        v.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_NONE);
        LinearLayout.LayoutParams lp = Kit.margins(Kit.wrap(), 0, 0, 0, Kit.u(a, 2));
        if (old != null && slotOf.get(key) == column) {
            int at = column.indexOfChild(old);
            column.removeViewAt(at);
            column.addView(v, at, lp);
        } else {
            remove(key);
            column.addView(v, lp);
        }
        widgets.put(key, v);
        slotOf.put(key, column);
    }

    private static void remove(String key) {
        View v = widgets.remove(key);
        LinearLayout col = slotOf.remove(key);
        if (v != null && col != null) col.removeView(v);
    }

    private static void toast(Activity a, String text) {
        if (Overlay.root == null) return;
        ensureColumns(a);
        while (toasts.size() >= MAX_TOASTS) Overlay.root.removeView(toasts.remove(0));
        final TextView t = Kit.badge(a, text, Kit.TEXT, Kit.panel(a));
        t.setTextSize(13);
        t.setTypeface(Kit.body);
        t.setMaxWidth(Kit.u(a, 120));
        t.setSingleLine(false);
        t.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM | Gravity.END);
        lp.setMargins(0, 0, Kit.u(a, 4), Kit.u(a, 30) + toasts.size() * Kit.u(a, 20));
        Overlay.root.addView(t, lp);
        toasts.add(t);
        Bridge.ui.postDelayed(new Runnable() {
            public void run() {
                if (toasts.remove(t) && Overlay.root != null) Overlay.root.removeView(t);
            }
        }, 4000);
    }

    private static void notice(Activity a, String text, String level) {
        if (Overlay.root == null) return;
        ensureColumns(a);
        if (notice != null) Overlay.root.removeView(notice);
        if (noticeHide != null) Bridge.ui.removeCallbacks(noticeHide);
        int fg = Kit.TEXT;
        android.graphics.drawable.Drawable bg = Kit.panel(a);
        if ("error".equals(level)) bg = Kit.danger(a);
        else if ("warning".equals(level)) bg = Kit.banner(a);
        else if ("success".equals(level)) bg = new Kit.Box(Kit.OK, 0xFF46B070, 0xFF1F6A3E, Kit.u(a, 1), 0, false);
        final TextView t = Kit.badge(a, text, fg, bg);
        t.setTextSize(13);
        t.setTypeface(Kit.body);
        t.setMaxWidth(Kit.u(a, 160));
        t.setSingleLine(false);
        t.setGravity(Gravity.CENTER);
        t.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP | Gravity.CENTER_HORIZONTAL);
        lp.setMargins(0, Kit.u(a, 4), 0, 0);
        Overlay.root.addView(t, lp);
        notice = t;
        noticeHide = new Runnable() {
            public void run() {
                if (notice == t && Overlay.root != null) {
                    Overlay.root.removeView(t);
                    notice = null;
                }
            }
        };
        Bridge.ui.postDelayed(noticeHide, 5000);
    }

    private static void dialog(Activity a, final String mod, JSONObject c) {
        final int id = c.optInt("id");
        LinearLayout body = Kit.column(a);
        String text = c.optString("text");
        if (!text.isEmpty()) body.addView(Kit.body(a, text), Kit.fill());
        final EditText field;
        JSONObject f = c.optJSONObject("field");
        if (f != null) {
            field = new EditText(a);
            field.setBackground(Kit.field(a));
            field.setTextColor(Kit.TEXT);
            field.setHintTextColor(Kit.TEXT3);
            field.setTypeface(Kit.body);
            field.setTextSize(15);
            field.setSingleLine(true);
            field.setPadding(Kit.u(a, 3), Kit.u(a, 3), Kit.u(a, 3), Kit.u(a, 3));
            field.setHint(f.optString("placeholder"));
            field.setFilters(new InputFilter[] {new InputFilter.LengthFilter(Math.max(1, f.optInt("max", 200)))});
            field.setText(f.optString("initial"));
            body.addView(field, Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
        } else {
            field = null;
        }
        JSONArray bs = c.optJSONArray("buttons");
        List<View> buttons = new ArrayList<>();
        for (int i = 0; bs != null && i < bs.length(); i++) {
            final int index = i;
            JSONObject b = bs.optJSONObject(i);
            String kind = b.optString("kind");
            Kit.Kind k = "primary".equals(kind) ? Kit.Kind.PRI : "danger".equals(kind) ? Kit.Kind.DAN : Kit.Kind.SEC;
            buttons.add(Kit.button(a, b.optString("label"), k, new View.OnClickListener() {
                public void onClick(View v) {
                    answer(id, index, field == null ? null : field.getText().toString());
                }
            }));
        }
        View footer;
        if (buttons.size() <= 2) {
            footer = Kit.footer(a, buttons.toArray(new View[0]));
        } else {
            // Three or four buttons do not fit one row of the narrow dialog: two rows of two.
            LinearLayout rows = Kit.column(a);
            for (int i = 0; i < buttons.size(); i += 2) {
                rows.addView(Kit.footer(a, buttons.subList(i, Math.min(i + 2, buttons.size())).toArray(new View[0])), Kit.fill());
            }
            footer = rows;
        }
        View panel = Overlay.frame(c.optString("title"), null, body, footer);
        dialogId = id;
        dialogMod = mod;
        Overlay.show(Overlay.onBack(panel, new Runnable() { public void run() { closeDialog(true); } }));
    }

    private static void answer(final int id, final int button, final String text) {
        if (dialogId != id) return;
        closeDialog(false);
        Native.ok(new Runnable() { public void run() { Native.dialogAnswer(id, button, text); } });
    }

    /** Closes the open dialog; `report`: tell the mod it went away without an answer. */
    static void closeDialog(boolean report) {
        final int id = dialogId;
        if (id == 0) return;
        dialogId = 0;
        dialogMod = null;
        Overlay.dismiss();
        if (report) Native.ok(new Runnable() { public void run() { Native.dialogAnswer(id, CLOSED, null); } });
    }

    private static void clear(String mod) {
        for (String key : new ArrayList<>(widgets.keySet())) {
            if (key.startsWith(mod + "/")) remove(key);
        }
        if (mod.equals(dialogMod)) closeDialog(false);
        VoiceUi.clearMod(mod);
    }
}
