package com.migo.runtime;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

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
    /** Cumulative input updates safely coalesced in the host queue. */
    public final int inputCoalesced;
    /** Cumulative reliable transitions accepted through reserved capacity. */
    public final int inputReliableReserveUses;
    /** Cumulative input events refused because every eligible lane was full. */
    public final int inputSaturationEvents;

    public PerformanceSnapshot(float fps, float frameTimeMs, int droppedFrames,
                               int firstFrameMs, int commandDrops) {
        this(fps, frameTimeMs, droppedFrames, firstFrameMs, commandDrops, 0, 0, 0);
    }

    public PerformanceSnapshot(float fps, float frameTimeMs, int droppedFrames,
                               int firstFrameMs, int commandDrops, int inputCoalesced,
                               int inputReliableReserveUses, int inputSaturationEvents) {
        this.fps = fps;
        this.frameTimeMs = frameTimeMs;
        this.droppedFrames = droppedFrames;
        this.firstFrameMs = firstFrameMs;
        this.commandDrops = commandDrops;
        this.inputCoalesced = inputCoalesced;
        this.inputReliableReserveUses = inputReliableReserveUses;
        this.inputSaturationEvents = inputSaturationEvents;
    }

    static PerformanceSnapshot fromStatsPacket(byte[] data) {
        if (data == null || data.length < 16) return null;
        ByteBuffer buffer = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        if ((buffer.getShort(0) & 0xFFFF) != 0x4D47) return null;

        int version = buffer.getShort(2) & 0xFFFF;
        if (version >= 6 && data.length < 144) return null;

        int h = 4;
        int fpsX10 = buffer.getInt(h);
        int frameTimeUs = buffer.getInt(h + 4);
        int dropped = buffer.getInt(h + 8);
        int firstFrameMs = data.length >= h + 20 ? buffer.getInt(h + 16) : 0;
        int commandDrops = data.length >= h + 24 ? buffer.getInt(h + 20) : 0;
        int inputCoalesced = data.length >= h + 132 ? buffer.getInt(h + 128) : 0;
        int inputReliableReserveUses =
                data.length >= h + 136 ? buffer.getInt(h + 132) : 0;
        int inputSaturationEvents =
                data.length >= h + 140 ? buffer.getInt(h + 136) : 0;

        return new PerformanceSnapshot(
                (fpsX10 & 0xFFFFFFFFL) / 10f,
                (frameTimeUs & 0xFFFFFFFFL) / 1000f,
                dropped,
                firstFrameMs,
                commandDrops,
                inputCoalesced,
                inputReliableReserveUses,
                inputSaturationEvents);
    }

    @Override
    public String toString() {
        return String.format(
                "PerformanceSnapshot{fps=%.1f, frameTime=%.1fms, dropped=%d, "
                        + "firstFrame=%dms, inputSaturation=%d}",
                fps, frameTimeMs, droppedFrames, firstFrameMs, inputSaturationEvents);
    }
}
