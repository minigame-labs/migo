use std::{
    error::Error,
    fmt,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use super::{
    SurfaceGeneration, SurfaceGenerationError, SurfaceGenerationGate, SurfaceLivenessToken,
};

/// Failure to issue a Surface token through [`SurfaceControl`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceControlAttachError {
    /// Shutdown has begun; no generation may become live afterward.
    ShuttingDown,
    /// The private generation space has been exhausted.
    GenerationExhausted,
}

impl fmt::Display for SurfaceControlAttachError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShuttingDown => formatter.write_str("Surface host is shutting down"),
            Self::GenerationExhausted => formatter.write_str("Surface generation space exhausted"),
        }
    }
}

impl Error for SurfaceControlAttachError {}

impl From<SurfaceGenerationError> for SurfaceControlAttachError {
    fn from(_: SurfaceGenerationError) -> Self {
        Self::GenerationExhausted
    }
}

/// The direct render control sender is immutable once installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceControlInstallError;

impl fmt::Display for SurfaceControlInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Surface render-control sender is already installed")
    }
}

impl Error for SurfaceControlInstallError {}

/// Queue-independent authority for Surface generation and render teardown.
///
/// Release requests publish a monotonic generation high-water mark and nudge a
/// dedicated one-slot crossbeam stream. This avoids blocking detach, bounded
/// render-queue drops, and unbounded lifecycle allocation. A full wake slot is
/// safe because the receiver reads the authoritative high-water level; before
/// render installation the atomic itself is the cold pending queue.
pub struct SurfaceControl {
    gate: Arc<SurfaceGenerationGate>,
    shutting_down: AtomicBool,
    latest_retired: AtomicU64,
    render_wake: OnceLock<crossbeam_channel::Sender<()>>,
}

impl SurfaceControl {
    /// Creates an unattached control object before its Host is published.
    pub fn new() -> Self {
        Self {
            gate: Arc::new(SurfaceGenerationGate::new()),
            shutting_down: AtomicBool::new(false),
            latest_retired: AtomicU64::new(0),
            render_wake: OnceLock::new(),
        }
    }

    /// Issues or reuses the current live private generation.
    ///
    /// The second shutdown check closes the race where shutdown sets its level
    /// after the first check but before the gate CAS.  Any token issued in that
    /// window is immediately retired and its render teardown is requested.
    pub fn attach_or_update(&self) -> Result<SurfaceLivenessToken, SurfaceControlAttachError> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(SurfaceControlAttachError::ShuttingDown);
        }

        let token = self.gate.attach_or_update()?;
        if self.shutting_down.load(Ordering::Acquire) {
            self.retire_current_and_request();
            return Err(SurfaceControlAttachError::ShuttingDown);
        }
        Ok(token)
    }

    /// Retires the current generation and posts exactly one direct teardown
    /// request. `None` means it was already retired or never attached.
    pub fn retire_current_and_request(&self) -> Option<SurfaceGeneration> {
        let generation = self.gate.retire_current()?;
        // Concurrent retire/reattach callers can complete out of order. A
        // monotonic high-water mark prevents an older store from hiding a newer
        // retirement while allowing every queued wake to coalesce to one slot.
        self.latest_retired
            .fetch_max(generation.get(), Ordering::AcqRel);
        self.wake_render();
        Some(generation)
    }

    /// Retire exactly `expected`. A stale renderer failure must never retire a
    /// newer Surface that the host attached concurrently.
    pub fn retire_generation_and_request(&self, expected: SurfaceGeneration) -> bool {
        if !self.gate.retire_if_current(expected) {
            return false;
        }
        self.latest_retired
            .fetch_max(expected.get(), Ordering::AcqRel);
        self.wake_render();
        true
    }

    /// Publishes shutdown before retiring the current generation.  Repeated
    /// calls are idempotent and can only emit the first successful retirement.
    pub fn shutdown(&self) -> Option<SurfaceGeneration> {
        self.shutting_down.store(true, Ordering::Release);
        self.retire_current_and_request()
    }

    /// Returns whether this Host has permanently closed future attachment.
    #[inline]
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    /// Installs the render thread's dedicated coalescing wake and delivers any
    /// release level that raced with render initialization.
    pub fn install_render_sender(
        &self,
        sender: crossbeam_channel::Sender<()>,
    ) -> Result<(), SurfaceControlInstallError> {
        self.render_wake
            .set(sender)
            .map_err(|_| SurfaceControlInstallError)?;
        if self.latest_retired.load(Ordering::Acquire) != 0 {
            self.wake_render();
        }
        Ok(())
    }

    /// Highest generation known to be retired. Render control handles this
    /// level, not the wake payload, so a full one-slot channel loses no state.
    pub fn latest_retired_generation(&self) -> Option<SurfaceGeneration> {
        let generation = self.latest_retired.load(Ordering::Acquire);
        (generation != 0).then(|| SurfaceGeneration::from_non_zero_value(generation))
    }

    fn wake_render(&self) {
        if let Some(sender) = self.render_wake.get() {
            // The channel has capacity one. Full means an existing wake will
            // read the newer high-water mark; disconnected means render exit
            // already released all render-owned leases.
            let _ = sender.try_send(());
        }
    }
}

impl Default for SurfaceControl {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SurfaceControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceControl")
            .field("shutting_down", &self.is_shutting_down())
            .field("latest_retired", &self.latest_retired_generation())
            .field("render_wake_installed", &self.render_wake.get().is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::SurfaceControl;

    #[test]
    fn release_before_render_install_is_drained_in_generation_order() {
        let control = Arc::new(SurfaceControl::new());

        let first = control.attach_or_update().unwrap();
        assert_eq!(
            control.retire_current_and_request(),
            Some(first.generation())
        );
        let second = control.attach_or_update().unwrap();
        assert_eq!(
            control.retire_current_and_request(),
            Some(second.generation())
        );

        let (sender, receiver) = crossbeam_channel::bounded(1);
        control.install_render_sender(sender).unwrap();

        receiver.try_recv().unwrap();
        assert_eq!(control.latest_retired_generation().unwrap().get(), 2);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn post_install_release_goes_directly_to_render_control_stream() {
        let control = Arc::new(SurfaceControl::new());
        let (sender, receiver) = crossbeam_channel::bounded(1);
        control.install_render_sender(sender).unwrap();
        let token = control.attach_or_update().unwrap();

        assert_eq!(
            control.retire_current_and_request(),
            Some(token.generation())
        );

        receiver.try_recv().unwrap();
        assert_eq!(control.latest_retired_generation().unwrap().get(), 1);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn duplicate_retirement_does_not_emit_a_duplicate_release() {
        let control = Arc::new(SurfaceControl::new());
        let (sender, receiver) = crossbeam_channel::bounded(1);
        control.install_render_sender(sender).unwrap();
        control.attach_or_update().unwrap();

        assert!(control.retire_current_and_request().is_some());
        assert_eq!(control.retire_current_and_request(), None);

        receiver.try_recv().unwrap();
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn stale_conditional_retirement_cannot_close_a_newer_generation() {
        let control = SurfaceControl::new();
        let first = control.attach_or_update().unwrap();
        assert_eq!(
            control.retire_current_and_request(),
            Some(first.generation())
        );
        let second = control.attach_or_update().unwrap();

        assert!(!control.retire_generation_and_request(first.generation()));
        assert!(second.is_live());
        assert!(control.retire_generation_and_request(second.generation()));
        assert!(!second.is_live());
    }

    #[test]
    fn shutdown_retires_and_closes_future_attachment_without_host_queue_progress() {
        let control = Arc::new(SurfaceControl::new());
        let (sender, receiver) = crossbeam_channel::bounded(1);
        control.install_render_sender(sender).unwrap();
        let token = control.attach_or_update().unwrap();

        assert_eq!(control.shutdown(), Some(token.generation()));
        assert!(control.is_shutting_down());
        assert!(control.attach_or_update().is_err());

        receiver.try_recv().unwrap();
        assert_eq!(control.latest_retired_generation().unwrap().get(), 1);
    }

    #[test]
    fn render_sender_is_installed_exactly_once() {
        let control = SurfaceControl::new();
        let (first, _first_receiver) = crossbeam_channel::bounded(1);
        let (second, _second_receiver) = crossbeam_channel::bounded(1);

        control.install_render_sender(first).unwrap();

        assert!(control.install_render_sender(second).is_err());
    }
}
