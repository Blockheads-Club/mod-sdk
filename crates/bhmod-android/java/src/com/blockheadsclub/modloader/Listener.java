// Blockheads Club mod loader: entry point listed in apportable.app.lifecycle_listeners.
// Copyright (C) 2026 Blockheads Club contributors. GPL-3.0-or-later.
package com.blockheadsclub.modloader;

import android.app.Activity;
import android.app.Application;
import android.util.Log;

/**
 * Created by the game in Application.onCreate (Class.forName + newInstance) before any native library loads. It
 * tracks the game activity for the overlay UI and never throws.
 */
public final class Listener implements com.apportable.Lifecycle.Listener {
    static final int APPLICATION_CREATED = 0, ACTIVITY_RESUMED = 4, ACTIVITY_PAUSED = 5, ACTIVITY_DESTROYED = 7;

    public Listener() {
        Log.i(Bridge.TAG, "lifecycle listener constructed");
    }

    @Override
    public void OnLifecycleStateChanged(int event, Application app, Activity activity) {
        try {
            if (app != null) Bridge.init(app);
            if (activity == null) return;
            if (event == ACTIVITY_DESTROYED) {
                Bridge.activityGone(activity);
            } else if (event == ACTIVITY_RESUMED || event == 1 || event == 2) {
                Bridge.activity(activity);
            }
        } catch (Throwable t) {
            Log.w(Bridge.TAG, "lifecycle event " + event + " failed", t);
        }
    }
}
