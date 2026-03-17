use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use tokio::sync::oneshot;

pub struct SharedImageEntry {
    pub shared_image_id: u32,
    pub refs: usize,
    pub dims: (usize, usize),
}

pub struct ImageCache {
    // keyed by canonical src
    by_src: HashMap<String, SharedImageEntry>,
    // alias (caller image_id) -> shared_image_id
    alias_to_shared: HashMap<u32, u32>,
    // shared_image_id -> src
    shared_to_src: HashMap<u32, String>,

    // in-flight loads: src -> list of waiters
    loading_map: HashMap<String, Vec<oneshot::Sender<Result<(u32, (usize, usize)), String>>>>,
}

pub enum BeginLoadResult {
    AlreadyLoaded((u32, (usize, usize))),
    Join(oneshot::Receiver<Result<(u32, (usize, usize)), String>>),
    StartLoading,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            by_src: HashMap::new(),
            alias_to_shared: HashMap::new(),
            shared_to_src: HashMap::new(),
            loading_map: HashMap::new(),
        }
    }

    /// Called at start of op_load_image: remove previous alias and dec refs.
    /// Returns Some(shared_id) if refs dropped to 0 -> caller should destroy that shared_id.
    pub fn remove_previous_alias(&mut self, image_id: u32) -> Option<u32> {
        if let Some(shared_id) = self.alias_to_shared.remove(&image_id) {
            if let Some(src) = self.shared_to_src.get(&shared_id).cloned() {
                if let Some(entry) = self.by_src.get_mut(&src) {
                    entry.refs = entry.refs.saturating_sub(1);
                    if entry.refs == 0 {
                        self.by_src.remove(&src);
                        self.shared_to_src.remove(&shared_id);
                        return Some(shared_id);
                    }
                }
            }
        }
        None
    }

    pub fn begin_load(&mut self, image_id: u32, src: &str) -> BeginLoadResult {
        // already loaded
        if let Some(entry) = self.by_src.get_mut(src) {
            entry.refs = entry.refs.saturating_add(1);
            let shared_id = entry.shared_image_id;

            self.alias_to_shared.insert(image_id, shared_id);
            self.shared_to_src
                .entry(shared_id)
                .or_insert_with(|| src.to_string());

            return BeginLoadResult::AlreadyLoaded((shared_id, entry.dims));
        }

        // loading in progress
        if self.loading_map.contains_key(src) {
            let (tx, rx) = oneshot::channel();
            self.loading_map.get_mut(src).unwrap().push(tx);
            return BeginLoadResult::Join(rx);
        }

        // become loader
        self.loading_map.insert(src.to_string(), Vec::new());
        BeginLoadResult::StartLoading
    }

    /// Called by *joiners* after they got (shared_id, dims), to ensure alias/ref is correct
    /// even if JS does not replace image_id.
    pub fn bind_alias_existing(&mut self, image_id: u32, src: &str, shared_id: u32) {
        if let Some(entry) = self.by_src.get_mut(src) {
            // entry should exist already
            entry.refs = entry.refs.saturating_add(1);
            self.alias_to_shared.insert(image_id, shared_id);
            self.shared_to_src
                .entry(shared_id)
                .or_insert_with(|| src.to_string());
        }
    }

    /// Called by the loader when finished.
    pub fn finish_load(
        &mut self,
        loader_image_id: u32,
        src: &str,
        res: Result<(usize, usize), String>,
    ) {
        let waiters = self.loading_map.remove(src).unwrap_or_default();

        match res {
            Ok(dims) => {
                // loader becomes the shared_id owner for this src
                self.by_src.insert(
                    src.to_string(),
                    SharedImageEntry {
                        shared_image_id: loader_image_id,
                        refs: 1,
                        dims,
                    },
                );
                self.alias_to_shared
                    .insert(loader_image_id, loader_image_id);
                self.shared_to_src.insert(loader_image_id, src.to_string());

                for tx in waiters {
                    let _ = tx.send(Ok((loader_image_id, dims)));
                }
            }
            Err(msg) => {
                for tx in waiters {
                    let _ = tx.send(Err(msg.clone()));
                }
            }
        }
    }

    /// destroy: dec refs; return Some(shared_id) if should actually destroy underlying shared resource
    pub fn try_release_and_get_destroy_rid(&mut self, image_id: u32) -> Option<u32> {
        let shared_id = self.alias_to_shared.remove(&image_id).unwrap_or(image_id);

        if let Some(src) = self.shared_to_src.get(&shared_id).cloned() {
            if let Some(entry) = self.by_src.get_mut(&src) {
                if entry.refs > 0 {
                    entry.refs -= 1;
                }
                if entry.refs == 0 {
                    self.by_src.remove(&src);
                    self.shared_to_src.remove(&shared_id);
                    return Some(shared_id);
                }
                return None;
            }
        }

        // not tracked as shared -> destroy caller id directly
        Some(image_id)
    }
}

pub static IMAGE_CACHE: Lazy<Mutex<ImageCache>> = Lazy::new(|| Mutex::new(ImageCache::new()));

/// Clear all shared image tracking state.
///
/// Must be called during host shutdown to prevent stale entries from
/// leaking into the next session (IMAGE_CACHE is process-global).
pub fn clear_shared_image_cache() {
    let mut c = IMAGE_CACHE.lock();
    c.by_src.clear();
    c.alias_to_shared.clear();
    c.shared_to_src.clear();
    c.loading_map.clear();
}
