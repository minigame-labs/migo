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
    pub fn signal(&self, ts_ms: f64) -> bool {
        match &self.0 {
            #[cfg(target_os = "android")]
            SenderInner::Eventfd { fd, timestamp } => {
                use std::os::fd::AsRawFd;
                use std::sync::atomic::Ordering;
                timestamp.store(ts_ms.to_bits(), Ordering::Release);
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
            SenderInner::Channel(tx) => tx.try_send(ts_ms).is_ok(),
        }
    }
}

impl RafReceiver {
    /// Wait for the next frame signal.  Returns timestamp in ms.
    ///
    /// Returns `None` only when the channel is truly closed (sender dropped).
    /// Spurious wakes (EAGAIN) are retried internally.
    pub async fn recv(&self) -> Option<f64> {
        match &self.0 {
            #[cfg(target_os = "android")]
            ReceiverInner::Eventfd {
                async_fd,
                fd,
                timestamp,
            } => {
                use std::os::fd::AsRawFd;
                use std::sync::atomic::Ordering;

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
                        let bits = timestamp.load(Ordering::Acquire);
                        return Some(f64::from_bits(bits));
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
                while let Ok(ts) = rx.try_recv() {
                    latest = ts;
                }
                Some(latest)
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
        timestamp: Arc<std::sync::atomic::AtomicU64>,
    },
    Channel(tokio::sync::mpsc::Sender<f64>),
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
        timestamp: Arc<std::sync::atomic::AtomicU64>,
    },
    Channel(tokio::sync::Mutex<tokio::sync::mpsc::Receiver<f64>>),
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

    let timestamp = Arc::new(std::sync::atomic::AtomicU64::new(0));

    Ok((
        RafSender(SenderInner::Eventfd {
            fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) },
            timestamp: timestamp.clone(),
        }),
        RafReceiver(ReceiverInner::Eventfd {
            async_fd: tokio::sync::OnceCell::new(),
            fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(fd2) },
            timestamp,
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
