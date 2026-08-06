use shared::command_vec_pool::PooledVec;
use shared::protocol::GlBatchPayload;
use shared::protocol::render_cmd::{Canvas2DCmd, CanvasBatchPayload, DirtyRect, GLCmd};
use shared::{FrameOp, FramePacket, FramePacketBuilder};

pub(crate) struct LegacyFrameBridge {
    ops: Vec<FrameOp>,
}

impl LegacyFrameBridge {
    pub(crate) fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub(crate) fn push_gl_batch(&mut self, commands: PooledVec<GLCmd>) {
        self.ops.push(FrameOp::GlBatch(GlBatchPayload { commands }));
    }

    pub(crate) fn push_canvas_batch(
        &mut self,
        canvas_id: u32,
        commands: PooledVec<Canvas2DCmd>,
        present: bool,
        dirty_rect: Option<DirtyRect>,
    ) {
        self.ops.push(FrameOp::CanvasBatch(CanvasBatchPayload {
            canvas_id,
            commands,
            present,
            dirty_rect,
        }));
    }

    pub(crate) fn finish_frame(&mut self, frame_id: u64, raf_time_ms: f64) -> Option<FramePacket> {
        if self.ops.is_empty() {
            return None;
        }

        let mut builder = FramePacketBuilder::new(frame_id, raf_time_ms).push(FrameOp::BeginFrame);
        let mut should_present = false;
        for op in self.ops.drain(..) {
            should_present |= frame_op_requests_present(&op);
            builder = builder.push(op);
        }
        Some(finish_packet(builder, should_present))
    }
}

pub(crate) fn finish_packet(builder: FramePacketBuilder, should_present: bool) -> FramePacket {
    let builder = if should_present {
        builder.push(FrameOp::Present)
    } else {
        builder
    };
    builder.finish()
}

fn frame_op_requests_present(op: &FrameOp) -> bool {
    matches!(
        op,
        FrameOp::CanvasBatch(CanvasBatchPayload { present: true, .. })
    )
}

#[cfg(test)]
pub(crate) fn assert_bridge_flushes_pending_ops_without_packet_level_present_boundary() {
    let mut bridge = LegacyFrameBridge::new();
    bridge.push_gl_batch(Vec::new().into());
    let packet = bridge.finish_frame(12, 33.0).unwrap();

    assert_eq!(packet.frame_id(), 12);
    assert_eq!(packet.ops().len(), 2);
    assert!(matches!(packet.ops()[0], FrameOp::BeginFrame));
    assert!(matches!(
        packet.ops()[1],
        FrameOp::GlBatch(GlBatchPayload { .. })
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_flushes_pending_ops_without_packet_level_present_boundary() {
        assert_bridge_flushes_pending_ops_without_packet_level_present_boundary();
    }

    #[test]
    fn gl_only_frame_omits_packet_level_present_boundary() {
        let mut bridge = LegacyFrameBridge::new();
        bridge.push_gl_batch(Vec::new().into());

        let packet = bridge.finish_frame(5, 20.0).unwrap();

        assert!(matches!(
            packet.ops()[1],
            FrameOp::GlBatch(GlBatchPayload { .. })
        ));
        assert!(!matches!(packet.ops().last(), Some(FrameOp::Present)));
    }
}
