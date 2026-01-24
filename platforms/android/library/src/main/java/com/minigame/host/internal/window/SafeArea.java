package com.minigame.host.internal.window;

public class SafeArea {
    public int left;
    public int top;
    public int right;
    public int bottom;
    public int width;
    public int height;

    public SafeArea(int left, int top, int right, int bottom) {
        this.left = left;
        this.top = top;
        this.right = right;
        this.bottom = bottom;
        this.width = right - left;
        this.height = bottom - top;
    }
}
