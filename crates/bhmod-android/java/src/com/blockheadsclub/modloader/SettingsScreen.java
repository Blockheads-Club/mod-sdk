// Blockheads Club mod loader: a mod's settings screen (API 0.3.4 `ui.open-settings`, V3 in-game admin settings).
// Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.text.InputFilter;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * The form the loader draws from a settings schema (`{fields, values}`, the same JSON the host app's Settings tab
 * renders), in up to four sections: the mod's `scope = "client"` settings (stored on this phone), its key bindings,
 * the voice volume (for a mod holding `voice`) and, for a server admin, the server's settings of that mod
 * (`SETTINGS_SCHEMA`; a change goes to the server as `SETTINGS_SET` and its answer comes back). Every change is
 * sent at once (`Native.settingSet`); the native side answers with a fresh screen or an error.
 *
 * <p>Types: `bool` (switch), `int`/`float` (slider with min/max/step, else a number field), `enum` (cycles through
 * its values), `string` (text field), anything else (`roles`, `matrix`, `list`) as JSON text.
 */
final class SettingsScreen {
    static boolean open;
    static String mod;
    private static int scrollY;
    private static ScrollView scroll;

    private SettingsScreen() {}

    static boolean handles(String t) {
        return "settings".equals(t) || "settings-result".equals(t);
    }

    static void command(String t, JSONObject c) {
        Activity a = Overlay.activity();
        if (a == null) return;
        if ("settings-result".equals(t)) {
            if (!c.optBoolean("ok")) Overlay.toast(c.optString("message"));
            else if (c.optBoolean("restart")) Overlay.toast(Strings.t("settings_restart"));
            return;
        }
        boolean same = open && c.optString("mod").equals(mod);
        if (same && scroll != null) scrollY = scroll.getScrollY();
        else scrollY = 0;
        show(a, c);
    }

    private static void set(final String scope, final String key, final Object value) {
        final String m = mod;
        final String json = value == null ? "null" : value instanceof String ? JSONObject.quote((String) value) : value.toString();
        Native.ok(new Runnable() { public void run() { Native.settingSet(m, scope, key, json); } });
    }

    private static String label(JSONObject f, String key) {
        Object l = f.opt(key);
        if (l instanceof JSONObject) return Strings.pick((JSONObject) l, f.optString("key"));
        if (l instanceof String && !((String) l).isEmpty()) return (String) l;
        return "help".equals(key) ? "" : f.optString("key");
    }

    private static void show(final Activity a, JSONObject c) {
        mod = c.optString("mod");
        LinearLayout body = Kit.column(a);
        boolean any = false;
        JSONObject client = c.optJSONObject("client");
        if (client != null && client.optJSONArray("fields") != null && client.optJSONArray("fields").length() > 0) {
            section(a, body, Strings.t("settings_mine"));
            form(a, body, "client", client);
            any = true;
        }
        JSONObject voice = c.optJSONObject("voice");
        if (voice != null) {
            section(a, body, Strings.t("settings_voice"));
            JSONObject f = new JSONObject();
            try {
                f.put("key", "master").put("type", "int").put("min", 0).put("max", 200).put("step", 5).put("label", Strings.t("settings_voice") + " %");
            } catch (org.json.JSONException ignored) {
            }
            body.addView(row(a, "voice", f, voice.opt("master")), Kit.fill());
            JSONArray ps = voice.optJSONArray("players");
            for (int i = 0; ps != null && i < ps.length(); i++) {
                JSONObject p = ps.optJSONObject(i);
                if (p == null) continue;
                String id = p.optString("id");
                try {
                    JSONObject vol = new JSONObject().put("key", "player:" + id).put("type", "int").put("min", 0).put("max", 200)
                        .put("step", 5).put("label", p.optString("name", id) + " %");
                    body.addView(row(a, "voice", vol, p.opt("volume")), Kit.fill());
                    JSONObject mute = new JSONObject().put("key", "mute:" + id).put("type", "bool")
                        .put("label", Strings.t("settings_mute", "name", p.optString("name", id)));
                    body.addView(row(a, "voice", mute, p.opt("muted")), Kit.fill());
                } catch (org.json.JSONException ignored) {
                }
            }
            any = true;
        }
        JSONArray keys = c.optJSONArray("keys");
        if (keys != null && keys.length() > 0) {
            section(a, body, Strings.t("settings_keys"));
            for (int i = 0; i < keys.length(); i++) body.addView(keyRow(a, keys.optJSONObject(i)), Kit.fill());
            any = true;
        }
        JSONObject server = c.optJSONObject("server");
        if (server != null && server.optJSONArray("fields") != null && server.optJSONArray("fields").length() > 0) {
            section(a, body, Strings.t("settings_server"));
            form(a, body, "server", server);
            any = true;
        }
        if (!any) body.addView(Kit.body(a, Strings.t("settings_none")), Kit.fill());
        View.OnClickListener close = new View.OnClickListener() { public void onClick(View v) { close(); } };
        View panel = Overlay.frame(Strings.t("settings_title", "mod", c.optJSONObject("name") != null ? Strings.pick(c.optJSONObject("name"), mod) : c.optString("name", mod)), null, body,
            Kit.footer(a, Kit.button(a, Strings.t("done"), Kit.Kind.PRI, close)));
        Overlay.show(Overlay.onBack(panel, new Runnable() { public void run() { close(); } }));
        open = true;
        scroll = find(panel);
        if (scroll != null) {
            final ScrollView sv = scroll;
            final int y = scrollY;
            sv.post(new Runnable() { public void run() { sv.scrollTo(0, y); } });
        }
    }

    private static ScrollView find(View v) {
        if (v instanceof ScrollView) return (ScrollView) v;
        if (v instanceof ViewGroup) {
            ViewGroup g = (ViewGroup) v;
            for (int i = 0; i < g.getChildCount(); i++) {
                ScrollView s = find(g.getChildAt(i));
                if (s != null) return s;
            }
        }
        return null;
    }

    static void close() {
        open = false;
        scroll = null;
        Overlay.dismiss();
    }

    private static void section(Activity a, LinearLayout body, String title) {
        TextView t = Kit.text(a, title, Kit.SUN, 15, true);
        if (android.os.Build.VERSION.SDK_INT >= 28) t.setAccessibilityHeading(true);
        body.addView(t, Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, Kit.u(a, 1)));
    }

    private static void form(Activity a, LinearLayout body, String scope, JSONObject schema) {
        JSONArray fields = schema.optJSONArray("fields");
        JSONObject values = schema.optJSONObject("values");
        // Group ids are headings only when a form has several groups ("modes", "talk_matrix" → "Modes", "Talk matrix").
        java.util.Set<String> groups = new java.util.HashSet<>();
        for (int i = 0; fields != null && i < fields.length(); i++) {
            JSONObject f = fields.optJSONObject(i);
            if (f != null && !f.optString("group", "").isEmpty()) groups.add(f.optString("group"));
        }
        String group = null;
        for (int i = 0; fields != null && i < fields.length(); i++) {
            JSONObject f = fields.optJSONObject(i);
            if (f == null) continue;
            String g = f.optString("group", "");
            if (groups.size() > 1 && !g.isEmpty() && !g.equals(group)) {
                String h = g.replace('_', ' ').replace('-', ' ');
                body.addView(Kit.small(a, Character.toUpperCase(h.charAt(0)) + h.substring(1)), Kit.margins(Kit.fill(), 0, Kit.u(a, 2), 0, 0));
                group = g;
            }
            Object v = values == null ? f.opt("default") : values.opt(f.optString("key"));
            body.addView(row(a, scope, f, v), Kit.fill());
        }
    }

    /** One field: label (+ help) and the control for its type. */
    private static View row(final Activity a, final String scope, final JSONObject f, final Object value) {
        final String key = f.optString("key");
        String type = f.optString("type");
        LinearLayout col = Kit.column(a);
        col.setPadding(0, Kit.u(a, 1), 0, Kit.u(a, 2));
        LinearLayout head = Kit.row(a);
        TextView name = Kit.body(a, label(f, "label"));
        head.addView(name, Kit.weight(1));
        col.addView(head, Kit.fill());
        switch (type) {
            case "bool": {
                final boolean on = Boolean.TRUE.equals(value);
                head.addView(Kit.button(a, Strings.t(on ? "on" : "off"), on ? Kit.Kind.PRI : Kit.Kind.SEC, new View.OnClickListener() {
                    public void onClick(View v) { set(scope, key, !on); }
                }));
                break;
            }
            case "int":
            case "float": {
                final boolean isInt = "int".equals(type);
                final double v = value instanceof Number ? ((Number) value).doubleValue() : f.optDouble("default", 0);
                final TextView shown = Kit.text(a, fmt(v, isInt), Kit.TEXT, 15, true);
                head.addView(shown);
                if (f.has("min") && f.has("max")) {
                    final double min = f.optDouble("min"), max = f.optDouble("max");
                    final double step = f.optDouble("step", isInt ? 1 : (max - min) / 100);
                    final int steps = (int) Math.max(1, Math.round((max - min) / step));
                    SeekBar bar = new SeekBar(a);
                    bar.setMax(steps);
                    bar.setProgress((int) Math.round((v - min) / step));
                    bar.setContentDescription(label(f, "label"));
                    bar.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                        double at(SeekBar s) { return Math.min(max, min + s.getProgress() * step); }
                        public void onProgressChanged(SeekBar s, int p, boolean user) { shown.setText(fmt(at(s), isInt)); }
                        public void onStartTrackingTouch(SeekBar s) {}
                        public void onStopTrackingTouch(SeekBar s) {
                            double x = at(s);
                            set(scope, key, isInt ? (Object) Math.round(x) : (Object) x);
                        }
                    });
                    col.addView(bar, Kit.fill());
                } else {
                    col.addView(field(a, fmt(v, isInt), InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_FLAG_SIGNED | (isInt ? 0 : InputType.TYPE_NUMBER_FLAG_DECIMAL), 32, new Done() {
                        public void done(String s) {
                            try {
                                set(scope, key, isInt ? (Object) Long.parseLong(s.trim()) : (Object) Double.parseDouble(s.trim()));
                            } catch (NumberFormatException e) {
                                Overlay.toast(s);
                            }
                        }
                    }), Kit.fill());
                }
                break;
            }
            case "enum": {
                final JSONArray vs = f.optJSONArray("values");
                final String cur = value == null ? "" : String.valueOf(value);
                head.addView(Kit.button(a, cur, Kit.Kind.SEC, new View.OnClickListener() {
                    public void onClick(View v) {
                        if (vs == null || vs.length() == 0) return;
                        int i = 0;
                        while (i < vs.length() && !vs.optString(i).equals(cur)) i++;
                        set(scope, key, vs.optString((i + 1) % vs.length()));
                    }
                }));
                break;
            }
            case "string":
                col.addView(field(a, value == null ? "" : String.valueOf(value), InputType.TYPE_CLASS_TEXT, f.optInt("max_length", 200), new Done() {
                    public void done(String s) { set(scope, key, s); }
                }), Kit.fill());
                break;
            default:
                // roles, matrix, list: edited as JSON text (the server validates).
                col.addView(field(a, value == null ? "null" : value.toString(), InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE, 4000, new Done() {
                    public void done(String s) {
                        try {
                            Object v = new org.json.JSONTokener(s).nextValue();
                            set(scope, key, v == JSONObject.NULL ? null : v);
                        } catch (org.json.JSONException e) {
                            Overlay.toast(e.getMessage());
                        }
                    }
                }), Kit.fill());
        }
        String help = label(f, "help");
        if (!help.isEmpty()) col.addView(Kit.small(a, help), Kit.fill());
        if (f.optBoolean("restart")) col.addView(Kit.small(a, Strings.t("settings_restart")), Kit.fill());
        return col;
    }

    private static String fmt(double v, boolean isInt) {
        return isInt ? String.valueOf(Math.round(v)) : String.valueOf(Math.round(v * 1000) / 1000.0);
    }

    interface Done {
        void done(String text);
    }

    private static EditText field(Activity a, String text, int type, int max, final Done done) {
        final EditText e = new EditText(a);
        e.setBackground(Kit.field(a));
        e.setTextColor(Kit.TEXT);
        e.setTypeface(Kit.body);
        e.setTextSize(15);
        e.setInputType(type);
        e.setImeOptions(EditorInfo.IME_ACTION_DONE);
        e.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
        e.setFilters(new InputFilter[] {new InputFilter.LengthFilter(Math.max(1, max))});
        e.setText(text);
        e.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            public boolean onEditorAction(TextView v, int id, KeyEvent ev) {
                if (id == EditorInfo.IME_ACTION_DONE || (ev != null && ev.getKeyCode() == KeyEvent.KEYCODE_ENTER)) {
                    done.done(e.getText().toString());
                    return true;
                }
                return false;
            }
        });
        return e;
    }

    /** A key binding: the key, "press a key" to rebind, None to clear, Default to go back. */
    private static View keyRow(final Activity a, JSONObject k) {
        final String action = k.optString("action");
        String key = k.isNull("key") ? null : k.optString("key");
        LinearLayout r = Kit.row(a);
        r.setPadding(0, Kit.u(a, 1), 0, Kit.u(a, 1));
        r.addView(Kit.body(a, k.optString("label", action)), Kit.weight(1));
        final TextView btn = Kit.button(a, key == null ? Strings.t("key_none") : key.toUpperCase(java.util.Locale.ROOT), Kit.Kind.SEC, null);
        btn.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                btn.setText(Strings.t("key_press"));
                btn.setFocusableInTouchMode(true);
                btn.requestFocus();
                btn.setOnKeyListener(new View.OnKeyListener() {
                    public boolean onKey(View v, int code, KeyEvent e) {
                        if (e.getAction() != KeyEvent.ACTION_UP || code == KeyEvent.KEYCODE_BACK) return false;
                        String n = Keys.name(code);
                        if (n == null) return true;
                        btn.setOnKeyListener(null);
                        set("keys", action, n);
                        return true;
                    }
                });
            }
        });
        r.addView(btn);
        r.addView(Kit.button(a, Strings.t("key_none"), Kit.Kind.SEC, new View.OnClickListener() {
            public void onClick(View v) { set("keys", action, null); }
        }), Kit.margins(Kit.wrap(), Kit.u(a, 1), 0, 0, 0));
        if (!k.optBoolean("isDefault", true)) {
            r.addView(Kit.button(a, Strings.t("key_default"), Kit.Kind.SEC, new View.OnClickListener() {
                public void onClick(View v) { set("keys", action, false); }
            }), Kit.margins(Kit.wrap(), Kit.u(a, 1), 0, 0, 0));
        }
        return r;
    }
}
