// Blockheads Club mod loader: in-game UI kit (IG-Kit). Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.ColorFilter;
import android.graphics.Paint;
import android.graphics.PixelFormat;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.StateListDrawable;
import android.text.TextUtils;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

/**
 * Our own pixel parts, drawn in code: chunky 2-step borders, raised buttons with a 2 u bottom edge, warm dark panels
 * and an orange title strip. One unit u is 2 dp on phones. Colours are the kit tokens.
 */
final class Kit {
    static final int INK = 0xFF17110D, PANEL = 0xFF3D3029, PANEL_HI = 0xFF5E4A3C, PANEL_LO = 0xFF2A201B;
    static final int WELL = 0xFF2A221D, FIELD = 0xFF1F1814, BANNER = 0xFFBF4F1C, SUN = 0xFFFFD27A;
    static final int DANGER = 0xFFD23B26, STRIPE_A = 0xFFC8351F, STRIPE_B = 0xFF8E2516, OK = 0xFF2F8F57;
    static final int TEXT = 0xFFFBF3E6, TEXT2 = 0xFFDCCDBB, TEXT3 = 0xFFBBA894;

    /** Titles, buttons, tabs: a pixel-like face (Tiny5 when bundled; the system monospace bold otherwise). */
    static Typeface title = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD);
    static Typeface body = Typeface.DEFAULT;

    private Kit() {}

    static int u(Context c, float units) {
        return Math.round(TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, units * 2, c.getResources().getDisplayMetrics()));
    }

    /** A 9-slice-like pixel box: ink outline, light top/left, shade bottom/right, fill; optional raised bottom edge. */
    static final class Box extends Drawable {
        final int fill, hi, lo, px, edge;
        final boolean stripes;
        private final Paint p = new Paint();

        Box(int fill, int hi, int lo, int px, int edge, boolean stripes) {
            this.fill = fill; this.hi = hi; this.lo = lo; this.px = px; this.edge = edge; this.stripes = stripes;
        }

        @Override public void draw(Canvas c) {
            android.graphics.Rect b = getBounds();
            int l = b.left, t = b.top, r = b.right, bo = b.bottom;
            p.setColor(INK);
            c.drawRect(l + px, t, r - px, bo, p);
            c.drawRect(l, t + px, r, bo - px, p);
            int il = l + px, it = t + px, ir = r - px, ib = bo - px - edge;
            p.setColor(fill);
            c.drawRect(il, it, ir, ib, p);
            if (stripes) {
                p.setColor(STRIPE_B);
                int s = px * 4;
                c.save();
                c.clipRect(il, it, ir, ib);
                for (int x = il - (ib - it); x < ir; x += s * 2) {
                    android.graphics.Path path = new android.graphics.Path();
                    path.moveTo(x, ib); path.lineTo(x + s, ib); path.lineTo(x + s + (ib - it), it); path.lineTo(x + (ib - it), it); path.close();
                    c.drawPath(path, p);
                }
                c.restore();
            }
            p.setColor(hi);
            c.drawRect(il, it, ir, it + px, p);
            c.drawRect(il, it, il + px, ib, p);
            p.setColor(lo);
            c.drawRect(il, ib - px, ir, ib, p);
            c.drawRect(ir - px, it, ir, ib, p);
            if (edge > 0) {
                p.setColor(lo);
                c.drawRect(il, ib, ir, bo - px, p);
            }
        }
        @Override public void setAlpha(int a) {}
        @Override public void setColorFilter(ColorFilter cf) {}
        @Override public int getOpacity() { return PixelFormat.TRANSLUCENT; }
    }

    static Drawable panel(Context c) { return new Box(PANEL, PANEL_HI, PANEL_LO, u(c, 1), 0, false); }
    static Drawable well(Context c) { return new Box(WELL, PANEL_LO, PANEL, u(c, 1), 0, false); }
    static Drawable field(Context c) { return new Box(FIELD, INK, PANEL_LO, u(c, 1), 0, false); }
    static Drawable banner(Context c) { return new Box(BANNER, 0xFFD9682F, 0xFF8F3A14, u(c, 1), 0, false); }
    static Drawable danger(Context c) { return new Box(STRIPE_A, 0xFFE0563D, STRIPE_B, u(c, 1), 0, true); }

    enum Kind { PRI, SEC, DAN }

    static Drawable button(Context c, Kind k) {
        int fill = k == Kind.PRI ? SUN : k == Kind.DAN ? DANGER : PANEL_HI;
        int hi = k == Kind.PRI ? 0xFFFFE6B0 : k == Kind.DAN ? 0xFFE8604A : 0xFF7A6250;
        int lo = k == Kind.PRI ? 0xFFC99A45 : k == Kind.DAN ? 0xFF8E2516 : PANEL_LO;
        StateListDrawable s = new StateListDrawable();
        s.addState(new int[] {-android.R.attr.state_enabled}, new Box(0xFF4A3D34, 0xFF55473C, 0xFF3A2F28, u(c, 1), 0, false));
        s.addState(new int[] {android.R.attr.state_pressed}, new Box(fill, lo, hi, u(c, 1), 0, false));
        s.addState(new int[] {android.R.attr.state_focused}, new Box(fill, SUN, lo, u(c, 1), u(c, 2), false));
        s.addState(new int[] {}, new Box(fill, hi, lo, u(c, 1), u(c, 2), false));
        return s;
    }

    static TextView text(Context c, CharSequence s, int color, float sp, boolean titleFace) {
        TextView t = new TextView(c);
        t.setText(s);
        t.setTextColor(color);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, sp);
        t.setTypeface(titleFace ? title : body);
        t.setShadowLayer(0.01f, 0, u(c, 0.5f), INK);
        return t;
    }

    static TextView title(Context c, CharSequence s) { return text(c, s, TEXT, 18, true); }
    static TextView body(Context c, CharSequence s) { return text(c, s, TEXT, 15, false); }
    static TextView small(Context c, CharSequence s) { return text(c, s, TEXT2, 13, false); }

    static Button button(Context c, CharSequence label, Kind k, View.OnClickListener l) {
        Button b = new Button(c);
        b.setText(label);
        b.setAllCaps(false);
        b.setTypeface(title);
        b.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
        b.setTextColor(k == Kind.PRI ? INK : TEXT);
        b.setBackground(button(c, k));
        b.setMinHeight(u(c, 24));
        b.setMinimumHeight(u(c, 24));
        b.setMinWidth(u(c, 24));
        b.setPadding(u(c, 5), u(c, 2), u(c, 5), u(c, 4));
        b.setStateListAnimator(null);
        b.setOnClickListener(l);
        return b;
    }

    /** A small label with a background: status words, trust badges, "NATIVE". */
    static TextView badge(Context c, CharSequence s, int fg, Drawable bg) {
        TextView t = text(c, s, fg, 11, true);
        t.setBackground(bg);
        t.setPadding(u(c, 3), u(c, 1), u(c, 3), u(c, 1));
        t.setSingleLine(true);
        return t;
    }

    static TextView nativeBadge(Context c) { return badge(c, Strings.t("native").toUpperCase(), TEXT, danger(c)); }

    static LinearLayout row(Context c) {
        LinearLayout l = new LinearLayout(c);
        l.setOrientation(LinearLayout.HORIZONTAL);
        l.setGravity(Gravity.CENTER_VERTICAL);
        return l;
    }

    static LinearLayout column(Context c) {
        LinearLayout l = new LinearLayout(c);
        l.setOrientation(LinearLayout.VERTICAL);
        return l;
    }

    static LinearLayout.LayoutParams wrap() {
        return new LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }

    static LinearLayout.LayoutParams fill() {
        return new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }

    static LinearLayout.LayoutParams weight(float w) {
        return new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, w);
    }

    static LinearLayout.LayoutParams margins(LinearLayout.LayoutParams p, int l, int t, int r, int b) {
        p.setMargins(l, t, r, b);
        return p;
    }

    static void ellipsize(TextView t) {
        t.setSingleLine(true);
        t.setEllipsize(TextUtils.TruncateAt.END);
    }

    /** Footer of a dialog: buttons right-aligned, wrapping to a new line when they don't fit (FlowLayout-like). */
    static LinearLayout footer(Context c, View... buttons) {
        LinearLayout l = row(c);
        l.setGravity(Gravity.END | Gravity.CENTER_VERTICAL);
        for (View b : buttons) {
            l.addView(b, margins(wrap(), u(c, 3), u(c, 2), 0, 0));
        }
        return l;
    }

    static String size(long bytes) {
        java.text.NumberFormat f = java.text.NumberFormat.getNumberInstance();
        f.setMaximumFractionDigits(1);
        if (bytes < 1024) return f.format(bytes) + " B";
        if (bytes < 1024 * 1024) return f.format(Math.max(1, Math.round(bytes / 1024.0))) + " KB";
        return f.format(bytes / (1024.0 * 1024.0)) + " MB";
    }
}
