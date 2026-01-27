package com.migo.host.internal;

import android.content.Context;

public final class AppContext {

    private static volatile Context appContext;

    private AppContext() {}

    public static void init(Context context) {
        if (appContext == null) {
            synchronized (AppContext.class) {
                if (appContext == null) {
                    appContext = context.getApplicationContext();
                }
            }
        }
    }

    public static Context get() {
        if (appContext == null) {
            throw new IllegalStateException("SDK not initialized");
        }
        return appContext;
    }
}
