// Blockheads Club mod loader: hardware keyboard keys for mod key bindings (API 0.3.4 `input`).
// Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.view.KeyEvent;
import android.view.View;
import android.view.Window;
import android.widget.EditText;

/**
 * Key names of the `input` interface for Android key codes, and the forwarding of hardware key presses to the
 * loader. Phones mostly have no keyboard; with one (Bluetooth, DeX, a Chromebook) the game activity's window
 * callback is wrapped so every key down (no repeat) and up goes to {@code Native.key} while no text field has focus.
 * The game still gets every key.
 */
final class Keys {
    private Keys() {}

    /** The `input` key name of an Android key code, null for keys the interface has no name for. */
    static String name(int code) {
        if (code >= KeyEvent.KEYCODE_A && code <= KeyEvent.KEYCODE_Z) return String.valueOf((char) ('a' + code - KeyEvent.KEYCODE_A));
        if (code >= KeyEvent.KEYCODE_0 && code <= KeyEvent.KEYCODE_9) return String.valueOf((char) ('0' + code - KeyEvent.KEYCODE_0));
        if (code >= KeyEvent.KEYCODE_F1 && code <= KeyEvent.KEYCODE_F12) return "f" + (code - KeyEvent.KEYCODE_F1 + 1);
        if (code >= KeyEvent.KEYCODE_NUMPAD_0 && code <= KeyEvent.KEYCODE_NUMPAD_9) return "keypad-" + (code - KeyEvent.KEYCODE_NUMPAD_0);
        switch (code) {
            case KeyEvent.KEYCODE_SPACE: return "space";
            case KeyEvent.KEYCODE_TAB: return "tab";
            case KeyEvent.KEYCODE_ENTER: return "enter";
            case KeyEvent.KEYCODE_DEL: return "backspace";
            case KeyEvent.KEYCODE_INSERT: return "insert";
            case KeyEvent.KEYCODE_FORWARD_DEL: return "delete";
            case KeyEvent.KEYCODE_MOVE_HOME: return "home";
            case KeyEvent.KEYCODE_MOVE_END: return "end";
            case KeyEvent.KEYCODE_PAGE_UP: return "page-up";
            case KeyEvent.KEYCODE_PAGE_DOWN: return "page-down";
            case KeyEvent.KEYCODE_DPAD_UP: return "up";
            case KeyEvent.KEYCODE_DPAD_DOWN: return "down";
            case KeyEvent.KEYCODE_DPAD_LEFT: return "left";
            case KeyEvent.KEYCODE_DPAD_RIGHT: return "right";
            case KeyEvent.KEYCODE_SHIFT_LEFT: return "left-shift";
            case KeyEvent.KEYCODE_SHIFT_RIGHT: return "right-shift";
            case KeyEvent.KEYCODE_CTRL_LEFT: return "left-ctrl";
            case KeyEvent.KEYCODE_CTRL_RIGHT: return "right-ctrl";
            case KeyEvent.KEYCODE_ALT_LEFT: return "left-alt";
            case KeyEvent.KEYCODE_ALT_RIGHT: return "right-alt";
            case KeyEvent.KEYCODE_GRAVE: return "grave";
            case KeyEvent.KEYCODE_MINUS: return "minus";
            case KeyEvent.KEYCODE_EQUALS: return "equals";
            case KeyEvent.KEYCODE_LEFT_BRACKET: return "left-bracket";
            case KeyEvent.KEYCODE_RIGHT_BRACKET: return "right-bracket";
            case KeyEvent.KEYCODE_BACKSLASH: return "backslash";
            case KeyEvent.KEYCODE_SEMICOLON: return "semicolon";
            case KeyEvent.KEYCODE_APOSTROPHE: return "apostrophe";
            case KeyEvent.KEYCODE_COMMA: return "comma";
            case KeyEvent.KEYCODE_PERIOD: return "period";
            case KeyEvent.KEYCODE_SLASH: return "slash";
            case KeyEvent.KEYCODE_ESCAPE: return "escape";
            default: return null;
        }
    }

    /** Forwards a key event (true when it had a name and went to the loader). */
    static boolean forward(KeyEvent e) {
        if (e.getRepeatCount() > 0) return false;
        final String n = name(e.getKeyCode());
        if (n == null) return false;
        final boolean down = e.getAction() == KeyEvent.ACTION_DOWN;
        if (!down && e.getAction() != KeyEvent.ACTION_UP) return false;
        return Native.ok(new Runnable() { public void run() { Native.key(n, down); } });
    }

    private static boolean typing(Activity a) {
        View f = a.getCurrentFocus();
        return f instanceof EditText || ModUi.dialogId != 0 || SettingsScreen.open;
    }

    /** Wraps the activity's window callback (once). */
    static void attach(final Activity a) {
        final Window w = a.getWindow();
        final Window.Callback inner = w.getCallback();
        if (inner == null || inner instanceof Tap) return;
        w.setCallback(new Tap(inner, a));
    }

    /** The game's callback with key events copied to the loader first. */
    static final class Tap extends WindowCallbackWrapper {
        private final Activity a;

        Tap(Window.Callback inner, Activity a) {
            super(inner);
            this.a = a;
        }

        @Override public boolean dispatchKeyEvent(KeyEvent e) {
            try {
                if (!typing(a)) forward(e);
            } catch (Throwable ignored) {
            }
            return super.dispatchKeyEvent(e);
        }
    }
}
