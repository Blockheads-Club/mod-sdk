// Blockheads Club mod loader: the kit fonts (IG-Kit). Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.content.Context;
import android.graphics.Typeface;
import android.util.Log;

import java.io.File;
import java.io.FileOutputStream;

/**
 * Tiny5 for titles, buttons, tabs and HUD numbers, Atkinson Hyperlegible Next for reading text (Golos Text for
 * Russian and Kazakh). The font files are embedded in libbhmodloader.so ({@code Native.font}); they are written
 * once to the app's cache and loaded from there. Until the library is loaded (or if anything fails) the kit keeps
 * the system faces, so the UI never depends on the fonts.
 */
final class Fonts {
    static final int TINY5 = 0, ATKINSON = 1, GOLOS = 2;
    private static boolean loaded;

    private Fonts() {}

    /** Loads the fonts once (true when this call or an earlier one succeeded); sets {@link Kit#title} and {@link Kit#body}. */
    static synchronized boolean load(Context c) {
        if (loaded) return true;
        try {
            int lang = Strings.lang();
            Typeface title = face(c, TINY5, "Tiny5-Regular.ttf");
            Typeface body = face(c, lang >= 2 ? GOLOS : ATKINSON, lang >= 2 ? "GolosText-Regular.ttf" : "AtkinsonHyperlegibleNext-Regular.ttf");
            if (title == null || body == null) return false;
            Kit.title = title;
            Kit.body = body;
            loaded = true;
        } catch (Throwable t) {
            Log.w(Bridge.TAG, "fonts not loaded", t);
        }
        return loaded;
    }

    private static Typeface face(Context c, int id, String name) throws Exception {
        byte[] bytes;
        try {
            bytes = Native.font(id);
        } catch (UnsatisfiedLinkError e) {
            return null;
        }
        if (bytes == null || bytes.length == 0) return null;
        File dir = new File(c.getCacheDir(), "bhmods-fonts");
        dir.mkdirs();
        File f = new File(dir, name);
        if (!f.exists() || f.length() != bytes.length) {
            File tmp = new File(dir, name + ".tmp");
            try (FileOutputStream o = new FileOutputStream(tmp)) {
                o.write(bytes);
            }
            if (!tmp.renameTo(f)) throw new java.io.IOException("cannot move " + tmp);
        }
        return Typeface.createFromFile(f);
    }
}
