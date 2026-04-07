use std::collections::HashSet;

pub struct LayerCache {
    dirty_layers: HashSet<u32>,
}

impl LayerCache {
    pub fn new() -> Self {
        Self {
            dirty_layers: HashSet::new(),
        }
    }

    pub fn mark_dirty(&mut self, canvas_id: u32) {
        self.dirty_layers.insert(canvas_id);
    }

    pub fn take_flush_for_readback(&mut self, canvas_id: u32) -> bool {
        self.dirty_layers.remove(&canvas_id)
    }

    pub fn clear_dirty(&mut self, canvas_id: u32) {
        self.dirty_layers.remove(&canvas_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_requests_flush_pending_layer_once() {
        let mut cache = LayerCache::new();
        cache.mark_dirty(1);
        assert!(cache.take_flush_for_readback(1));
        assert!(!cache.take_flush_for_readback(1));
    }

    #[test]
    fn clear_dirty_resets_pending_readback_flush() {
        let mut cache = LayerCache::new();
        cache.mark_dirty(7);
        cache.clear_dirty(7);

        assert!(!cache.take_flush_for_readback(7));
    }
}
