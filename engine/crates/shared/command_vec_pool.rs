//! Bounded recycler for command vectors crossing the host/render thread boundary.

use std::sync::OnceLock;

use crossbeam_channel::{Receiver, Sender, bounded};

use crate::protocol::render_cmd::{Canvas2DCmd, GLCmd};

pub const GL_COMMAND_VEC_INITIAL_CAPACITY: usize = 16;
pub const CANVAS_COMMAND_VEC_INITIAL_CAPACITY: usize = 8;
pub const COMMAND_VEC_POOL_SLOTS: usize = 16;
pub const MAX_RECYCLABLE_COMMAND_CAPACITY: usize = 512;

struct CommandVecPool<T> {
    sender: Sender<Vec<T>>,
    receiver: Receiver<Vec<T>>,
    minimum_capacity: usize,
    maximum_capacity: usize,
}

impl<T> CommandVecPool<T> {
    fn new(slots: usize, minimum_capacity: usize, maximum_capacity: usize) -> Self {
        assert!(slots > 0, "command vector pool must have at least one slot");
        assert!(
            minimum_capacity <= maximum_capacity,
            "minimum command capacity exceeds retention ceiling"
        );
        let (sender, receiver) = bounded(slots);
        Self {
            sender,
            receiver,
            minimum_capacity,
            maximum_capacity,
        }
    }

    #[inline]
    fn take(&self) -> Vec<T> {
        let mut commands = self
            .receiver
            .try_recv()
            .unwrap_or_else(|_| Vec::with_capacity(self.minimum_capacity));
        debug_assert!(commands.is_empty());
        if commands.capacity() < self.minimum_capacity {
            commands.reserve_exact(self.minimum_capacity);
        }
        commands
    }

    #[inline]
    fn recycle(&self, commands: Vec<T>) -> bool {
        if !commands.is_empty() {
            return false;
        }
        if commands.capacity() > self.maximum_capacity {
            return false;
        }
        self.sender.try_send(commands).is_ok()
    }
}

fn gl_pool() -> &'static CommandVecPool<GLCmd> {
    static POOL: OnceLock<CommandVecPool<GLCmd>> = OnceLock::new();
    POOL.get_or_init(|| {
        CommandVecPool::new(
            COMMAND_VEC_POOL_SLOTS,
            GL_COMMAND_VEC_INITIAL_CAPACITY,
            MAX_RECYCLABLE_COMMAND_CAPACITY,
        )
    })
}

fn canvas_pool() -> &'static CommandVecPool<Canvas2DCmd> {
    static POOL: OnceLock<CommandVecPool<Canvas2DCmd>> = OnceLock::new();
    POOL.get_or_init(|| {
        CommandVecPool::new(
            COMMAND_VEC_POOL_SLOTS,
            CANVAS_COMMAND_VEC_INITIAL_CAPACITY,
            MAX_RECYCLABLE_COMMAND_CAPACITY,
        )
    })
}

#[inline]
pub fn take_gl_command_vec() -> Vec<GLCmd> {
    gl_pool().take()
}

#[inline]
pub fn take_canvas_command_vec() -> Vec<Canvas2DCmd> {
    canvas_pool().take()
}

#[inline]
pub fn recycle_gl_command_vec(commands: Vec<GLCmd>) {
    let _ = gl_pool().recycle(commands);
}

#[inline]
pub fn recycle_canvas_command_vec(commands: Vec<Canvas2DCmd>) {
    let _ = canvas_pool().recycle(commands);
}

#[cfg(test)]
mod tests {
    use super::CommandVecPool;

    #[test]
    fn recycled_vector_reuses_its_allocation() {
        let pool = CommandVecPool::<u32>::new(1, 4, 8);
        let mut commands = pool.take();
        commands.extend_from_slice(&[1, 2, 3, 4]);
        commands.clear();
        let allocation = commands.as_ptr();
        let capacity = commands.capacity();

        assert!(pool.recycle(commands));
        let reused = pool.take();
        assert_eq!(reused.as_ptr(), allocation);
        assert_eq!(reused.capacity(), capacity);
        assert!(reused.is_empty());
    }

    #[test]
    fn full_pool_drops_excess_vectors_without_blocking() {
        let pool = CommandVecPool::<u32>::new(1, 4, 8);
        assert!(pool.recycle(Vec::with_capacity(4)));
        assert!(!pool.recycle(Vec::with_capacity(4)));
    }

    #[test]
    fn oversized_vector_is_not_retained() {
        let pool = CommandVecPool::<u32>::new(1, 4, 8);
        assert!(!pool.recycle(Vec::with_capacity(9)));
        let fresh = pool.take();
        assert!(fresh.capacity() >= 4);
        assert!(fresh.capacity() <= 8);
    }

    #[test]
    fn non_empty_vector_is_rejected_without_panicking() {
        let pool = CommandVecPool::<u32>::new(1, 4, 8);

        assert!(!pool.recycle(vec![1]));
        assert!(pool.receiver.is_empty());
    }
}
