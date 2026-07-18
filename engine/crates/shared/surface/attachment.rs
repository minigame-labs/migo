use std::{
    error::Error,
    fmt,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::SurfaceRef;

const LIVE_BIT: u64 = 1;
const MAX_GENERATION: u64 = (u64::MAX - LIVE_BIT) >> 1;

/// A non-zero, monotonically increasing Surface attachment generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceGeneration(NonZeroU64);

impl SurfaceGeneration {
    /// Returns the numeric generation value.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    #[inline]
    fn from_live_word(word: u64) -> Self {
        let generation = word >> 1;
        Self(NonZeroU64::new(generation).expect("a live Surface word has a non-zero generation"))
    }
}

/// Failure to issue a fresh Surface generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceGenerationError {
    /// All representable generations have been consumed.
    Exhausted,
}

impl fmt::Display for SurfaceGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted => formatter.write_str("Surface generation space exhausted"),
        }
    }
}

impl Error for SurfaceGenerationError {}

/// Issues and retires Surface attachment generations without a frame-path lock.
///
/// The packed state is `generation << 1 | live_bit`. Generation zero is the
/// never-attached state and is never issued to callers.
#[derive(Debug)]
pub struct SurfaceGenerationGate {
    state: AtomicU64,
}

impl SurfaceGenerationGate {
    /// Creates a gate in the never-attached state.
    pub const fn new() -> Self {
        Self {
            state: AtomicU64::new(0),
        }
    }

    /// Returns a token for the current live generation, issuing the next
    /// generation when the previous one is retired.
    pub fn attach_or_update(
        self: &Arc<Self>,
    ) -> Result<SurfaceLivenessToken, SurfaceGenerationError> {
        let mut current = self.state.load(Ordering::Acquire);

        loop {
            if current & LIVE_BIT != 0 {
                return Ok(SurfaceLivenessToken::new(Arc::clone(self), current));
            }

            let previous_generation = current >> 1;
            if previous_generation >= MAX_GENERATION {
                return Err(SurfaceGenerationError::Exhausted);
            }

            let next_live_word = ((previous_generation + 1) << 1) | LIVE_BIT;
            match self.state.compare_exchange_weak(
                current,
                next_live_word,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(SurfaceLivenessToken::new(Arc::clone(self), next_live_word));
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Retires the current live generation.
    ///
    /// Returns `None` when the gate is already detached or has never attached.
    pub fn retire_current(&self) -> Option<SurfaceGeneration> {
        let mut current = self.state.load(Ordering::Acquire);

        loop {
            if current & LIVE_BIT == 0 {
                return None;
            }

            let generation = SurfaceGeneration::from_live_word(current);
            let retired_word = current & !LIVE_BIT;
            match self.state.compare_exchange_weak(
                current,
                retired_word,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(generation),
                Err(observed) => current = observed,
            }
        }
    }

    #[cfg(test)]
    fn from_max_retired_for_test() -> Self {
        Self {
            state: AtomicU64::new(MAX_GENERATION << 1),
        }
    }
}

impl Default for SurfaceGenerationGate {
    fn default() -> Self {
        Self::new()
    }
}

/// A cloneable, read-only proof tied to one issued Surface generation.
#[derive(Clone, Debug)]
pub struct SurfaceLivenessToken {
    gate: Arc<SurfaceGenerationGate>,
    generation: SurfaceGeneration,
    expected_live_word: u64,
}

impl SurfaceLivenessToken {
    fn new(gate: Arc<SurfaceGenerationGate>, expected_live_word: u64) -> Self {
        debug_assert_eq!(expected_live_word & LIVE_BIT, LIVE_BIT);
        Self {
            generation: SurfaceGeneration::from_live_word(expected_live_word),
            gate,
            expected_live_word,
        }
    }

    /// Returns the generation represented by this token.
    #[inline]
    pub const fn generation(&self) -> SurfaceGeneration {
        self.generation
    }

    /// Returns whether this exact generation is still the gate's live one.
    #[inline]
    pub fn is_live(&self) -> bool {
        self.gate.state.load(Ordering::Acquire) == self.expected_live_word
    }
}

/// A read-only Surface reference paired with its generation liveness token.
///
/// Clones keep the platform resource alive but cannot retire or reactivate the
/// generation. Ownership transitions remain centralized in the generation
/// gate and the Host attachment slot.
#[derive(Clone)]
pub struct SurfaceLease {
    surface: SurfaceRef,
    liveness: SurfaceLivenessToken,
}

impl SurfaceLease {
    /// Pairs a platform Surface with the liveness token issued for its handoff.
    pub fn new(surface: SurfaceRef, liveness: SurfaceLivenessToken) -> Self {
        Self { surface, liveness }
    }

    /// Returns the generation associated with this Surface handoff.
    #[inline]
    pub const fn generation(&self) -> SurfaceGeneration {
        self.liveness.generation()
    }

    /// Returns whether this lease's exact generation is still live.
    #[inline]
    pub fn is_live(&self) -> bool {
        self.liveness.is_live()
    }

    /// Returns the physical Surface size in pixels.
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.surface.size()
    }

    /// Borrows the underlying platform Surface reference.
    #[inline]
    pub fn surface(&self) -> &SurfaceRef {
        &self.surface
    }
}

impl fmt::Debug for SurfaceLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceLease")
            .field("generation", &self.generation())
            .field("live", &self.is_live())
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use super::{SurfaceGenerationError, SurfaceGenerationGate, SurfaceLease};
    use crate::surface::{Surface, SurfaceRef};

    #[derive(Debug)]
    struct TestSurface {
        size: (u32, u32),
        drops: Arc<AtomicUsize>,
    }

    impl TestSurface {
        fn new(size: (u32, u32), drops: Arc<AtomicUsize>) -> Self {
            Self { size, drops }
        }
    }

    impl Surface for TestSurface {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn size(&self) -> (u32, u32) {
            self.size
        }
    }

    impl Drop for TestSurface {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn initial_attach_issues_first_live_generation() {
        let gate = Arc::new(SurfaceGenerationGate::new());

        let token = gate.attach_or_update().unwrap();

        assert_eq!(token.generation().get(), 1);
        assert!(token.is_live());
    }

    #[test]
    fn repeated_update_reuses_the_live_generation() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = gate.attach_or_update().unwrap();

        let second = gate.attach_or_update().unwrap();

        assert_eq!(second.generation(), first.generation());
        assert!(first.is_live());
        assert!(second.is_live());
    }

    #[test]
    fn retirement_invalidates_all_tokens_for_the_current_generation() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = gate.attach_or_update().unwrap();
        let retry = gate.attach_or_update().unwrap();

        assert_eq!(gate.retire_current(), Some(first.generation()));

        assert!(!first.is_live());
        assert!(!retry.is_live());
    }

    #[test]
    fn duplicate_retirement_is_idempotent() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = gate.attach_or_update().unwrap();

        assert_eq!(gate.retire_current(), Some(first.generation()));
        assert_eq!(gate.retire_current(), None);
    }

    #[test]
    fn retired_generation_never_becomes_live_again() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = gate.attach_or_update().unwrap();
        assert_eq!(first.generation().get(), 1);
        assert!(first.is_live());

        assert_eq!(gate.retire_current(), Some(first.generation()));
        assert!(!first.is_live());

        let second = gate.attach_or_update().unwrap();
        assert_eq!(second.generation().get(), 2);
        assert!(second.is_live());
        assert!(!first.is_live());
    }

    #[test]
    fn exhausted_generation_fails_closed_without_changing_state() {
        let gate = Arc::new(SurfaceGenerationGate::from_max_retired_for_test());

        assert!(matches!(
            gate.attach_or_update(),
            Err(SurfaceGenerationError::Exhausted)
        ));
        assert_eq!(gate.retire_current(), None);
        assert!(matches!(
            gate.attach_or_update(),
            Err(SurfaceGenerationError::Exhausted)
        ));
    }

    #[test]
    fn lease_clones_share_generation_size_and_liveness() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let token = gate.attach_or_update().unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let surface: SurfaceRef = Arc::new(TestSurface::new((1920, 1080), drops.clone()));
        let lease = SurfaceLease::new(surface, token);

        let retry = lease.clone();

        assert_eq!(lease.generation(), retry.generation());
        assert_eq!(lease.size(), (1920, 1080));
        assert_eq!(retry.size(), (1920, 1080));
        assert!(lease.is_live());
        assert!(retry.is_live());
    }

    #[test]
    fn retired_lease_cannot_be_revived_by_a_new_generation() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let token = gate.attach_or_update().unwrap();
        let surface: SurfaceRef = Arc::new(TestSurface::new((1, 1), Arc::new(AtomicUsize::new(0))));
        let lease = SurfaceLease::new(surface, token);

        assert_eq!(gate.retire_current(), Some(lease.generation()));
        assert!(!lease.is_live());

        let replacement = gate.attach_or_update().unwrap();
        assert!(replacement.is_live());
        assert_ne!(replacement.generation(), lease.generation());
        assert!(!lease.is_live());
    }

    #[test]
    fn lease_clones_hold_the_surface_until_the_final_drop() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let token = gate.attach_or_update().unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let surface: SurfaceRef = Arc::new(TestSurface::new((1, 1), drops.clone()));
        let lease = SurfaceLease::new(surface, token);
        let retry = lease.clone();

        drop(lease);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        drop(retry);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn stale_token_never_revives_during_concurrent_generation_churn() {
        const READER_COUNT: usize = 4;
        const GENERATION_COUNT: u64 = 10_000;

        let gate = Arc::new(SurfaceGenerationGate::new());
        let stale = gate.attach_or_update().unwrap();
        assert_eq!(gate.retire_current(), Some(stale.generation()));
        assert!(!stale.is_live());

        let running = Arc::new(AtomicBool::new(true));
        let readers: Vec<_> = (0..READER_COUNT)
            .map(|_| {
                let stale = stale.clone();
                let running = running.clone();
                std::thread::spawn(move || {
                    while running.load(Ordering::Acquire) {
                        assert!(!stale.is_live());
                        std::hint::spin_loop();
                    }
                    assert!(!stale.is_live());
                })
            })
            .collect();

        for expected in 2..=GENERATION_COUNT {
            let current = gate.attach_or_update().unwrap();
            assert_eq!(current.generation().get(), expected);
            assert!(current.is_live());
            assert_eq!(gate.retire_current(), Some(current.generation()));
            assert!(!current.is_live());
        }

        running.store(false, Ordering::Release);
        for reader in readers {
            reader.join().unwrap();
        }
        assert!(!stale.is_live());
    }
}
