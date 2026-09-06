use std::{
    error::Error,
    fmt,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use parking_lot::Mutex;

use super::{
    SurfaceGeneration, SurfaceGenerationError, SurfaceGenerationGate, SurfaceLease,
    SurfaceLivenessToken,
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
///
/// The candidate Surface is a level here for the same reason. Owning a
/// [`SurfaceLease`] pins the host's native Surface, and `RELEASED` is published by
/// the last one going away -- so wherever a lease waits, the host waits. It used
/// to wait in two places that cannot be hurried: handed to the render worker at
/// spawn, it was owned across EGL display, config and pbuffer-context
/// construction, which name no window and so provably cannot use it; carried
/// inside a `RecreateOnscreen` command, it sat in a bounded queue behind exactly
/// the same phase. Measured, that phase is 33 ms on macOS and 5.7-41 s on the iOS
/// simulator, where ANGLE compiles its Metal shaders cold, and for all of it
/// `migo_surface_begin_detach` could not complete.
///
/// Published here instead, the candidate is revoked by the retirement that made it
/// unusable, and a worker reads it when it can act on it. Reading is deliberately
/// non-destructive: the level is "the Surface this Host currently has", not a
/// message, so a worker claiming it at startup cannot consume the one an
/// `UpdateSurface` published while that startup was still running.
pub struct SurfaceControl {
    gate: Arc<SurfaceGenerationGate>,
    shutting_down: AtomicBool,
    latest_retired: AtomicU64,
    render_wake: OnceLock<crossbeam_channel::Sender<()>>,
    /// The Surface this Host currently has, if any. Revoked on retirement.
    candidate: Mutex<Option<SurfaceLease>>,
}

impl SurfaceControl {
    /// Creates an unattached control object before its Host is published.
    pub fn new() -> Self {
        Self {
            gate: Arc::new(SurfaceGenerationGate::new()),
            shutting_down: AtomicBool::new(false),
            latest_retired: AtomicU64::new(0),
            render_wake: OnceLock::new(),
            candidate: Mutex::new(None),
        }
    }

    /// Publish the Surface a render worker should install.
    ///
    /// Supersedes whatever was published before, which by then is a generation the
    /// gate has already retired -- a newer one is only mintable once the previous
    /// is detached.
    pub fn publish_candidate(&self, lease: SurfaceLease) {
        let superseded = self.candidate.lock().replace(lease);
        // Dropped outside the lock, always: see `release_dead_candidate`.
        drop(superseded);
        // A retirement that landed between minting this generation and publishing
        // it is honoured now, by the same rule every retirement uses, rather than
        // leaving a Surface nobody can install published until a worker looks.
        self.release_dead_candidate();
    }

    /// Read whatever Surface is currently live, with no expectation about which.
    ///
    /// For a worker starting up: it has not been told about a particular
    /// generation, so any live one is the one to install.
    ///
    /// `None` means the host took it back, which is not a failure: a session whose
    /// Surface was retired before the renderer could install one is in the state a
    /// warm start begins in, and the next attach installs exactly as an initial
    /// Surface would have.
    ///
    /// Non-destructive by design -- see the type's documentation.
    pub fn live_candidate(&self) -> Option<SurfaceLease> {
        // Cloning under the lock is safe where dropping is not: three Arc
        // increments, no destructor, no host code.
        self.candidate.lock().clone().filter(SurfaceLease::is_live)
    }

    /// Read the live Surface only if it is the generation the caller was told about.
    ///
    /// This exists as a separate entry point because a caller acting on a *request*
    /// must not adopt whatever happens to be published now. A recreate request can
    /// outlive its own candidate: `RenderService` gives up on the reply after 500 ms
    /// and the request stays queued, so by the time a worker reaches it the host may
    /// have detached that Surface and attached another. Reading the bare level there
    /// would install the new Surface under the old request's presentation parameters
    /// and reply on the old request's channel -- and if that install failed, the
    /// failure path would retire the generation the host had just attached.
    ///
    /// Naming the generation is what the payload used to do implicitly: a request
    /// carrying its own lease identified itself, and a stale one was rejected as a
    /// stale generation. Moving the Surface to a level took that away, so the
    /// request carries the one thing it needs to stay self-identifying -- a
    /// generation, which is `Copy` and pins nothing.
    pub fn live_candidate_for(&self, expected: SurfaceGeneration) -> Option<SurfaceLease> {
        self.live_candidate()
            .filter(|lease| lease.generation() == expected)
    }

    /// Drop the published Surface once it can never be installed.
    ///
    /// Only a retired generation is taken; a live one is still what the Host has.
    fn release_dead_candidate(&self) {
        let dead = {
            let mut published = self.candidate.lock();
            if published.as_ref().is_some_and(|lease| !lease.is_live()) {
                published.take()
            } else {
                None
            }
        };
        // The guard is gone before this drops, and that is load-bearing. This may
        // be the final resource lease, whose drop publishes RELEASED and
        // synchronously runs the host's release notification -- which a C embedder
        // is allowed to dispatch inline, on this thread. Dropping it under a
        // lifecycle mutex would invite the host straight back into a lock it is
        // already inside.
        drop(dead);
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
        // Before the wake, so a worker that is about to look finds the level in
        // the state this retirement leaves it in rather than one request behind.
        self.release_dead_candidate();
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
        self.release_dead_candidate();
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
    use std::sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    };

    use super::SurfaceControl;
    use crate::surface::{
        PublicSurfaceGeneration, Surface, SurfaceLease, SurfaceRef, SurfaceReleasePhase,
    };

    #[derive(Debug)]
    struct TestSurface;

    impl Surface for TestSurface {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn size(&self) -> (u32, u32) {
            (640, 480)
        }
    }

    /// The host's own lease for a freshly attached Surface, plus the level a render
    /// worker reads -- the state a session is in while its render thread builds EGL.
    fn attach_and_publish(control: &Arc<SurfaceControl>) -> SurfaceLease {
        let token = control.attach_or_update().unwrap();
        let surface: SurfaceRef = Arc::new(TestSurface);
        let host =
            SurfaceLease::new_tracked(surface, token, PublicSurfaceGeneration::new(1).unwrap());
        control.publish_candidate(host.clone());
        host
    }

    #[test]
    fn a_retirement_releases_the_published_surface_before_any_worker_exists() {
        // The property this whole arrangement exists for: a host can be told its
        // Surface is released while the renderer is still coming up. No render
        // sender is installed here and no worker ever claims anything, which is
        // exactly the situation on the iOS simulator for the 5.7-41 s that ANGLE
        // spends compiling Metal shaders.
        //
        // Both retirement entry points, because a host detach and a stale renderer
        // failure reach different ones and only one of them being wired would look
        // right from either side.
        for exact in [false, true] {
            let control = Arc::new(SurfaceControl::new());
            let host = attach_and_publish(&control);
            let pending = Arc::new(AtomicUsize::new(0));

            // The detach sequence the C boundary runs: prepare, retire, commit,
            // and only then let go of the host's own lease.
            let prepared = host.prepare_release(Arc::clone(&pending), None).unwrap();
            let retired = if exact {
                control.retire_generation_and_request(host.generation())
            } else {
                control.retire_current_and_request().is_some()
            };
            assert!(retired, "the generation must retire (exact={exact})");
            let release = prepared.commit();
            assert_eq!(
                release.phase(),
                SurfaceReleasePhase::Pending,
                "the host still holds its own lease at this point (exact={exact})"
            );
            drop(host);

            assert_eq!(
                release.phase(),
                SurfaceReleasePhase::Released,
                "the retirement must have let go of the published lease, or the host \
                 waits for a renderer that has not finished starting (exact={exact})"
            );
            assert_eq!(pending.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn a_candidate_published_after_its_generation_was_retired_is_dropped_at_once() {
        // Publishing happens on the caller's thread while a detach can be running on
        // another. A candidate that arrives already retired has no worker coming
        // for it, so nothing else would ever let it go.
        let control = Arc::new(SurfaceControl::new());
        let token = control.attach_or_update().unwrap();
        let surface: SurfaceRef = Arc::new(TestSurface);
        let host =
            SurfaceLease::new_tracked(surface, token, PublicSurfaceGeneration::new(1).unwrap());
        let pending = Arc::new(AtomicUsize::new(0));
        let prepared = host.prepare_release(Arc::clone(&pending), None).unwrap();
        assert!(control.retire_current_and_request().is_some());
        let release = prepared.commit();

        control.publish_candidate(host.clone());
        drop(host);

        assert_eq!(
            release.phase(),
            SurfaceReleasePhase::Released,
            "publishing a retired candidate must not resurrect the pin"
        );
    }

    #[test]
    fn a_worker_arriving_after_a_retirement_finds_no_candidate() {
        let control = Arc::new(SurfaceControl::new());
        let host = attach_and_publish(&control);

        assert!(control.retire_current_and_request().is_some());

        assert!(
            control.live_candidate().is_none(),
            "a retired candidate must never be given to a worker"
        );
        drop(host);
    }

    #[test]
    fn a_live_candidate_can_be_read_more_than_once() {
        // Load-bearing, and the reason reading is not a hand-off. A worker reads
        // this level twice on the one path that matters: once when GPU
        // initialization finishes, and again when it reaches the `RecreateOnscreen`
        // an `UpdateSurface` queued while that initialization was still running.
        // Consuming it on the first read would leave the second with nothing and
        // the session convinced its attach had failed.
        let control = Arc::new(SurfaceControl::new());
        let host = attach_and_publish(&control);
        let pending = Arc::new(AtomicUsize::new(0));
        let release = host
            .prepare_release(Arc::clone(&pending), None)
            .unwrap()
            .commit();

        let at_startup = control
            .live_candidate()
            .expect("the live candidate is what the worker installs");
        let at_update = control
            .live_candidate()
            .expect("and it is still what the Host has when the worker looks again");
        assert_eq!(at_startup.size(), (640, 480));
        assert_eq!(at_startup.generation(), at_update.generation());

        // The pin now sits with the worker as well, which is correct: past this
        // point it is about to name the Surface in an EGL call. Only when the
        // retirement has revoked the level and every reader has let go is the host
        // told the Surface is its own again.
        drop(host);
        assert!(control.retire_current_and_request().is_some());
        assert_eq!(release.phase(), SurfaceReleasePhase::Pending);
        drop(at_startup);
        assert_eq!(release.phase(), SurfaceReleasePhase::Pending);
        drop(at_update);
        assert_eq!(release.phase(), SurfaceReleasePhase::Released);
    }

    #[test]
    fn the_published_surface_is_dropped_outside_the_lifecycle_lock() {
        // Letting go of the published lease can be the drop that publishes
        // RELEASED, and that drop runs the host's release notification
        // synchronously -- a C embedder is allowed to dispatch it inline, on this
        // thread. If it ran under the candidate lock, a host that touched its
        // session from that callback would meet a lock this thread is already
        // inside.
        //
        // Asserted from the notification itself, where the answer is not a matter
        // of reading the code: `parking_lot::Mutex` is not reentrant, so a
        // successful `try_lock` here means the guard was already gone.
        let control = Arc::new(SurfaceControl::new());
        let observer: Weak<SurfaceControl> = Arc::downgrade(&control);
        let unlocked = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&unlocked);

        let token = control.attach_or_update().unwrap();
        let surface: SurfaceRef = Arc::new(TestSurface);
        let host =
            SurfaceLease::new_tracked(surface, token, PublicSurfaceGeneration::new(1).unwrap());
        control.publish_candidate(host.clone());
        let pending = Arc::new(AtomicUsize::new(0));
        let release = host
            .prepare_release(
                Arc::clone(&pending),
                Some(Box::new(move |_| {
                    if let Some(control) = observer.upgrade() {
                        // Deliberately not `lock()`: this must report, not hang.
                        if control.candidate.try_lock().is_some() {
                            seen.fetch_add(1, Ordering::AcqRel);
                        }
                    }
                })),
            )
            .unwrap()
            .commit();

        // Let the host go first, so the published clone is the last lease and its
        // drop is the one that fires the notification.
        drop(host);
        assert_eq!(release.phase(), SurfaceReleasePhase::Pending);
        assert!(control.retire_current_and_request().is_some());

        assert_eq!(release.phase(), SurfaceReleasePhase::Released);
        assert_eq!(
            unlocked.load(Ordering::Acquire),
            1,
            "the release notification ran while the candidate lock was held"
        );
    }

    #[test]
    fn a_request_that_outlived_its_candidate_does_not_adopt_the_next_one() {
        // The failure this prevents, in the order it happens: `update_surface`
        // publishes generation 1 and queues a recreate; the reply times out after
        // 500 ms while the request stays queued; the host detaches 1 and attaches 2,
        // which publishes 2 and queues its own recreate. The worker then reaches the
        // *first* request. Reading the bare level there hands it generation 2 -- so
        // it would install the host's new Surface under the old request's
        // presentation parameters, answer on the old request's channel, and if that
        // install failed, retire the generation the host had just attached.
        //
        // A request carrying its own lease used to be self-identifying, and this is
        // what replaced that.
        let control = Arc::new(SurfaceControl::new());
        let first = attach_and_publish(&control);
        let stale_request = first.generation();

        assert!(control.retire_current_and_request().is_some());
        drop(first);
        let second = attach_and_publish(&control);
        assert_ne!(second.generation(), stale_request);

        assert!(
            control.live_candidate_for(stale_request).is_none(),
            "a superseded request must be refused, not served the new Surface"
        );
        assert_eq!(
            control
                .live_candidate_for(second.generation())
                .map(|lease| lease.generation()),
            Some(second.generation()),
            "and the request that names the live generation must still be served"
        );
        // The bare read stays available for the one caller with no expectation to
        // check: a worker starting up, which installs whatever is live.
        assert_eq!(
            control.live_candidate().map(|lease| lease.generation()),
            Some(second.generation())
        );
        drop(second);
    }

    #[test]
    fn a_candidate_whose_generation_died_before_revocation_is_refused() {
        // Revocation cannot close this window on its own: a retirement can land
        // between the gate transition and the revocation that follows it, and a
        // worker looking in that gap sees a published Surface whose generation is
        // already gone. Reproduced by retiring the gate directly, which is what
        // that gap looks like from the level's side.
        //
        // Without the liveness filter the worker would carry a dead Surface as far
        // as `preflight`, which does reject it -- so the visible outcome would be
        // the same and only the pin would last longer. That is exactly why it needs
        // a test: a redundant guard nobody can see fail is a guard someone deletes.
        let control = Arc::new(SurfaceControl::new());
        let host = attach_and_publish(&control);
        let pending = Arc::new(AtomicUsize::new(0));
        let prepared = host.prepare_release(Arc::clone(&pending), None).unwrap();
        control
            .gate
            .retire_current()
            .expect("the generation must have been live");
        let release = prepared.commit();

        assert!(
            control.live_candidate().is_none(),
            "a candidate whose generation died must be refused"
        );

        // And the revocation that follows the gap releases it, which is the pairing
        // every retirement entry point is wired for.
        drop(host);
        assert_eq!(release.phase(), SurfaceReleasePhase::Pending);
        control.release_dead_candidate();
        assert_eq!(release.phase(), SurfaceReleasePhase::Released);
    }

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
