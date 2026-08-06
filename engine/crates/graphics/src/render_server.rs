use shared::FramePacket;
use shared::command_vec_pool::PooledVec;
use shared::protocol::render_cmd::{Canvas2DCmd, DirtyRect, GLCmd};

use crate::LegacyFrameBridge;

pub struct RenderServer {
    frame_id: u64,
    /// Last RAF timestamp from the VSync scheduler, used to stamp packets.
    current_raf_time_ms: f64,
    bridge: LegacyFrameBridge,
}

impl RenderServer {
    pub fn new() -> Self {
        Self {
            frame_id: 0,
            current_raf_time_ms: 0.0,
            bridge: LegacyFrameBridge::new(),
        }
    }

    /// Update the current RAF timestamp.  Called by the render thread each
    /// time a VSync/ticker fires, before draining commands.
    pub fn set_raf_time_ms(&mut self, ts: f64) {
        self.current_raf_time_ms = ts;
    }

    /// Stamp a FramePacket with the authoritative frame_id and raf_time_ms.
    /// Called on the render thread before packet execution.
    pub fn stamp_packet(&mut self, packet: &mut FramePacket) {
        self.frame_id = self.frame_id.wrapping_add(1);
        packet.set_frame_metadata(self.frame_id, self.current_raf_time_ms);
    }

    pub fn enqueue_gl_batch(&mut self, commands: PooledVec<GLCmd>) {
        self.bridge.push_gl_batch(commands);
    }

    pub fn enqueue_canvas_batch(
        &mut self,
        canvas_id: u32,
        commands: PooledVec<Canvas2DCmd>,
        present: bool,
        dirty_rect: Option<DirtyRect>,
    ) {
        self.bridge
            .push_canvas_batch(canvas_id, commands, present, dirty_rect);
    }

    pub fn packet_for_canvas_batch(
        frame_id: u64,
        raf_time_ms: f64,
        canvas_id: u32,
        commands: PooledVec<Canvas2DCmd>,
        present: bool,
        dirty_rect: Option<DirtyRect>,
    ) -> FramePacket {
        FramePacket::for_canvas_batch(
            frame_id,
            raf_time_ms,
            canvas_id,
            commands,
            present,
            dirty_rect,
        )
    }

    pub fn finish_frame(&mut self, raf_time_ms: f64) -> Option<FramePacket> {
        let next_frame_id = self.frame_id.wrapping_add(1);
        let packet = self.bridge.finish_frame(next_frame_id, raf_time_ms)?;
        self.frame_id = next_frame_id;
        Some(packet)
    }

    pub fn frame_id(&self) -> u64 {
        self.frame_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::FrameOp;
    use shared::protocol::render_cmd::{Canvas2DCmd, CanvasBatchPayload, DirtyRect};

    #[test]
    fn finish_frame_does_not_advance_frame_id_when_no_packet_is_produced() {
        let mut server = RenderServer::new();

        assert!(server.finish_frame(16.0).is_none());
        assert_eq!(server.frame_id(), 0);

        server.enqueue_gl_batch(Vec::new().into());
        let packet = server.finish_frame(32.0).unwrap();

        assert_eq!(packet.frame_id(), 1);
        assert_eq!(server.frame_id(), 1);
        assert!(!matches!(packet.ops().last(), Some(FrameOp::Present)));
    }

    #[test]
    fn finish_canvas2d_frame_emits_presenting_packet() {
        let mut server = RenderServer::new();
        server.enqueue_canvas_batch(
            1,
            vec![Canvas2DCmd::Save].into(),
            true,
            Some(DirtyRect {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 16.0,
            }),
        );

        let packet = server.finish_frame(33.0).unwrap();

        assert_eq!(packet.frame_id(), 1);
        assert!(matches!(
            packet.ops()[1],
            FrameOp::CanvasBatch(CanvasBatchPayload { present: true, .. })
        ));
        assert!(matches!(packet.ops().last(), Some(FrameOp::Present)));
    }

    #[test]
    fn packet_for_non_presenting_canvas_batch_omits_present() {
        let packet = RenderServer::packet_for_canvas_batch(
            7,
            12.5,
            1,
            vec![Canvas2DCmd::Save].into(),
            false,
            None,
        );

        assert_eq!(packet.frame_id(), 7);
        assert!(matches!(
            packet.ops()[1],
            FrameOp::CanvasBatch(CanvasBatchPayload { present: false, .. })
        ));
        assert!(!matches!(packet.ops().last(), Some(FrameOp::Present)));
    }

    #[test]
    fn finish_non_presenting_canvas2d_frame_omits_present_packet() {
        let mut server = RenderServer::new();
        server.enqueue_canvas_batch(1, vec![Canvas2DCmd::Save].into(), false, None);

        let packet = server.finish_frame(33.0).unwrap();

        assert_eq!(packet.frame_id(), 1);
        assert!(matches!(
            packet.ops()[1],
            FrameOp::CanvasBatch(CanvasBatchPayload { present: false, .. })
        ));
        assert!(!matches!(packet.ops().last(), Some(FrameOp::Present)));
    }

    #[test]
    fn frame_id_increments_monotonically_across_frames() {
        let mut server = RenderServer::new();
        assert_eq!(server.frame_id(), 0);

        for expected in 1..=5u64 {
            server.enqueue_canvas_batch(1, vec![Canvas2DCmd::Save].into(), true, None);
            let packet = server.finish_frame(expected as f64 * 16.666).unwrap();
            assert_eq!(packet.frame_id(), expected);
            assert_eq!(server.frame_id(), expected);
        }
    }

    #[test]
    fn raf_time_ms_preserved_in_packet() {
        let mut server = RenderServer::new();
        server.enqueue_canvas_batch(1, vec![Canvas2DCmd::Save].into(), true, None);

        let packet = server.finish_frame(16.666).unwrap();
        assert!((packet.raf_time_ms() - 16.666).abs() < f64::EPSILON);
    }

    /// stamp_packet overwrites sentinel (0, 0.0) with real metadata.
    /// This is the live path: JS sends packets with sentinels,
    /// RenderServer stamps them on the render thread before execution.
    #[test]
    fn stamp_packet_overwrites_sentinel_with_real_metadata() {
        let mut server = RenderServer::new();
        server.set_raf_time_ms(16.666);

        // JS constructs packet with sentinel values.
        let mut packet =
            FramePacket::for_canvas_batch(0, 0.0, 1, vec![Canvas2DCmd::Save].into(), true, None);
        assert_eq!(packet.frame_id(), 0);
        assert_eq!(packet.raf_time_ms(), 0.0);

        // RenderServer stamps real metadata before execution.
        server.stamp_packet(&mut packet);

        assert_eq!(packet.frame_id(), 1);
        assert!((packet.raf_time_ms() - 16.666).abs() < f64::EPSILON);
        assert_eq!(server.frame_id(), 1);
    }

    /// Multiple stamp_packet calls produce incrementing frame_ids.
    #[test]
    fn stamp_packet_increments_frame_id_across_calls() {
        let mut server = RenderServer::new();
        server.set_raf_time_ms(33.333);

        for expected in 1..=3u64 {
            let mut packet = FramePacket::for_canvas_batch(
                0,
                0.0,
                1,
                vec![Canvas2DCmd::Save].into(),
                true,
                None,
            );
            server.stamp_packet(&mut packet);
            assert_eq!(packet.frame_id(), expected);
        }
        assert_eq!(server.frame_id(), 3);
    }

    /// stamp_packet uses the latest raf_time_ms from set_raf_time_ms.
    #[test]
    fn stamp_packet_uses_latest_raf_time() {
        let mut server = RenderServer::new();

        server.set_raf_time_ms(16.666);
        let mut p1 =
            FramePacket::for_canvas_batch(0, 0.0, 1, vec![Canvas2DCmd::Save].into(), true, None);
        server.stamp_packet(&mut p1);
        assert!((p1.raf_time_ms() - 16.666).abs() < f64::EPSILON);

        server.set_raf_time_ms(33.333);
        let mut p2 =
            FramePacket::for_canvas_batch(0, 0.0, 1, vec![Canvas2DCmd::Save].into(), true, None);
        server.stamp_packet(&mut p2);
        assert!((p2.raf_time_ms() - 33.333).abs() < f64::EPSILON);
    }

    /// WebGL packet (via for_gl_batch) gets stamped by RenderServer just
    /// like Canvas2D packets — same frame_id sequence, same raf_time_ms.
    #[test]
    fn stamp_packet_works_for_gl_batch_packets() {
        use shared::protocol::render_cmd::GLCmd;

        let mut server = RenderServer::new();
        server.set_raf_time_ms(16.666);

        // Canvas2D frame
        let mut p1 =
            FramePacket::for_canvas_batch(0, 0.0, 1, vec![Canvas2DCmd::Save].into(), true, None);
        server.stamp_packet(&mut p1);
        assert_eq!(p1.frame_id(), 1);

        // WebGL frame — same frame_id sequence, same raf_time.
        let mut p2 = FramePacket::for_gl_batch(
            0,
            0.0,
            vec![GLCmd::Clear {
                canvas_id: 1,
                bit_field: 0x4000,
            }]
            .into(),
        );
        server.stamp_packet(&mut p2);
        assert_eq!(p2.frame_id(), 2);
        assert!((p2.raf_time_ms() - 16.666).abs() < f64::EPSILON);
    }

    /// Mixed Canvas2D + WebGL packets in sequence maintain monotonic frame_ids.
    #[test]
    fn mixed_canvas2d_and_gl_packets_share_frame_id_sequence() {
        use shared::protocol::render_cmd::GLCmd;

        let mut server = RenderServer::new();
        server.set_raf_time_ms(16.666);

        let mut ids = Vec::new();
        for i in 0..3 {
            let mut packet = if i % 2 == 0 {
                FramePacket::for_canvas_batch(0, 0.0, 1, vec![Canvas2DCmd::Save].into(), true, None)
            } else {
                FramePacket::for_gl_batch(
                    0,
                    0.0,
                    vec![GLCmd::Clear {
                        canvas_id: 1,
                        bit_field: 0x4000,
                    }]
                    .into(),
                )
            };
            server.stamp_packet(&mut packet);
            ids.push(packet.frame_id());
        }

        assert_eq!(ids, vec![1, 2, 3]);
    }
}
