//! Bounded recycler for command vectors crossing the host/render thread boundary.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::{Receiver, Sender, bounded};

use crate::protocol::render_cmd::{Canvas2DCmd, GLCmd};

pub const GL_COMMAND_VEC_INITIAL_CAPACITY: usize = 16;
pub const CANVAS_COMMAND_VEC_INITIAL_CAPACITY: usize = 8;
pub const COMMAND_VEC_POOL_SLOTS: usize = 16;

/// Commands per slot the pool's memory budget is sized for.
///
/// **This is a budget, not a per-vector ceiling, and the difference is the whole
/// point.** It was a ceiling: a vector that outgrew it was dropped, so a frame
/// one command past it started the next frame from the minimum capacity and
/// regrew — six reallocations and about 175 KiB of copying, every frame, on the
/// thread running the game, for one command's difference. A cliff, not a
/// gradient.
///
/// What the ceiling was protecting is memory: one pathological frame must not
/// leave a huge allocation parked in the pool forever. A per-vector element
/// count does not express that. It bounds the wrong quantity (one vector, not
/// the pool), in the wrong unit (elements, so the same constant means different
/// amounts for `GLCmd` and `Canvas2DCmd`), and it already permitted
/// `slots * this * size_of::<T>()` bytes to sit in a full pool anyway.
///
/// So the pool now bounds exactly that quantity — its own retained bytes — and
/// spends the allowance on whatever shape the workload has: one large vector or
/// sixteen small ones. **The permitted worst case is unchanged by construction**,
/// because the budget is derived from the same arithmetic the old rule already
/// allowed, and no single frame size is special any more.
pub const COMMAND_VEC_POOL_BUDGET_COMMANDS_PER_SLOT: usize = 512;

struct CommandVecPool<T> {
    sender: Sender<Vec<T>>,
    receiver: Receiver<Vec<T>>,
    minimum_capacity: usize,
    retained_byte_budget: usize,
    retained_bytes: AtomicUsize,
}

impl<T> CommandVecPool<T> {
    /// `budget_commands_per_slot` stays in *commands*, deliberately: the pool
    /// converts to bytes itself, so no call site can pass one unit where the
    /// other is meant. A `usize` budget in bytes next to a `usize` count of
    /// commands is a mistake the compiler cannot catch, and the first draft of
    /// this change made it.
    fn new(slots: usize, minimum_capacity: usize, budget_commands_per_slot: usize) -> Self {
        assert!(slots > 0, "command vector pool must have at least one slot");
        assert!(
            minimum_capacity <= budget_commands_per_slot,
            "a vector at the minimum capacity must fit the pool's budget"
        );
        let (sender, receiver) = bounded(slots);
        Self {
            sender,
            receiver,
            minimum_capacity,
            retained_byte_budget: slots
                .saturating_mul(budget_commands_per_slot)
                .saturating_mul(size_of::<T>()),
            retained_bytes: AtomicUsize::new(0),
        }
    }

    #[inline]
    /// Takes the capacity rather than the vector: a recycled vector is empty, so
    /// what it occupies is the allocation the next frame will reuse, never its
    /// length.
    fn bytes_of(capacity: usize) -> usize {
        capacity.saturating_mul(size_of::<T>())
    }

    #[inline]
    fn take(&self) -> Vec<T> {
        let mut commands = match self.receiver.try_recv() {
            Ok(recycled) => {
                // Released before the capacity below can change it, so what is
                // subtracted is exactly what `recycle` added.
                self.retained_bytes
                    .fetch_sub(Self::bytes_of(recycled.capacity()), Ordering::Relaxed);
                recycled
            }
            Err(_) => Vec::with_capacity(self.minimum_capacity),
        };
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
        let bytes = Self::bytes_of(commands.capacity());
        // Reserve first, then place. Both refusals below give the reservation
        // back: one that outlived its vector would shrink the budget for the rest
        // of the process, and a pool that has quietly stopped retaining anything
        // looks exactly like a pool that is working — every caller still gets a
        // vector, just a freshly allocated one every time.
        //
        // This one check also turns away the pathological frame the budget exists
        // for, the single vector that would fill the pool by itself: it is over
        // budget even from an empty pool, so it is refused like any other
        // overflow. An explicit `bytes > budget` test ahead of this one would read
        // like a second guard while changing no outcome, and mutation says so —
        // removing it killed no test.
        //
        // Under concurrent recyclers the counter can transiently read high, never
        // low, so the pool may refuse slightly early but can never over-retain.
        if self.retained_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes
            > self.retained_byte_budget
        {
            self.retained_bytes.fetch_sub(bytes, Ordering::Relaxed);
            return false;
        }
        if self.sender.try_send(commands).is_err() {
            self.retained_bytes.fetch_sub(bytes, Ordering::Relaxed);
            return false;
        }
        true
    }

    #[cfg(test)]
    fn retained_bytes(&self) -> usize {
        self.retained_bytes.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn retained_byte_budget(&self) -> usize {
        self.retained_byte_budget
    }
}

fn gl_pool() -> &'static CommandVecPool<GLCmd> {
    static POOL: OnceLock<CommandVecPool<GLCmd>> = OnceLock::new();
    POOL.get_or_init(|| {
        CommandVecPool::new(
            COMMAND_VEC_POOL_SLOTS,
            GL_COMMAND_VEC_INITIAL_CAPACITY,
            COMMAND_VEC_POOL_BUDGET_COMMANDS_PER_SLOT,
        )
    })
}

fn canvas_pool() -> &'static CommandVecPool<Canvas2DCmd> {
    static POOL: OnceLock<CommandVecPool<Canvas2DCmd>> = OnceLock::new();
    POOL.get_or_init(|| {
        CommandVecPool::new(
            COMMAND_VEC_POOL_SLOTS,
            CANVAS_COMMAND_VEC_INITIAL_CAPACITY,
            COMMAND_VEC_POOL_BUDGET_COMMANDS_PER_SLOT,
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

    /// The pathological frame the budget exists for: a single vector that would
    /// fill the pool by itself never gets in, however many slots are free.
    #[test]
    fn a_vector_larger_than_the_whole_budget_is_not_retained() {
        let pool = CommandVecPool::<u32>::new(1, 4, 8);
        assert!(!pool.recycle(Vec::with_capacity(9)));
        assert_eq!(pool.retained_bytes(), 0, "a refusal reserved budget anyway");
        let fresh = pool.take();
        assert!(fresh.capacity() >= 4);
    }

    /// **The property the per-vector ceiling never actually delivered.** It
    /// bounded one vector while letting every slot hold one, so the pool's real
    /// worst case was `slots ×` the ceiling. The budget bounds the pool, so
    /// retention stops when the bytes run out even though slots remain.
    #[test]
    fn the_pool_bounds_its_own_bytes_not_the_size_of_one_vector() {
        // Four slots, sixteen commands of budget in total.
        let pool = CommandVecPool::<u32>::new(4, 4, 4);
        assert_eq!(pool.retained_byte_budget(), 4 * 4 * size_of::<u32>());

        assert!(
            pool.recycle(Vec::with_capacity(8)),
            "first half of the budget"
        );
        assert!(
            pool.recycle(Vec::with_capacity(8)),
            "second half of the budget"
        );
        assert_eq!(pool.retained_bytes(), pool.retained_byte_budget());

        assert!(
            !pool.recycle(Vec::with_capacity(8)),
            "the pool kept retaining past its budget because two slots were free"
        );
        assert_eq!(
            pool.retained_bytes(),
            pool.retained_byte_budget(),
            "the refused vector left its reservation behind"
        );
    }

    /// A reservation that outlives its vector shrinks the budget for the rest of
    /// the process, and a pool that has quietly stopped retaining anything is
    /// indistinguishable from one that is working — every caller still gets a
    /// vector, just a fresh one every time. So the accounting has to come back to
    /// zero, on the refusal paths as much as on the success path.
    #[test]
    fn retention_accounting_returns_to_zero_however_the_vectors_leave() {
        // One slot, but budget enough for many vectors: the two limits have to be
        // separated or "refused by a full pool" is really "refused by the budget"
        // and the rollback below is never reached. A mutant that dropped that
        // rollback survived the first version of this test for exactly that
        // reason.
        let pool = CommandVecPool::<u32>::new(1, 4, 512);
        let one = 8 * size_of::<u32>();
        assert!(
            one * 2 <= pool.retained_byte_budget(),
            "fixture needs budget to spare so a refusal can come from the slots"
        );

        // Accepted, then reclaimed.
        assert!(pool.recycle(Vec::with_capacity(8)));
        assert_eq!(pool.retained_bytes(), one);
        let taken = pool.take();
        assert_eq!(pool.retained_bytes(), 0);
        drop(taken);

        // Refused by the single occupied slot, with budget still available: the
        // reservation taken to attempt the placement must be given back.
        assert!(pool.recycle(Vec::with_capacity(8)));
        assert!(
            !pool.recycle(Vec::with_capacity(8)),
            "fixture needs the second recycle to be refused by a full pool"
        );
        assert_eq!(
            pool.retained_bytes(),
            one,
            "a vector refused by a full pool kept its reservation, so the budget \
             shrinks every time the pool is full until nothing is ever retained"
        );

        // Refused for being non-empty, before any reservation is taken.
        assert!(!pool.recycle(vec![1]));
        assert_eq!(pool.retained_bytes(), one);

        let _ = pool.take();
        assert_eq!(pool.retained_bytes(), 0);
    }

    /// A frame heavier than the retention rule allows must still keep its
    /// allocation. Under the per-vector element ceiling it did not: crossing the
    /// ceiling by one command dropped the vector, so the next frame regrew it
    /// from the minimum -- six reallocations and 175 KiB of copying per frame, on
    /// the thread running the game, for one command's difference.
    #[test]
    fn a_vector_heavier_than_one_frames_allowance_keeps_its_allocation() {
        // Production shape: the same slot count and per-slot allowance the real
        // pools use, and a vector twice that allowance.
        let pool = CommandVecPool::<u32>::new(16, 4, 512);
        let mut commands = pool.take();
        commands.reserve_exact(1024);
        let allocation = commands.as_ptr();
        let capacity = commands.capacity();

        assert!(
            pool.recycle(commands),
            "a vector the pool has room for was dropped because of its length"
        );
        let reused = pool.take();
        assert_eq!(reused.as_ptr(), allocation);
        assert_eq!(reused.capacity(), capacity);
    }

    #[test]
    fn non_empty_vector_is_rejected_without_panicking() {
        let pool = CommandVecPool::<u32>::new(1, 4, 8);

        assert!(!pool.recycle(vec![1]));
        assert!(pool.receiver.is_empty());
    }
}
