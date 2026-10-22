// Blockheads Club mod loader: joining a server that uses mods (IG-Join). Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.view.View;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/** The join prompt, download progress and the error screens. Nothing downloads before "Download and join". */
final class JoinFlow {
    static String server = "";
    static JSONArray mods = new JSONArray();
    static long total;
    static TextView status;
    static Overlay.Bar bar;
    static final List<TextView> rowState = new ArrayList<>();
    static final List<String> rowIds = new ArrayList<>();
    static boolean active;

    private JoinFlow() {}

    static void command(String t, JSONObject c) {
        switch (t) {
            case "prompt": prompt(c); break;
            case "progress": progress(c); break;
            case "joined":
            case "closed":
                if (active) Overlay.dismiss();
                active = false;
                break;
            case "failed": failed(c); break;
            case "reject": reject(c); break;
            default: break;
        }
    }

    static String sides(JSONArray s) {
        boolean c = false, sv = false;
        for (int i = 0; s != null && i < s.length(); i++) {
            c |= "client".equals(s.optString(i));
            sv |= "server".equals(s.optString(i));
        }
        return Strings.t(c && sv ? "side_both" : c ? "side_client" : "side_server");
    }

    /** Permission chips for a `{world, players, netHttp, channels, native}` object. */
    static List<String> perms(JSONObject p) {
        List<String> out = new ArrayList<>();
        if (p == null) p = new JSONObject();
        String w = p.optString("world", "none");
        if ("read".equals(w)) out.add(Strings.t("perm_world_read"));
        if ("write".equals(w)) out.add(Strings.t("perm_world_write"));
        JSONArray pl = p.optJSONArray("players");
        for (int i = 0; pl != null && i < pl.length(); i++) {
            out.add("chat".equals(pl.optString(i)) ? Strings.t("perm_chat") : Strings.t("perm_players") + " (" + pl.optString(i) + ")");
        }
        JSONArray net = p.optJSONArray("netHttp");
        if (net != null && net.length() > 0) out.add(Strings.t("perm_net"));
        if (p.optBoolean("channels")) out.add(Strings.t("perm_channels"));
        if (p.optBoolean("native")) out.add(Strings.t("perm_native"));
        if (out.isEmpty()) out.add(Strings.t("perm_none"));
        return out;
    }

    static View chips(Activity a, List<String> labels) {
        LinearLayout l = Kit.row(a);
        for (String s : labels) {
            TextView b = Kit.badge(a, s, Kit.TEXT2, Kit.well(a));
            b.setTypeface(Kit.body);
            l.addView(b, Kit.margins(Kit.wrap(), 0, Kit.u(a, 1), Kit.u(a, 2), 0));
        }
        android.widget.HorizontalScrollView h = new android.widget.HorizontalScrollView(a);
        h.setHorizontalScrollBarEnabled(false);
        h.addView(l);
        return h;
    }

    static View modRow(Activity a, JSONObject m, String detail, TextView state) {
        LinearLayout r = Kit.column(a);
        r.setBackground(Kit.well(a));
        r.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
        LinearLayout top = Kit.row(a);
        TextView name = Kit.text(a, Strings.pick(m.optJSONObject("name"), m.optString("id")), Kit.TEXT, 15, true);
        Kit.ellipsize(name);
        top.addView(name, Kit.weight(1));
        if (m.optJSONObject("permissions") != null && m.optJSONObject("permissions").optBoolean("native")) top.addView(Kit.nativeBadge(a));
        if (state != null) top.addView(state);
        r.addView(top, Kit.fill());
        r.addView(Kit.small(a, detail), Kit.fill());
        return r;
    }

    static void prompt(JSONObject c) {
        Activity a = Overlay.activity();
        if (a == null) {
            Native.ok(new Runnable() { public void run() { Native.decide(false); } });
            return;
        }
        active = true;
        server = c.optString("server");
        mods = c.optJSONArray("mods");
        final boolean required = c.optBoolean("required");
        final long download = c.optLong("download");
        LinearLayout body = Kit.column(a);
        int clientMods = 0;
        List<String> have = new ArrayList<>();
        long all = 0;
        for (int i = 0; i < mods.length(); i++) {
            JSONObject m = mods.optJSONObject(i);
            if (!m.optBoolean("client")) continue;
            clientMods++;
            all += m.optLong("size");
            if (m.optBoolean("cached")) have.add(Strings.pick(m.optJSONObject("name"), m.optString("id")) + " " + m.optString("version"));
        }
        body.addView(Kit.body(a, Strings.t(required ? "join_body" : "join_optional", "server", server)), Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 3)));
        for (int i = 0; i < mods.length(); i++) {
            JSONObject m = mods.optJSONObject(i);
            if (!m.optBoolean("client")) continue;
            String detail = m.optString("version") + " · " + sides(m.optJSONArray("sides")) + " · " + Kit.size(m.optLong("size"));
            View row = modRow(a, m, detail, null);
            ((LinearLayout) row).addView(chips(a, perms(m.optJSONObject("permissions"))));
            body.addView(row, Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 2)));
        }
        String foot = Strings.t("total", "size", Kit.size(all));
        if (!have.isEmpty()) foot = Strings.t("you_have", "mods", android.text.TextUtils.join(", ", have)) + " · " + foot;
        body.addView(Kit.small(a, foot), Kit.fill());
        View.OnClickListener no = new View.OnClickListener() {
            public void onClick(View v) {
                Native.ok(new Runnable() { public void run() { Native.decide(false); } });
                if (required) Overlay.dismiss();
            }
        };
        View.OnClickListener yes = new View.OnClickListener() {
            public void onClick(View v) {
                Native.ok(new Runnable() { public void run() { Native.decide(true); } });
                showProgress();
            }
        };
        String go = download > 0 ? Strings.t("download_join", "size", Kit.size(download)) : Strings.t("join");
        View footer = Kit.footer(a, Kit.button(a, Strings.t(required ? "cancel" : "join_without"), Kit.Kind.SEC, no),
            Kit.button(a, go, Kit.Kind.PRI, yes));
        View panel = Overlay.frame(Strings.t(clientMods == 1 ? "join_title_one" : "join_title", "server", server, "n", clientMods), null, body, footer);
        Overlay.show(Overlay.onBack(panel, new Runnable() {
            public void run() {
                Native.ok(new Runnable() { public void run() { Native.cancel(); } });
                Overlay.dismiss();
            }
        }));
        total = download;
    }

    static void showProgress() {
        Activity a = Overlay.activity();
        if (a == null) return;
        LinearLayout body = Kit.column(a);
        status = Kit.body(a, Strings.t("getting", "server", server, "done", Kit.size(0), "total", Kit.size(total)));
        status.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        body.addView(status, Kit.fill());
        bar = new Overlay.Bar(a);
        bar.set(total > 0 ? 0 : -1);
        body.addView(bar, Kit.margins(new LinearLayout.LayoutParams(-1, Kit.u(a, 6)), 0, Kit.u(a, 2), 0, Kit.u(a, 3)));
        rowState.clear();
        rowIds.clear();
        for (int i = 0; mods != null && i < mods.length(); i++) {
            JSONObject m = mods.optJSONObject(i);
            if (!m.optBoolean("client")) continue;
            TextView st = Kit.badge(a, Strings.t(m.optBoolean("cached") ? "checked" : "waiting"), Kit.TEXT2, Kit.well(a));
            rowState.add(st);
            rowIds.add(m.optString("id"));
            body.addView(modRow(a, m, m.optString("version") + " · " + Kit.size(m.optLong("size")), st), Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 2)));
        }
        body.addView(Kit.small(a, Strings.t("responsive")), Kit.fill());
        View.OnClickListener cancel = new View.OnClickListener() {
            public void onClick(View v) {
                Native.ok(new Runnable() { public void run() { Native.cancel(); } });
                Overlay.dismiss();
                active = false;
            }
        };
        View panel = Overlay.frame(Strings.t("joining_title", "server", server), null, body, Kit.footer(a, Kit.button(a, Strings.t("cancel"), Kit.Kind.SEC, cancel)));
        Overlay.show(Overlay.onBack(panel, new Runnable() { public void run() { cancel_(); } }));
        active = true;
    }

    private static void cancel_() {
        Native.ok(new Runnable() { public void run() { Native.cancel(); } });
        Overlay.dismiss();
        active = false;
    }

    static long doneBefore(int index) {
        long d = 0;
        int k = 0;
        for (int i = 0; mods != null && i < mods.length(); i++) {
            JSONObject m = mods.optJSONObject(i);
            if (!m.optBoolean("client") || m.optBoolean("cached")) continue;
            if (k++ < index) d += m.optLong("size");
        }
        return d;
    }

    static void progress(JSONObject c) {
        if (status == null || bar == null) return;
        long done = doneBefore(c.optInt("index")) + c.optLong("done");
        status.setText(Strings.t("getting", "server", server, "done", Kit.size(done), "total", Kit.size(total)));
        bar.set(total > 0 ? (float) done / total : -1);
        int i = rowIds.indexOf(c.optString("id"));
        if (i >= 0) {
            boolean fin = c.optLong("done") >= c.optLong("size");
            rowState.get(i).setText(fin ? Strings.t("checked") : Kit.size(c.optLong("done")) + " / " + Kit.size(c.optLong("size")));
        }
    }

    static String modName(String id) {
        for (int i = 0; mods != null && i < mods.length(); i++) {
            JSONObject m = mods.optJSONObject(i);
            if (id.equals(m.optString("id"))) return Strings.pick(m.optJSONObject("name"), id) + " " + m.optString("version");
        }
        return id;
    }

    static void failed(JSONObject c) {
        final Activity a = Overlay.activity();
        if (a == null) return;
        active = false;
        String code = c.optString("code"), id = c.optString("id", ""), refusal = c.optString("refusal", "");
        final String details = c.optString("text") + " · " + code + (id.isEmpty() ? "" : " · " + id) + " · " + c.optString("server");
        String title, headline, text;
        LinearLayout body = Kit.column(a);
        boolean update = "needs-newer-loader".equals(refusal) || "needs-newer-api".equals(refusal);
        if (update) {
            title = Strings.t("update_title");
            headline = Strings.t("needs_loader", "server", server, "v", c.optString("needs"));
            text = Strings.t("you_have_loader", "v", c.optString("loader")) + "\n" + Strings.t("update_body");
        } else if ("download".equals(code) || "stalled".equals(code)) {
            title = Strings.t("dl_failed");
            headline = Strings.t("didnt_download", "mod", modName(id));
            text = Strings.t("dl_failed_body");
        } else if ("hash-mismatch".equals(code)) {
            title = Strings.t("check_failed");
            headline = Strings.t("didnt_pass", "mod", modName(id));
            text = Strings.t("hash_body", "server", server);
        } else if ("refused".equals(code)) {
            title = Strings.t("check_failed");
            headline = Strings.t("didnt_pass", "mod", modName(id));
            text = Strings.t("refused_body", "why", c.optString("text"));
        } else if ("declined".equals(code)) {
            title = Strings.t("cant_join", "server", server);
            headline = null;
            text = Strings.t("declined", "server", server);
        } else {
            title = Strings.t("cant_join", "server", server);
            headline = null;
            text = Strings.t("load-failed".equals(code) ? "load_failed" : "protocol");
        }
        if (headline != null) body.addView(Kit.text(a, headline, Kit.TEXT, 16, true), Kit.margins(Kit.fill(), 0, 0, 0, Kit.u(a, 2)));
        body.addView(Kit.body(a, text), Kit.fill());
        TextView code_ = Kit.text(a, details, Kit.TEXT3, 12, false);
        code_.setTypeface(android.graphics.Typeface.MONOSPACE);
        code_.setBackground(Kit.field(a));
        code_.setPadding(Kit.u(a, 3), Kit.u(a, 2), Kit.u(a, 3), Kit.u(a, 2));
        body.addView(code_, Kit.margins(Kit.fill(), 0, Kit.u(a, 3), 0, 0));
        View.OnClickListener close = new View.OnClickListener() { public void onClick(View v) { Overlay.dismiss(); } };
        View footer = update
            ? Kit.footer(a, Kit.button(a, Strings.t("back_menu"), Kit.Kind.SEC, close),
                Kit.button(a, Strings.t("how_update"), Kit.Kind.PRI, new View.OnClickListener() {
                    public void onClick(View v) { open(a, "https://blockheads.club/patcher"); }
                }))
            : Kit.footer(a, Kit.button(a, Strings.t("copy_details"), Kit.Kind.SEC, new View.OnClickListener() {
                    public void onClick(View v) { Overlay.copy(details); }
                }), Kit.button(a, Strings.t("back_menu"), Kit.Kind.PRI, close));
        Overlay.show(Overlay.onBack(Overlay.frame(title, null, body, footer), new Runnable() { public void run() { Overlay.dismiss(); } }));
    }

    static void reject(JSONObject c) {
        final Activity a = Overlay.activity();
        if (a == null) return;
        active = false;
        LinearLayout body = Kit.column(a);
        body.addView(Kit.body(a, c.optString("text")), Kit.fill());
        final String url = c.isNull("url") ? "" : c.optString("url", "");
        View.OnClickListener close = new View.OnClickListener() { public void onClick(View v) { Overlay.dismiss(); } };
        View footer = url.isEmpty()
            ? Kit.footer(a, Kit.button(a, Strings.t("back_menu"), Kit.Kind.PRI, close))
            : Kit.footer(a, Kit.button(a, Strings.t("back_menu"), Kit.Kind.SEC, close),
                Kit.button(a, Strings.t("how_update"), Kit.Kind.PRI, new View.OnClickListener() { public void onClick(View v) { open(a, url); } }));
        Overlay.show(Overlay.onBack(Overlay.frame(Strings.t("cant_join", "server", c.optString("server")), null, body, footer),
            new Runnable() { public void run() { Overlay.dismiss(); } }));
    }

    static void open(Activity a, String url) {
        try {
            a.startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
        } catch (Exception e) {
            Overlay.copy(url);
        }
    }
}
