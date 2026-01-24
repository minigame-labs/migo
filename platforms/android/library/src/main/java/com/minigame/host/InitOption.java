package com.minigame.host;

import android.content.Context;

import com.minigame.host.internal.util.Utils;

public final class InitOption {
    private final String appTmpDir;

    private final float dpi;

    private final boolean fullScreen;

    private InitOption(Builder builder) {
        this.appTmpDir = builder.appTmpDir;
        this.dpi = builder.dpi;
        this.fullScreen = builder.fullScreen;
    }

    public float getDpi() {
        return this.dpi;
    }

    public boolean isFullScreen() {
        return this.fullScreen;
    }

    public static class Builder {
        private final String appTmpDir;
        private final float dpi;
        private boolean fullScreen = true;

        public Builder(Context ctx) {
            if (ctx == null) throw new IllegalArgumentException("Context must not be null");
            this.appTmpDir = ctx.getCacheDir().getAbsolutePath();
            this.dpi = Utils.getDpi(ctx);
        }

        public Builder setFullScreen(boolean fullScreen) {
            this.fullScreen = fullScreen;
            return this;
        }

        public InitOption build() {
            return new InitOption(this);
        }
    }
}
