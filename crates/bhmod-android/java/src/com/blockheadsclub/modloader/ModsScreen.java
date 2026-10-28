// Blockheads Club mod loader: the Mods area (IG-Mods, IG-Browse, IG-Install). Copyright (C) 2026 Blockheads Club contributors.
// GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.content.ClipboardManager;
import android.content.Context;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.View;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicBoolean;

/** Installed / Mod list / From link. All file and network work runs on {@link Net#POOL}; installs apply at the next world. */
final class ModsScreen {
    static final int INSTALLED = 0, LIST = 1, LINK = 2;
    static int tab = INSTALLED;
    static LinearLayout content;
    static JSONArray index;
    static String query = "";
    static int installedCount = -1;

    private ModsScreen() {}

    static void open() {
        show(tab);
    }

    static void show(int which) {
        Activity a = Overlay.activity();
        if (a == null) return;
        tab = which;
        LinearLayout body = Kit.column(a);
        LinearLayout tabs = Kit.row(a);
        String[] names = {Strings.t("tab_installed") + (installedCount > 0 ? " " + installedCount : ""), Strings.t("tab_list"), Strings.t("tab_link")};
        for (int i = 0; i < 3; i++) {
            final int k = i;
            android.widget.Button b = Kit.button(a, names[i], i == which ? Kit.Kind.PRI : Kit.Kind.SEC, new View.OnClickListener() {
                public void onClick(View v) { show(k); }
            });
            b.setSelected(i == which);
            tabs.addView(b, Kit.margins(Kit.wrap(), 0, 0, Kit.u(a, 2), 0));
        }
        android.widget.HorizontalScrollView ts = new android.widget.HorizontalScrollView(a);
        ts.setHorizontalScrollBarEnabled(false);
        ts.addView(tabs);
        body.addView(ts, Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 3)));
        content = Kit.column(a);
        body.addView(content, Kit.fill());
        if (which == INSTALLED) {
            body.addView(Voice.modsBox(a, new Runnable() { public void run() { if (tab == INSTALLED && Overlay.dialog != null) show(INSTALLED); } }),
                Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
        }
        if (Overlay.inWorld) body.addView(Kit.small(a, Strings.t("online_note")), Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
        Runnable close = new Runnable() { public void run() { Overlay.dismiss(); } };
        View panel = Overlay.frame(Strings.t("mods"), new View.OnClickListener() { public void onClick(View v) { Overlay.dismiss(); } }, body, null);
        Overlay.show(Overlay.onBack(panel, close));
        if (which == INSTALLED) installed();
        else if (which == LIST) list();
        else link("");
    }

    static void busy(String text) {
        Activity a = Overlay.activity();
        content.removeAllViews();
        Overlay.Bar bar = new Overlay.Bar(a);
        bar.set(-1);
        content.addView(bar, new LinearLayout.LayoutParams(-1, Kit.u(a, 6)));
        content.addView(Kit.small(a, text), Kit.margins(Kit.fill(), 0, Kit.u(a, 2), 0, 0));
    }

    /** An empty / offline / error state: title, text, optional details line and buttons. */
    static void state(String title, String text, String details, View... buttons) {
        Activity a = Overlay.activity();
        content.removeAllViews();
        content.addView(Kit.text(a, title, Kit.TEXT, 16, true), Kit.fill());
        content.addView(Kit.body(a, text), Kit.margins(Kit.fill(), 0, Kit.u(a, 1), 0, Kit.u(a, 2)));
        if (details != null) {
            TextView d = Kit.text(a, details, Kit.TEXT3, 12, false);
            d.setTypeface(android.graphics.Typeface.MONOSPACE);
            content.addView(d, Kit.fill());
        }
        content.addView(Kit.footer(a, buttons), Kit.fill());
    }

    interface Done { void on(String json); }

    /** Runs a native call on the pool and hands its JSON to `done` on the UI thread. */
    static void call(final java.util.concurrent.Callable<String> c, final Done done) {
        Net.POOL.execute(new Runnable() {
            public void run() {
                String r;
                try {
                    r = c.call();
                } catch (Throwable t) {
                    r = "{\"ok\":false,\"code\":\"loader\",\"text\":" + JSONObject.quote(String.valueOf(t)) + "}";
                }
                final String res = r;
                Bridge.ui.post(new Runnable() { public void run() { done.on(res); } });
            }
        });
    }

    // ---- installed ---------------------------------------------------------------------------------------------

    static void installed() {
        busy(Strings.t("reading"));
        call(new java.util.concurrent.Callable<String>() { public String call() { return Native.listInstalled(); } }, new Done() {
            public void on(String json) {
                if (tab != INSTALLED || content == null) return;
                final Activity a = Overlay.activity();
                JSONObject r = parse(json);
                if (!r.optBoolean("ok")) {
                    state(Strings.t("cant_read"), r.optString("text"), r.optString("code"),
                        Kit.button(a, Strings.t("try_again"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { installed(); } }));
                    return;
                }
                JSONArray mods = r.optJSONArray("mods");
                installedCount = mods == null ? 0 : mods.length();
                if (installedCount == 0) {
                    state(Strings.t("no_mods"), Strings.t("no_mods_body"), null,
                        Kit.button(a, Strings.t("add_link"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View v) { show(LINK); } }),
                        Kit.button(a, Strings.t("browse"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { show(LIST); } }));
                    return;
                }
                content.removeAllViews();
                for (int i = 0; i < mods.length(); i++) {
                    final JSONObject m = mods.optJSONObject(i);
                    TextView st = Kit.badge(a, Strings.t(m.optBoolean("enabled") ? (Overlay.inWorld ? "st_running" : "on") : "st_off"), Kit.TEXT2, Kit.well(a));
                    View row = JoinFlow.modRow(a, m, m.optString("version") + " · " + author(m), st);
                    row.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { details(m); } });
                    content.addView(row, Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 2)));
                }
                content.addView(Kit.small(a, Strings.t("apply_note")), Kit.fill());
                settingsButtons(a);
                jvmSwitch(a);
            }
        });
    }

    /** In a world: a Settings button for every running mod that has a settings screen (server mods too). */
    static void settingsButtons(Activity a) {
        JSONArray ids = Overlay.running;
        if (!Overlay.inWorld || ids == null || ids.length() == 0) return;
        for (int i = 0; i < ids.length(); i++) {
            final String id = ids.optString(i);
            content.addView(Kit.button(a, Strings.t("settings") + ": " + id, Kit.Kind.SEC, new View.OnClickListener() {
                public void onClick(View v) { Native.ok(new Runnable() { public void run() { Native.openSettings(id); } }); }
            }), Kit.margins(Kit.fill(), 0, Kit.u(a, 2), 0, 0));
        }
    }

    /** The flag file the native loader reads at start; present = JVM mods (unsandboxed) are on. Off by default. */
    static File jvmFlag() { return new File(Bridge.dataDir(), "jvm-enabled"); }

    static boolean setJvm(boolean on) {
        File f = jvmFlag();
        try {
            if (on) return f.exists() || f.createNewFile();
            return !f.exists() || f.delete();
        } catch (java.io.IOException e) {
            return false;
        }
    }

    /** "Java mods" switch with the red warning: JVM code cannot be sandboxed. Applies at the next game start. */
    static void jvmSwitch(final Activity a) {
        LinearLayout box = Kit.column(a);
        box.setBackground(Kit.danger(a));
        box.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
        box.addView(Kit.text(a, Strings.t("jvm_title"), Kit.TEXT, 15, true), Kit.fill());
        box.addView(Kit.body(a, Strings.t("jvm_body")), Kit.fill());
        final android.widget.Switch s = new android.widget.Switch(a);
        s.setChecked(jvmFlag().exists());
        s.setText(Strings.t(s.isChecked() ? "on" : "off"));
        s.setTextColor(Kit.TEXT);
        s.setOnCheckedChangeListener(new android.widget.CompoundButton.OnCheckedChangeListener() {
            public void onCheckedChanged(android.widget.CompoundButton b, boolean v) {
                if (!setJvm(v)) {
                    b.setOnCheckedChangeListener(null);
                    b.setChecked(!v);
                    Overlay.toast(Strings.t("cant_read"));
                    return;
                }
                b.setText(Strings.t(v ? "on" : "off"));
                Overlay.toast(Strings.t("jvm_restart"));
            }
        });
        box.addView(s, Kit.margins(Kit.wrap(), 0, Kit.u(a, 1), 0, 0));
        content.addView(box, Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
    }

    static String author(JSONObject m) {
        JSONArray au = m.optJSONArray("authors");
        if (au == null || au.length() == 0) au = m.optJSONArray("owners");
        return au == null || au.length() == 0 ? Strings.t("from_link") : Strings.t("by", "a", au.optString(0));
    }

    static void details(final JSONObject m) {
        final Activity a = Overlay.activity();
        content.removeAllViews();
        final String id = m.optString("id");
        final String name = Strings.pick(m.optJSONObject("name"), id);
        TextView back = Kit.text(a, "◀ " + Strings.t("back"), Kit.SUN, 14, true);
        back.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { installed(); } });
        content.addView(back, Kit.margins(Kit.wrap(), 0, 0, 0, Kit.u(a, 2)));
        content.addView(Kit.text(a, name, Kit.TEXT, 18, true), Kit.fill());
        String sides = m.optBoolean("client") && m.optBoolean("server") ? Strings.t("side_both") : Strings.t("side_client");
        content.addView(Kit.small(a, m.optString("version") + " · " + author(m) + " · " + sides + " · "
            + Strings.t(m.optString("source", "").startsWith("index:") ? "reviewed" : "not_reviewed")), Kit.fill());
        LinearLayout sw = Kit.row(a);
        final boolean on = m.optBoolean("enabled");
        android.widget.Switch s = new android.widget.Switch(a);
        s.setChecked(on);
        s.setText(Strings.t(on ? "on" : "off"));
        s.setTextColor(Kit.TEXT);
        s.setOnCheckedChangeListener(new android.widget.CompoundButton.OnCheckedChangeListener() {
            public void onCheckedChanged(android.widget.CompoundButton b, final boolean v) {
                b.setText(Strings.t(v ? "on" : "off"));
                call(new java.util.concurrent.Callable<String>() {
                    public String call() { return String.valueOf(Native.setEnabled(id, v)); }
                }, new Done() { public void on(String r) { Overlay.toast(Strings.t("apply_note")); } });
            }
        });
        sw.addView(s);
        content.addView(sw, Kit.margins(Kit.fill(), 0, Kit.u(a, 2), 0, Kit.u(a, 2)));
        String desc = Strings.pick(m.optJSONObject("description"), "");
        if (!desc.isEmpty()) content.addView(Kit.body(a, desc), Kit.fill());
        content.addView(Kit.text(a, Strings.t("what_asks"), Kit.TEXT, 15, true), Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, Kit.u(a, 1)));
        for (String p : JoinFlow.perms(m.optJSONObject("permissions"))) content.addView(Kit.small(a, "• " + p), Kit.fill());
        content.addView(Kit.footer(a, Kit.button(a, Strings.t("remove"), Kit.Kind.DAN, new View.OnClickListener() {
            public void onClick(View v) { confirmRemove(id, name); }
        })), Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
    }

    static void confirmRemove(final String id, String name) {
        Activity a = Overlay.activity();
        LinearLayout body = Kit.column(a);
        body.addView(Kit.body(a, Strings.t("remove_body")), Kit.fill());
        View footer = Kit.footer(a,
            Kit.button(a, Strings.t("keep"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View v) { show(INSTALLED); } }),
            Kit.button(a, Strings.t("remove"), Kit.Kind.DAN, new View.OnClickListener() {
                public void onClick(View v) {
                    call(new java.util.concurrent.Callable<String>() { public String call() { return String.valueOf(Native.remove(id)); } },
                        new Done() { public void on(String r) { show(INSTALLED); } });
                }
            }));
        Overlay.show(Overlay.onBack(Overlay.frame(Strings.t("remove_q", "mod", name), null, body, footer),
            new Runnable() { public void run() { show(INSTALLED); } }));
    }

    // ---- mod list ----------------------------------------------------------------------------------------------

    /** The newest non-yanked version with a client half, or null. */
    static JSONObject clientVersion(JSONObject entry) {
        JSONArray vs = entry.optJSONArray("versions");
        for (int i = vs == null ? -1 : vs.length() - 1; i >= 0; i--) {
            JSONObject v = vs.optJSONObject(i);
            if (v.optBoolean("yanked")) continue;
            JSONArray s = v.optJSONArray("sides");
            for (int k = 0; s != null && k < s.length(); k++) if ("client".equals(s.optString(k))) return v;
        }
        return null;
    }

    static void list() {
        final Activity a = Overlay.activity();
        content.removeAllViews();
        final EditText search = new EditText(a);
        search.setHint(Strings.t("search"));
        search.setText(query);
        search.setSingleLine(true);
        search.setTextColor(Kit.TEXT);
        search.setHintTextColor(Kit.TEXT3);
        search.setBackground(Kit.field(a));
        search.setPadding(Kit.u(a, 3), Kit.u(a, 3), Kit.u(a, 3), Kit.u(a, 3));
        content.addView(search, Kit.fill());
        final LinearLayout results = Kit.column(a);
        content.addView(results, Kit.margins(Kit.fill(), 0, Kit.u(a, 2), 0, 0));
        search.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence s, int st, int c, int af) {}
            public void onTextChanged(CharSequence s, int st, int b, int c) {}
            public void afterTextChanged(Editable e) {
                query = e.toString();
                if (index != null) render(results);
            }
        });
        if (index != null) {
            render(results);
            return;
        }
        LinearLayout saved = content;
        content = results;
        busy(Strings.t("loading_list"));
        content = saved;
        Net.POOL.execute(new Runnable() {
            public void run() {
                String err = null;
                int status = -1;
                JSONArray mods = null;
                try {
                    mods = new JSONObject(Net.text(Net.indexUrl())).getJSONArray("mods");
                } catch (Net.HttpError e) {
                    status = e.status;
                    err = e.getMessage();
                } catch (Exception e) {
                    err = String.valueOf(e);
                }
                final JSONArray m = mods;
                final String error = err;
                final int st = status;
                Bridge.ui.post(new Runnable() {
                    public void run() {
                        if (tab != LIST) return;
                        if (m != null) {
                            index = m;
                            render(results);
                            return;
                        }
                        LinearLayout saved = content;
                        content = results;
                        View retry = Kit.button(a, Strings.t("try_again"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { show(LIST); } });
                        if (st == 0) {
                            state(Strings.t("offline"), Strings.t("offline_body"), null, retry,
                                Kit.button(a, Strings.t("tab_installed"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View v) { show(INSTALLED); } }));
                        } else {
                            state(Strings.t("list_error"), Strings.t("list_error_body"), error, retry);
                        }
                        content = saved;
                    }
                });
            }
        });
    }

    static void render(LinearLayout results) {
        final Activity a = Overlay.activity();
        results.removeAllViews();
        String q = query.trim().toLowerCase(Locale.ROOT);
        int shown = 0;
        for (int i = 0; i < index.length(); i++) {
            final JSONObject e = index.optJSONObject(i);
            final JSONObject v = clientVersion(e);
            if (v == null) continue;
            String name = Strings.indexText(e, "name", e.optString("id"));
            String desc = Strings.indexText(e, "description", "");
            if (!q.isEmpty() && !(name + " " + desc + " " + e.optString("id")).toLowerCase(Locale.ROOT).contains(q)) continue;
            shown++;
            LinearLayout row = Kit.row(a);
            row.setBackground(Kit.well(a));
            row.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
            LinearLayout col = Kit.column(a);
            LinearLayout top = Kit.row(a);
            TextView n = Kit.text(a, name, Kit.TEXT, 15, true);
            Kit.ellipsize(n);
            top.addView(n, Kit.weight(1));
            JSONObject perms = v.optJSONObject("permissions");
            if (perms != null && perms.optBoolean("native")) top.addView(Kit.nativeBadge(a));
            col.addView(top, Kit.fill());
            TextView d = Kit.small(a, desc + " · " + author(e) + " · " + Kit.size(v.optLong("size")));
            Kit.ellipsize(d);
            col.addView(d, Kit.fill());
            row.addView(col, Kit.weight(1));
            row.addView(Kit.button(a, Strings.t("install"), Kit.Kind.PRI, new View.OnClickListener() {
                public void onClick(View x) { confirmIndex(e, v); }
            }), Kit.margins(Kit.wrap(), Kit.u(a, 2), 0, 0, 0));
            results.addView(row, Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 2)));
        }
        if (shown == 0 && q.isEmpty()) {
            LinearLayout saved = content;
            content = results;
            state(Strings.t("no_client_mods"), Strings.t("no_client_mods_body"), null);
            content = saved;
        } else if (shown == 0) {
            LinearLayout saved = content;
            content = results;
            state(Strings.t("no_results", "q", query), "", null,
                Kit.button(a, Strings.t("clear_search"), Kit.Kind.SEC, new View.OnClickListener() {
                    public void onClick(View v) { query = ""; show(LIST); }
                }));
            content = saved;
        }
    }

    static void confirmIndex(final JSONObject e, final JSONObject v) {
        Activity a = Overlay.activity();
        final String name = Strings.indexText(e, "name", e.optString("id"));
        LinearLayout body = Kit.column(a);
        body.addView(Kit.text(a, name + " " + v.optString("version"), Kit.TEXT, 16, true), Kit.fill());
        body.addView(Kit.small(a, author(e) + " · " + Kit.size(v.optLong("size")) + " · " + Strings.t("reviewed")), Kit.fill());
        body.addView(Kit.text(a, Strings.t("what_asks"), Kit.TEXT, 15, true), Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, Kit.u(a, 1)));
        List<String> ps = JoinFlow.perms(indexPerms(v.optJSONObject("permissions")));
        body.addView(JoinFlow.chips(a, ps), Kit.fill());
        body.addView(Kit.small(a, Strings.t("st_next")), Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
        final String url = v.optString("mirror", v.optString("url"));
        final boolean nativeCode = indexPerms(v.optJSONObject("permissions")).optBoolean("native");
        if (nativeCode) {
            // Native code: a red warning, and a button that says what the player agrees to.
            LinearLayout danger = Kit.column(a);
            danger.setBackground(Kit.danger(a));
            danger.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
            danger.addView(Kit.text(a, Strings.t("danger_title"), Kit.TEXT, 15, true), Kit.fill());
            danger.addView(Kit.body(a, Strings.t("danger_body", "mod", name)), Kit.fill());
            danger.addView(Kit.small(a, Strings.t("danger_note")), Kit.fill());
            body.addView(danger, Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
        }
        View footer = Kit.footer(a,
            Kit.button(a, Strings.t("cancel"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View x) { show(LIST); } }),
            Kit.button(a, Strings.t(nativeCode ? "install_native" : "install"), nativeCode ? Kit.Kind.DAN : Kit.Kind.PRI, new View.OnClickListener() {
                public void onClick(View x) { download(name, url, v.optString("sha256"), "index:" + url, true, nativeCode); }
            }));
        Overlay.show(Overlay.onBack(Overlay.frame(Strings.t("install_q", "mod", name), null, body, footer),
            new Runnable() { public void run() { show(LIST); } }));
    }

    /** Index permissions use mod.toml keys (`net-http`); turn them into the UI shape. */
    static JSONObject indexPerms(JSONObject p) {
        JSONObject o = new JSONObject();
        if (p == null) return o;
        try {
            o.put("world", p.optString("world", "none"));
            o.put("players", p.optJSONArray("players") == null ? new JSONArray() : p.optJSONArray("players"));
            o.put("netHttp", p.optJSONArray("net-http") == null ? new JSONArray() : p.optJSONArray("net-http"));
            o.put("channels", p.optBoolean("channels"));
            o.put("voice", p.optBoolean("voice"));
            o.put("native", p.optBoolean("native"));
        } catch (Exception ignored) {
        }
        return o;
    }

    /**
     * Downloads `url` and then installs it (`trusted`: from the index, so no second confirmation) or shows the
     * "Install from a link?" confirmation with what the package asks for.
     */
    static void download(final String name, final String url, final String sha, final String source, final boolean trusted, final boolean nativeConfirmed) {
        final Activity a = Overlay.activity();
        final AtomicBoolean cancel = new AtomicBoolean(false);
        LinearLayout body = Kit.column(a);
        final TextView status = Kit.body(a, Strings.t("downloading", "mod", name, "done", Kit.size(0), "total", "…"));
        status.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        body.addView(status, Kit.fill());
        final Overlay.Bar bar = new Overlay.Bar(a);
        bar.set(-1);
        body.addView(bar, Kit.margins(new LinearLayout.LayoutParams(-1, Kit.u(a, 6)), 0, Kit.u(a, 2), 0, Kit.u(a, 2)));
        body.addView(Kit.small(a, Strings.t("bg_note")), Kit.fill());
        View footer = Kit.footer(a,
            Kit.button(a, Strings.t("cancel"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View v) { cancel.set(true); show(tab); } }),
            Kit.button(a, Strings.t("hide"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View v) { Overlay.dismiss(); } }));
        Overlay.show(Overlay.onBack(Overlay.frame(Strings.t("installing", "mod", name), null, body, footer),
            new Runnable() { public void run() { Overlay.dismiss(); } }));
        final File dir = new File(Bridge.dataDir(), "downloads");
        dir.mkdirs();
        final File f = new File(dir, "dl-" + System.currentTimeMillis() + ".bhmod");
        Net.POOL.execute(new Runnable() {
            public void run() {
                String err = null;
                try {
                    Net.file(url, f, new Net.Progress() {
                        public void on(final long done, final long total) {
                            Bridge.ui.post(new Runnable() {
                                public void run() {
                                    status.setText(Strings.t("downloading", "mod", name, "done", Kit.size(done), "total", total > 0 ? Kit.size(total) : "…"));
                                    bar.set(total > 0 ? (float) done / total : -1);
                                }
                            });
                        }
                    }, cancel);
                } catch (Exception e) {
                    err = e.getMessage() == null ? String.valueOf(e) : e.getMessage();
                }
                if (cancel.get()) {
                    f.delete();
                    return;
                }
                final String error = err;
                String out = null;
                try {
                    if (error == null) out = trusted ? Native.install(f.getPath(), sha.isEmpty() ? null : sha, source, nativeConfirmed) : Native.inspect(f.getPath(), null);
                } catch (Throwable t) {
                    out = "{\"ok\":false,\"code\":\"loader\",\"text\":" + JSONObject.quote(String.valueOf(t)) + "}";
                }
                final String res = out;
                Bridge.ui.post(new Runnable() {
                    public void run() {
                        if (error != null) {
                            f.delete();
                            failure(Strings.t("dl_failed"), Strings.t("bad_link"), error + " · " + url);
                            return;
                        }
                        JSONObject r = parse(res);
                        if (!r.optBoolean("ok")) {
                            f.delete();
                            failure(Strings.t("install_failed"), r.optString("text"), r.optString("code") + " · " + url);
                            return;
                        }
                        if (trusted) {
                            f.delete();
                            installedDone(Strings.pick(r.optJSONObject("mod").optJSONObject("name"), name));
                        } else {
                            confirmLink(r, f, url);
                        }
                    }
                });
            }
        });
    }

    static void failure(String title, String text, final String details) {
        Activity a = Overlay.activity();
        LinearLayout body = Kit.column(a);
        body.addView(Kit.body(a, text), Kit.fill());
        TextView d = Kit.text(a, details, Kit.TEXT3, 12, false);
        d.setTypeface(android.graphics.Typeface.MONOSPACE);
        body.addView(d, Kit.margins(Kit.fill(), 0, Kit.u(a, 2), 0, 0));
        View footer = Kit.footer(a,
            Kit.button(a, Strings.t("copy_details"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View v) { Overlay.copy(details); } }),
            Kit.button(a, Strings.t("back"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { show(tab); } }));
        Overlay.show(Overlay.onBack(Overlay.frame(title, null, body, footer), new Runnable() { public void run() { show(tab); } }));
    }

    static void installedDone(String name) {
        Activity a = Overlay.activity();
        installedCount = -1;
        LinearLayout body = Kit.column(a);
        body.addView(Kit.body(a, Strings.t("installed_body", "mod", name)), Kit.fill());
        View footer = Kit.footer(a, Kit.button(a, Strings.t("done"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { show(INSTALLED); } }));
        Overlay.show(Overlay.onBack(Overlay.frame(Strings.t("installed_title"), null, body, footer), new Runnable() { public void run() { show(INSTALLED); } }));
    }

    // ---- from link ---------------------------------------------------------------------------------------------

    static void link(String initial) {
        final Activity a = Overlay.activity();
        content.removeAllViews();
        content.addView(Kit.text(a, Strings.t("link_note"), Kit.TEXT, 15, true), Kit.fill());
        content.addView(Kit.small(a, Strings.t("link_note2")), Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 3)));
        final EditText url = new EditText(a);
        url.setHint("https://…/mod.bhmod");
        url.setText(initial);
        url.setSingleLine(true);
        url.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        url.setTextColor(Kit.TEXT);
        url.setHintTextColor(Kit.TEXT3);
        url.setBackground(Kit.field(a));
        url.setPadding(Kit.u(a, 3), Kit.u(a, 3), Kit.u(a, 3), Kit.u(a, 3));
        content.addView(url, Kit.fill());
        content.addView(Kit.footer(a,
            Kit.button(a, Strings.t("paste"), Kit.Kind.SEC, new View.OnClickListener() {
                public void onClick(View v) {
                    ClipboardManager cm = (ClipboardManager) a.getSystemService(Context.CLIPBOARD_SERVICE);
                    if (cm != null && cm.hasPrimaryClip() && cm.getPrimaryClip().getItemCount() > 0) {
                        CharSequence t = cm.getPrimaryClip().getItemAt(0).coerceToText(a);
                        if (t != null) url.setText(t.toString().trim());
                    }
                }
            }),
            Kit.button(a, Strings.t("check_link"), Kit.Kind.PRI, new View.OnClickListener() {
                public void onClick(View v) {
                    String u = url.getText().toString().trim();
                    if (!u.startsWith("https://") && !u.startsWith("http://")) {
                        url.setError(Strings.t("bad_link"));
                        return;
                    }
                    download(u.substring(u.lastIndexOf('/') + 1), u, "", u, false, false);
                }
            })), Kit.margins(Kit.fill(), 0, Kit.u(a, 2), 0, 0));
    }

    static void confirmLink(JSONObject r, final File f, final String url) {
        Activity a = Overlay.activity();
        JSONObject m = r.optJSONObject("mod");
        final String sha = r.optString("sha256");
        String name = Strings.pick(m.optJSONObject("name"), m.optString("id"));
        LinearLayout body = Kit.column(a);
        LinearLayout warn = Kit.column(a);
        warn.setBackground(Kit.well(a));
        warn.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
        warn.addView(Kit.text(a, Strings.t("link_warn"), Kit.SUN, 15, true), Kit.fill());
        warn.addView(Kit.small(a, Strings.t("link_warn2")), Kit.fill());
        body.addView(warn, Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 3)));
        body.addView(Kit.text(a, name + " " + m.optString("version"), Kit.TEXT, 16, true), Kit.fill());
        body.addView(Kit.small(a, author(m) + " · " + Strings.t("from_link") + " · " + Kit.size(r.optLong("size")) + " · " + Strings.t("not_reviewed")), Kit.fill());
        TextView u = Kit.small(a, url);
        Kit.ellipsize(u);
        body.addView(u, Kit.fill());
        if (!m.optBoolean("client")) body.addView(Kit.small(a, Strings.t("server_only_note")), Kit.fill());
        body.addView(Kit.text(a, Strings.t("what_asks"), Kit.TEXT, 15, true), Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, Kit.u(a, 1)));
        body.addView(JoinFlow.chips(a, JoinFlow.perms(m.optJSONObject("permissions"))), Kit.fill());
        final String finalName = name;
        View footer = Kit.footer(a,
            Kit.button(a, Strings.t("cancel"), Kit.Kind.SEC, new View.OnClickListener() { public void onClick(View v) { f.delete(); show(LINK); } }),
            Kit.button(a, Strings.t("install_anyway"), Kit.Kind.PRI, new View.OnClickListener() {
                public void onClick(View v) {
                    call(new java.util.concurrent.Callable<String>() { public String call() { return Native.install(f.getPath(), sha, url, false); } }, new Done() {
                        public void on(String json) {
                            f.delete();
                            JSONObject res = parse(json);
                            if (res.optBoolean("ok")) installedDone(finalName);
                            else failure(Strings.t("install_failed"), res.optString("text"), res.optString("code") + " · " + url);
                        }
                    });
                }
            }));
        Overlay.show(Overlay.onBack(Overlay.frame(Strings.t("link_q"), null, body, footer), new Runnable() { public void run() { f.delete(); show(LINK); } }));
    }

    static JSONObject parse(String json) {
        try {
            return new JSONObject(json == null ? "{}" : json);
        } catch (Exception e) {
            JSONObject o = new JSONObject();
            try {
                o.put("ok", false);
                o.put("text", String.valueOf(json));
            } catch (Exception ignored) {
            }
            return o;
        }
    }
}
