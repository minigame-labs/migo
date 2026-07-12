//! RAF (requestAnimationFrame) frame-timestamp signaling.
//!
//! The render thread signals "frame ready" with a timestamp; the host thread
//! (JS async op) waits for it.
//!
//! - **Android/Linux**: `eventfd` + `tokio::io::unix::AsyncFd` — true epoll wait,
//!   zero CPU when idle, ~1-3 µs wake latency.
//! - **Other platforms**: `tokio::sync::mpsc::channel(2)` — current path, unchanged.

use std::sync::Arc;

/// Sender half — lives on the render thread (native OS thread).
pub struct RafSender(SenderInner);

/// Receiver half — lives on the host thread (tokio async).
/// Wrapped in `Arc` for restart survival.
pub struct RafReceiver(ReceiverInner);

impl RafSender {
    /// Signal the next frame timestamp (milliseconds).
    ///
    /// Returns `true` if the signal was delivered, `false` if it was dropped
    /// (channel full or write error).  The caller should count drops for
    /// debug stats.
    pub fn signal(&self, ts_ms: f64, ticket: u64) -> bool {
        match &self.0 {
            #[cfg(target_os = "android")]
            SenderInner::Eventfd { fd, frame } => {
                use std::os::fd::AsRawFd;
                *frame.lock() = RafFrame { ts_ms, ticket };
                let val: u64 = 1;
                loop {
                    let ret = unsafe {
                        libc::write(fd.as_raw_fd(), &val as *const u64 as *const libc::c_void, 8)
                    };
                    if ret == 8 {
                        return true;
                    }
                    // Retry on EINTR; give up on any other error.
                    let errno = std::io::Error::last_os_error();
                    if errno.raw_os_error() != Some(libc::EINTR) {
                        return false;
                    }
                }
            }
            SenderInner::Channel(tx) => tx.try_send(RafFrame { ts_ms, ticket }).is_ok(),
        }
    }
}

impl RafReceiver {
    /// Wait for the frame signal matching `expected_ticket`.
    ///
    /// Returns `None` only when the channel is truly closed (sender dropped).
    /// Spurious wakes (EAGAIN) and stale signals from a cancelled runtime are
    /// retried internally. Ticket matching prevents a soft restart from
    /// consuming a timestamp produced for the old isolate.
    pub async fn recv(&self, expected_ticket: u64) -> Option<f64> {
        loop {
            let frame = match &self.0 {
                #[cfg(target_os = "android")]
                ReceiverInner::Eventfd {
                    async_fd,
                    fd,
                    frame,
                } => {
                    use std::os::fd::AsRawFd;

                    let raw_fd = fd.as_raw_fd();
                    // Register the fd with epoll once and reuse it every frame.
                    // Recreating an `AsyncFd` per `recv()` did an epoll add+remove on
                    // every RAF wait. Lazily initialised because the first `recv()`
                    // is the earliest point a tokio reactor is guaranteed current;
                    // the host tokio runtime (and thus this registration) survives
                    // soft restarts, so the cached handle stays valid.
                    let async_fd = async_fd
                        .get_or_try_init(|| async {
                            tokio::io::unix::AsyncFd::with_interest(
                                raw_fd,
                                tokio::io::Interest::READABLE,
                            )
                        })
                        .await
                        .ok()?;

                    // Loop until we get a real read or a fatal error.
                    // EAGAIN (spurious wake) just re-enters the readable wait.
                    loop {
                        let mut guard = async_fd.readable().await.ok()?;

                        let mut buf = [0u8; 8];
                        let ret =
                            unsafe { libc::read(raw_fd, buf.as_mut_ptr() as *mut libc::c_void, 8) };
                        guard.clear_ready();

                        if ret == 8 {
                            break *frame.lock();
                        }

                        let errno = std::io::Error::last_os_error();
                        if errno.raw_os_error() == Some(libc::EAGAIN)
                            || errno.raw_os_error() == Some(libc::EINTR)
                        {
                            // Spurious wake or signal interrupt — retry.
                            continue;
                        }

                        // Real error (e.g. EBADF) — treat as closed.
                        return None;
                    }
                }
                ReceiverInner::Channel(rx) => {
                    let mut rx = rx.lock().await;
                    let first = rx.recv().await?;
                    // Coalesce to the newest queued timestamp so RAF uses the latest
                    // frame time, matching the eventfd path (which collapses multiple
                    // signals into one wake with the newest ts via the atomic). The
                    // bounded(2) channel would otherwise hand back a stale buffered
                    // frame first when the consumer briefly falls behind.
                    let mut latest = first;
                    while let Ok(frame) = rx.try_recv() {
                        latest = frame;
                    }
                    latest
                }
            };

            if frame_matches_ticket(frame.ticket, expected_ticket) {
                return Some(frame.ts_ms);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal enum dispatch
// ---------------------------------------------------------------------------

enum SenderInner {
    #[cfg(target_os = "android")]
    Eventfd {
        fd: std::os::fd::OwnedFd,
        frame: Arc<parking_lot::Mutex<RafFrame>>,
    },
    Channel(tokio::sync::mpsc::Sender<RafFrame>),
}

enum ReceiverInner {
    #[cfg(target_os = "android")]
    Eventfd {
        /// Cached epoll registration for `fd`, created on the first `recv()`
        /// (needs a live tokio reactor) and reused every frame to avoid a
        /// per-frame epoll add/remove. Declared before `fd` so it deregisters
        /// before the fd is closed on drop.
        async_fd: tokio::sync::OnceCell<tokio::io::unix::AsyncFd<std::os::fd::RawFd>>,
        fd: std::os::fd::OwnedFd,
        frame: Arc<parking_lot::Mutex<RafFrame>>,
    },
    Channel(tokio::sync::Mutex<tokio::sync::mpsc::Receiver<RafFrame>>),
}

#[derive(Clone, Copy, Debug)]
struct RafFrame {
    ts_ms: f64,
    ticket: u64,
}

#[inline]
fn frame_matches_ticket(delivered_ticket: u64, expected_ticket: u64) -> bool {
    delivered_ticket == expected_ticket
}

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

/// Create a matched (sender, receiver) pair.
///
/// On Android: uses eventfd for low-latency, low-power wake.
/// Falls back to tokio mpsc channel on failure or other platforms.
pub fn create_raf_pair() -> (RafSender, Arc<RafReceiver>) {
    #[cfg(target_os = "android")]
    {
        match create_eventfd_pair() {
            Ok((tx, rx)) => {
                tracing::info!("RAF signal: using eventfd");
                return (tx, Arc::new(rx));
            }
            Err(e) => {
                tracing::warn!("RAF eventfd init failed ({e}), falling back to channel");
            }
        }
    }

    let (tx, rx) = create_channel_pair();
    tracing::info!("RAF signal: using tokio mpsc channel");
    (tx, Arc::new(rx))
}

#[cfg(target_os = "android")]
fn create_eventfd_pair() -> Result<(RafSender, RafReceiver), String> {
    use std::os::fd::FromRawFd;

    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!("eventfd(): {}", std::io::Error::last_os_error()));
    }

    let fd2 = unsafe { libc::dup(fd) };
    if fd2 < 0 {
        unsafe { libc::close(fd) };
        return Err(format!("dup(eventfd): {}", std::io::Error::last_os_error()));
    }

    let frame = Arc::new(parking_lot::Mutex::new(RafFrame {
        ts_ms: 0.0,
        ticket: 0,
    }));

    Ok((
        RafSender(SenderInner::Eventfd {
            fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) },
            frame: frame.clone(),
        }),
        RafReceiver(ReceiverInner::Eventfd {
            async_fd: tokio::sync::OnceCell::new(),
            fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(fd2) },
            frame,
        }),
    ))
}

fn create_channel_pair() -> (RafSender, RafReceiver) {
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    (
        RafSender(SenderInner::Channel(tx)),
        RafReceiver(ReceiverInner::Channel(tokio::sync::Mutex::new(rx))),
    )
}

// ---------------------------------------------------------------------------
// RAF demand latch
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicU64, Ordering};

/// Tracks whether a JS RAF consumer is actually awaiting a frame signal.
///
/// `op_await_next_frame` publishes demand (`mark_waiting`) before awaiting; the
/// render thread consumes it (`take_waiter`) and only then signals RAF, so a
/// dirty-only / upload-only frame never writes an unconsumed timestamp. On a
/// failed signal the render thread restores demand so RAF cannot freeze.
///
/// Shared via `Arc` between the host op (`HostOpState`) and the render thread;
/// survives JS soft restart the same way [`RafReceiver`] does.
#[derive(Debug, Default)]
pub struct RafDemand {
    next_ticket: AtomicU64,
    waiter_ticket: AtomicU64,
}

impl RafDemand {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish demand before awaiting (op side).
    #[inline]
    pub fn mark_waiting(&self) -> u64 {
        let previous = self
            .next_ticket
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(if current == u64::MAX { 1 } else { current + 1 })
            })
            .expect("RAF ticket update always succeeds");
        let ticket = if previous == u64::MAX {
            1
        } else {
            previous + 1
        };
        self.waiter_ticket.store(ticket, Ordering::Release);
        ticket
    }

    /// Non-consuming read (render demand check / tests).
    #[inline]
    pub fn is_waiting(&self) -> bool {
        self.waiter_ticket.load(Ordering::Acquire) != 0
    }

    /// Consume the waiter (render side, before signalling). Returns its ticket;
    /// a second consecutive call returns `None` so a dirty-only frame
    /// following a signalled one never signals twice.
    #[inline]
    pub fn take_waiter(&self) -> Option<u64> {
        let ticket = self.waiter_ticket.swap(0, Ordering::AcqRel);
        (ticket != 0).then_some(ticket)
    }

    /// Re-arm demand after a failed signal so RAF is not frozen.
    #[inline]
    pub fn restore_waiter(&self, ticket: u64) {
        let _ =
            self.waiter_ticket
                .compare_exchange(0, ticket, Ordering::Release, Ordering::Relaxed);
    }
}

/// Shared handle to the RAF demand latch.
pub type RafDemandRef = Arc<RafDemand>;

#[cfg(test)]
mod demand_tests {
    use super::{RafDemand, create_channel_pair, frame_matches_ticket};

    #[test]
    fn new_is_not_waiting() {
        let d = RafDemand::new();
        assert!(!d.is_waiting());
        assert_eq!(d.take_waiter(), None, "no waiter to take");
    }

    #[test]
    fn mark_then_take_consumes_exactly_once() {
        let d = RafDemand::new();
        let ticket = d.mark_waiting();
        assert!(d.is_waiting());
        assert_eq!(
            d.take_waiter(),
            Some(ticket),
            "first take consumes the waiter"
        );
        assert!(!d.is_waiting(), "consumed");
        assert!(
            d.take_waiter().is_none(),
            "second take sees no waiter (no double signal)"
        );
    }

    #[test]
    fn restore_after_failed_signal_rearms() {
        let d = RafDemand::new();
        let ticket = d.mark_waiting();
        assert_eq!(d.take_waiter(), Some(ticket));
        d.restore_waiter(ticket); // signal failed
        assert!(d.is_waiting(), "restored demand so RAF is not frozen");
        assert_eq!(d.take_waiter(), Some(ticket), "retry consumes same ticket");
    }

    #[test]
    fn stale_restore_does_not_overwrite_newer_waiter() {
        let d = RafDemand::new();
        let old = d.mark_waiting();
        assert_eq!(d.take_waiter(), Some(old));
        let newer = d.mark_waiting();
        d.restore_waiter(old);
        assert_eq!(d.take_waiter(), Some(newer));
    }

    #[test]
    fn receiver_ticket_filter_rejects_soft_restart_stale_signal() {
        assert!(!frame_matches_ticket(7, 8));
        assert!(frame_matches_ticket(8, 8));
    }

    #[test]
    fn channel_receiver_waits_past_stale_signal_for_current_ticket() {
        let (tx, rx) = create_channel_pair();
        assert!(tx.signal(1.0, 7));
        let producer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            assert!(tx.signal(2.0, 8));
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime");
        assert_eq!(runtime.block_on(rx.recv(8)), Some(2.0));
        producer.join().expect("producer thread");
    }
}
