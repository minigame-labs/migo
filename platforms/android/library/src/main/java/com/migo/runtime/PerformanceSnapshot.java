package com.migo.runtime;

/**
 * Immutable snapshot of engine performance metrics.
 * Obtain via {@link GameSession#getPerformanceSnapshot()}.
 */
public final class PerformanceSnapshot {
    /** Current frames per second (0 if not rendering). */
    public final float fps;
    /** Last frame render time in milliseconds. */
    public final float frameTimeMs;
    /** Cumulative dropped frame count since session start. */
    public final int droppedFrames;
    /** Milliseconds from session start to first rendered frame (0 if not yet rendered). */
    public final int firstFrameMs;
    /** Cumulative host command drops due to queue overflow. */
    public final int commandDrops;

    public PerformanceSnapshot(float fps, float frameTimeMs, int droppedFrames,
                               int firstFrameMs, int commandDrops) {
        this.fps = fps;
        this.frameTimeMs = frameTimeMs;
        this.droppedFrames = droppedFrames;
        this.firstFrameMs = firstFrameMs;
        this.commandDrops = commandDrops;
    }

    @Override
    public String toString() {
        return String.format("PerformanceSnapshot{fps=%.1f, frameTime=%.1fms, dropped=%d, firstFrame=%dms}",
            fps, frameTimeMs, droppedFrames, firstFrameMs);
    }
}
