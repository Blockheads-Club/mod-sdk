// Blockheads Club mod loader: a Window.Callback that forwards everything. Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.view.ActionMode;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.SearchEvent;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityEvent;

/** Forwards every call to the wrapped callback (the framework's own wrapper is hidden); subclasses override. */
class WindowCallbackWrapper implements Window.Callback {
    final Window.Callback inner;

    WindowCallbackWrapper(Window.Callback inner) {
        this.inner = inner;
    }

    @Override public boolean dispatchKeyEvent(KeyEvent e) { return inner.dispatchKeyEvent(e); }
    @Override public boolean dispatchKeyShortcutEvent(KeyEvent e) { return inner.dispatchKeyShortcutEvent(e); }
    @Override public boolean dispatchTouchEvent(MotionEvent e) { return inner.dispatchTouchEvent(e); }
    @Override public boolean dispatchTrackballEvent(MotionEvent e) { return inner.dispatchTrackballEvent(e); }
    @Override public boolean dispatchGenericMotionEvent(MotionEvent e) { return inner.dispatchGenericMotionEvent(e); }
    @Override public boolean dispatchPopulateAccessibilityEvent(AccessibilityEvent e) { return inner.dispatchPopulateAccessibilityEvent(e); }
    @Override public View onCreatePanelView(int f) { return inner.onCreatePanelView(f); }
    @Override public boolean onCreatePanelMenu(int f, Menu m) { return inner.onCreatePanelMenu(f, m); }
    @Override public boolean onPreparePanel(int f, View v, Menu m) { return inner.onPreparePanel(f, v, m); }
    @Override public boolean onMenuOpened(int f, Menu m) { return inner.onMenuOpened(f, m); }
    @Override public boolean onMenuItemSelected(int f, MenuItem i) { return inner.onMenuItemSelected(f, i); }
    @Override public void onWindowAttributesChanged(WindowManager.LayoutParams a) { inner.onWindowAttributesChanged(a); }
    @Override public void onContentChanged() { inner.onContentChanged(); }
    @Override public void onWindowFocusChanged(boolean f) { inner.onWindowFocusChanged(f); }
    @Override public void onAttachedToWindow() { inner.onAttachedToWindow(); }
    @Override public void onDetachedFromWindow() { inner.onDetachedFromWindow(); }
    @Override public void onPanelClosed(int f, Menu m) { inner.onPanelClosed(f, m); }
    @Override public boolean onSearchRequested() { return inner.onSearchRequested(); }
    @Override public boolean onSearchRequested(SearchEvent e) { return inner.onSearchRequested(e); }
    @Override public ActionMode onWindowStartingActionMode(ActionMode.Callback c) { return inner.onWindowStartingActionMode(c); }
    @Override public ActionMode onWindowStartingActionMode(ActionMode.Callback c, int type) { return inner.onWindowStartingActionMode(c, type); }
    @Override public void onActionModeStarted(ActionMode m) { inner.onActionModeStarted(m); }
    @Override public void onActionModeFinished(ActionMode m) { inner.onActionModeFinished(m); }
}
