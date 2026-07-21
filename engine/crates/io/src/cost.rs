#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheapPolicy {
    pub small_read_bytes: usize,
    pub small_copy_bytes: usize,
}

impl Default for CheapPolicy {
    fn default() -> Self {
        Self {
            small_read_bytes: 16 * 1024,
            small_copy_bytes: 4 * 1024,
        }
    }
}
