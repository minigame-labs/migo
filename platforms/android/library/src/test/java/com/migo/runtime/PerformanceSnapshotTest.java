package com.migo.runtime;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import org.junit.Test;

/** Host-JVM tests for the append-only native performance packet. */
public final class PerformanceSnapshotTest {
    private static byte[] packet(int version, int length) {
        byte[] data = new byte[length];
        ByteBuffer buffer = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        buffer.putShort(0, (short) 0x4D47);
        buffer.putShort(2, (short) version);
        return data;
    }

    @Test
    public void v6ParsesInputTransportTailWithoutMovingLegacyFields() {
        byte[] data = packet(6, 144);
        ByteBuffer buffer = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        buffer.putInt(4, 599);
        buffer.putInt(8, 16_700);
        buffer.putInt(12, 2);
        buffer.putInt(20, 345);
        buffer.putInt(24, 4);
        buffer.putInt(132, 101);
        buffer.putInt(136, 7);
        buffer.putInt(140, 3);

        PerformanceSnapshot snapshot = PerformanceSnapshot.fromStatsPacket(data);

        assertEquals(59.9f, snapshot.fps, 0.0f);
        assertEquals(16.7f, snapshot.frameTimeMs, 0.0f);
        assertEquals(2, snapshot.droppedFrames);
        assertEquals(345, snapshot.firstFrameMs);
        assertEquals(4, snapshot.commandDrops);
        assertEquals(101, snapshot.inputCoalesced);
        assertEquals(7, snapshot.inputReliableReserveUses);
        assertEquals(3, snapshot.inputSaturationEvents);
    }

    @Test
    public void v5DefaultsAbsentInputTransportTailToZero() {
        PerformanceSnapshot snapshot = PerformanceSnapshot.fromStatsPacket(packet(5, 132));

        assertEquals(0, snapshot.inputCoalesced);
        assertEquals(0, snapshot.inputReliableReserveUses);
        assertEquals(0, snapshot.inputSaturationEvents);
    }

    @Test
    public void malformedPacketIsRejected() {
        assertNull(PerformanceSnapshot.fromStatsPacket(null));
        assertNull(PerformanceSnapshot.fromStatsPacket(new byte[15]));
        assertNull(PerformanceSnapshot.fromStatsPacket(packet(6, 16)));

        byte[] badMagic = packet(6, 144);
        badMagic[0] = 0;
        assertNull(PerformanceSnapshot.fromStatsPacket(badMagic));
    }
}
