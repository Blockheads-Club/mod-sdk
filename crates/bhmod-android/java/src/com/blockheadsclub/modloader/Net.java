// Blockheads Club mod loader: HTTP off the UI and game threads. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.Properties;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Downloads on one background thread. Results come back on the UI thread. */
final class Net {
    static final String INDEX = "https://blockheads.club/mods/index.json";
    static final long MAX = 32L * 1024 * 1024;
    static final ExecutorService POOL = Executors.newSingleThreadExecutor();

    private Net() {}

    interface Progress { void on(long done, long total); }

    /** An HTTP failure with its status (0 when offline / no answer). */
    static final class HttpError extends IOException {
        final int status;
        HttpError(int status, String msg) { super(msg); this.status = status; }
    }

    /**
     * The index URL: the public index, or `index=` from `dev.properties` in the loader's data directory or
     * `bhmods-dev.properties` in the app's external files directory (for testing against a local index).
     */
    static String indexUrl() {
        for (File f : new File[] {new File(Bridge.dataDir(), "dev.properties"), new File(Bridge.app.getExternalFilesDir(null), "bhmods-dev.properties")}) {
            String u = readIndex(f);
            if (u != null) return u;
        }
        return INDEX;
    }

    private static String readIndex(File f) {
        try {
            if (f.isFile()) {
                Properties p = new Properties();
                InputStream in = new FileInputStream(f);
                try { p.load(in); } finally { in.close(); }
                String u = p.getProperty("index", "").trim();
                if (!u.isEmpty()) return u;
            }
        } catch (Exception e) {
            android.util.Log.w(Bridge.TAG, f + " not read", e);
        }
        return null;
    }

    static HttpURLConnection open(String url) throws IOException {
        String u = url;
        for (int i = 0; i < 5; i++) {
            HttpURLConnection c = (HttpURLConnection) new URL(u).openConnection();
            c.setConnectTimeout(10000);
            c.setReadTimeout(10000);
            c.setInstanceFollowRedirects(false);
            c.setRequestProperty("User-Agent", "BlockheadsModLoader/" + Overlay.version + " (Android)");
            int s;
            try {
                s = c.getResponseCode();
            } catch (IOException e) {
                throw new HttpError(0, e.toString());
            }
            if (s >= 300 && s < 400 && c.getHeaderField("Location") != null) {
                u = new URL(new URL(u), c.getHeaderField("Location")).toString();
                c.disconnect();
                continue;
            }
            if (s != 200) {
                c.disconnect();
                throw new HttpError(s, "HTTP " + s + " · " + new URL(u).getHost());
            }
            return c;
        }
        throw new HttpError(0, "too many redirects");
    }

    static String text(String url) throws IOException {
        HttpURLConnection c = open(url);
        try {
            InputStream in = c.getInputStream();
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] b = new byte[16384];
            int n;
            while ((n = in.read(b)) > 0) {
                out.write(b, 0, n);
                if (out.size() > 8 * 1024 * 1024) throw new IOException("index too large");
            }
            return out.toString("UTF-8");
        } finally {
            c.disconnect();
        }
    }

    /** Downloads to `dest` (at most 32 MiB); progress at most 10 times a second. */
    static void file(String url, File dest, Progress p, java.util.concurrent.atomic.AtomicBoolean cancel) throws IOException {
        HttpURLConnection c = open(url);
        try {
            long total = c.getContentLength();
            if (total > MAX) throw new IOException("file too large");
            InputStream in = c.getInputStream();
            OutputStream out = new FileOutputStream(dest);
            try {
                byte[] b = new byte[32768];
                long done = 0, last = 0;
                int n;
                while ((n = in.read(b)) > 0) {
                    if (cancel != null && cancel.get()) throw new IOException("cancelled");
                    out.write(b, 0, n);
                    done += n;
                    if (done > MAX) throw new IOException("file too large");
                    long now = System.currentTimeMillis();
                    if (now - last >= 100) {
                        last = now;
                        p.on(done, total);
                    }
                }
                p.on(done, total);
            } finally {
                out.close();
            }
        } finally {
            c.disconnect();
        }
    }
}
