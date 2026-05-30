use std::{
    collections::BinaryHeap,
    fmt,
    sync::{Arc, Condvar, Mutex as StdMutex, OnceLock, mpsc},
    thread,
};

use parking_lot::Mutex;
use shared::error::{EngineError, ErrorCode};

use crate::task::{PoolKind, PriorityClass};

// ---------------------------------------------------------------------------
// Priority channel — BinaryHeap + Mutex + Condvar
// ---------------------------------------------------------------------------

struct PriorityEntry<T> {
    priority: PriorityClass,
    seq: u64,
    value: T,
}

impl<T> Eq for PriorityEntry<T> {}
impl<T> PartialEq for PriorityEntry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}
impl<T> Ord for PriorityEntry<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}
impl<T> PartialOrd for PriorityEntry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct PriorityChannelInner<T> {
    heap: BinaryHeap<PriorityEntry<T>>,
    next_seq: u64,
    /// Monotonic pop counter used by the aging logic. Every
    /// [`PriorityReceiver::AGING_INTERVAL`] pops, if the heap holds
    /// any [`PriorityClass::Background`] entry while also holding a
    /// higher-priority entry, we surface the oldest `Background`
    /// first. This bounds the worst-case waiting time of a background
    /// task to roughly `AGING_INTERVAL` high-priority tasks, rather
    /// than "forever if foreground pressure never subsides".
    pop_count: u64,
    closed: bool,
}

struct PriorityChannelShared<T> {
    state: StdMutex<PriorityChannelInner<T>>,
    condvar: Condvar,
    sender_count: std::sync::atomic::AtomicUsize,
}

pub(crate) struct PrioritySender<T> {
    shared: Arc<PriorityChannelShared<T>>,
}

impl<T> Clone for PrioritySender<T> {
    fn clone(&self) -> Self {
        self.shared
            .sender_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for PrioritySender<T> {
    fn drop(&mut self) {
        if self
            .shared
            .sender_count
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
            == 1
        {
            // Last sender dropped — close the channel.
            let mut state = self.shared.state.lock().unwrap();
            state.closed = true;
            self.shared.condvar.notify_all();
        }
    }
}

pub(crate) struct PriorityReceiver<T> {
    shared: Arc<PriorityChannelShared<T>>,
}

pub(crate) fn priority_channel<T>() -> (PrioritySender<T>, PriorityReceiver<T>) {
    let shared = Arc::new(PriorityChannelShared {
        state: StdMutex::new(PriorityChannelInner {
            heap: BinaryHeap::new(),
            next_seq: 0,
            pop_count: 0,
            closed: false,
        }),
        condvar: Condvar::new(),
        sender_count: std::sync::atomic::AtomicUsize::new(1),
    });
    (
        PrioritySender {
            shared: Arc::clone(&shared),
        },
        PriorityReceiver { shared },
    )
}

impl<T> PrioritySender<T> {
    pub fn send(&self, priority: PriorityClass, value: T) -> Result<(), PoolError> {
        let mut state = self.shared.state.lock().unwrap();
        if state.closed {
            return Err(PoolError::Closed);
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        state.heap.push(PriorityEntry {
            priority,
            value,
            seq,
        });
        self.shared.condvar.notify_one();
        Ok(())
    }

    pub fn close(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.closed = true;
        self.shared.condvar.notify_all();
    }
}

impl<T> PriorityReceiver<T> {
    /// How often the aging policy pre-empts strict priority. Every
    /// `AGING_INTERVAL` successful pops, if the heap holds at least
    /// one `Background` entry *and* at least one higher-priority
    /// entry, we dequeue the oldest `Background` instead of the
    /// usual highest-priority-first pick.
    ///
    /// 16 is empirically large enough that foreground latency is
    /// dominated by the job itself (a 16th slot every cycle of small
    /// foreground work is ≤ 1 / 16 ≈ 6 % throughput taxation) and
    /// small enough that background tasks make steady progress under
    /// sustained foreground load.
    pub const AGING_INTERVAL: u64 = 16;

    pub fn recv(&self) -> Result<T, PoolError> {
        let mut state = self.shared.state.lock().unwrap();
        loop {
            // Aging slot: every Nth pop, prefer the oldest Background
            // entry if one is behind higher-priority work.
            let aging_due = state.pop_count != 0 && (state.pop_count % Self::AGING_INTERVAL == 0);
            if aging_due {
                if let Some(entry) = take_oldest_background(&mut state.heap) {
                    state.pop_count = state.pop_count.wrapping_add(1);
                    return Ok(entry.value);
                }
            }
            if let Some(entry) = state.heap.pop() {
                state.pop_count = state.pop_count.wrapping_add(1);
                return Ok(entry.value);
            }
            if state.closed {
                return Err(PoolError::Closed);
            }
            state = self.shared.condvar.wait(state).unwrap();
        }
    }
}

/// Surface the oldest `Background` entry currently waiting in the
/// heap, provided at least one higher-priority entry is present. If
/// there are no `Background` entries, or if the heap is exclusively
/// `Background`, returns `None` so the normal `pop` path handles it.
///
/// `BinaryHeap` has no indexed removal, so this drains and rebuilds
/// the heap. It is O(N) on the size of the heap, but only runs on
/// aging pulses (1 in `AGING_INTERVAL` pops) and typical IO-pool
/// queues hold ≤ 100 entries at peak.
fn take_oldest_background<T>(heap: &mut BinaryHeap<PriorityEntry<T>>) -> Option<PriorityEntry<T>> {
    if heap.is_empty() {
        return None;
    }
    let items: Vec<PriorityEntry<T>> = std::mem::take(heap).into_sorted_vec();
    let has_higher = items
        .iter()
        .any(|e| e.priority != PriorityClass::Background);
    if !has_higher {
        // Nothing to pre-empt for; put them back and let `pop` run.
        for e in items {
            heap.push(e);
        }
        return None;
    }
    let mut oldest_bg_seq: Option<u64> = None;
    for e in &items {
        if e.priority == PriorityClass::Background {
            oldest_bg_seq = Some(match oldest_bg_seq {
                None => e.seq,
                Some(s) => s.min(e.seq),
            });
        }
    }
    let oldest_bg_seq = match oldest_bg_seq {
        Some(s) => s,
        None => {
            for e in items {
                heap.push(e);
            }
            return None;
        }
    };
    let mut picked = None;
    for e in items {
        if picked.is_none() && e.priority == PriorityClass::Background && e.seq == oldest_bg_seq {
            picked = Some(e);
        } else {
            heap.push(e);
        }
    }
    picked
}

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolError {
    Closed,
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolError::Closed => f.write_str("IO worker pool closed"),
        }
    }
}

impl From<PoolError> for EngineError {
    fn from(err: PoolError) -> Self {
        match err {
            PoolError::Closed => {
                EngineError::new(ErrorCode::IoError).with_detail("IO worker pool closed")
            }
        }
    }
}

pub struct JobHandle<T> {
    rx: mpsc::Receiver<std::thread::Result<T>>,
}

impl<T> JobHandle<T> {
    pub fn join(self) -> Result<T, PoolError> {
        match self.rx.recv().map_err(|_| PoolError::Closed)? {
            Ok(value) => Ok(value),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

#[derive(Clone)]
pub struct WorkerPool {
    inner: Arc<WorkerPoolInner>,
}

struct WorkerPoolInner {
    name: String,
    tx: Mutex<Option<PrioritySender<Job>>>,
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    thread_count: usize,
}

impl WorkerPool {
    /// Single-threaded pool.  Kept as the default because most pool
    /// kinds (fs / pack / archive) aren't CPU-bound and don't benefit
    /// from fan-out — one worker keeps ordering predictable and
    /// serialises bursts against the shared pack/VFS layer.
    pub fn new(host_id: i32, label: &'static str) -> Self {
        Self::with_threads(host_id, label, 1)
    }

    /// Multi-threaded pool.  Use for CPU-heavy work that is safe to
    /// run concurrently (image decode + upload staging).  Threads
    /// share one [`PrioritySender`]/[`PriorityReceiver`] pair; since
    /// `PriorityReceiver::recv` serialises on the heap's internal
    /// mutex, the N workers dequeue atomically — no duplicate
    /// delivery, no cross-worker coordination needed from callers.
    ///
    /// `thread_count` is clamped to `≥ 1`; `with_threads(h, l, 0)` is
    /// treated as `with_threads(h, l, 1)` rather than panicking.
    pub fn with_threads(host_id: i32, label: &'static str, thread_count: usize) -> Self {
        let n = thread_count.max(1);
        let name = format!("io-{label}-host-{host_id}");
        let (tx, rx) = priority_channel::<Job>();
        let rx = Arc::new(rx);

        let mut workers = Vec::with_capacity(n);
        for worker_idx in 0..n {
            // When there's only one thread keep the legacy name so
            // telemetry / log greps keep matching (existing tests
            // assert on "io-{label}-host-{id}"). Multi-thread pools
            // get a -{idx} suffix so logs can tell threads apart.
            let thread_name = if n == 1 {
                name.clone()
            } else {
                format!("{name}-{worker_idx}")
            };
            let rx_clone = Arc::clone(&rx);
            let handle = thread::Builder::new()
                .name(thread_name)
                .spawn(move || {
                    while let Ok(job) = rx_clone.recv() {
                        job();
                    }
                })
                .expect("failed to spawn IO worker pool thread");
            workers.push(handle);
        }

        Self {
            inner: Arc::new(WorkerPoolInner {
                name,
                tx: Mutex::new(Some(tx)),
                workers: Mutex::new(workers),
                thread_count: n,
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Actual thread count this pool was configured with.
    pub fn thread_count(&self) -> usize {
        self.inner.thread_count
    }

    pub fn submit<T, F>(&self, priority: PriorityClass, job: F) -> Result<JobHandle<T>, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let sender = self.inner.tx.lock().clone().ok_or(PoolError::Closed)?;
        sender
            .send(
                priority,
                Box::new(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    let _ = result_tx.send(result);
                }),
            )
            .map_err(|_| PoolError::Closed)?;

        Ok(JobHandle { rx: result_rx })
    }

    pub fn run<T, F>(&self, priority: PriorityClass, job: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.submit(priority, job)?.join()
    }

    pub fn submit_async<T, F>(
        &self,
        priority: PriorityClass,
        job: F,
    ) -> Result<tokio::sync::oneshot::Receiver<std::thread::Result<T>>, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let sender = self.inner.tx.lock().clone().ok_or(PoolError::Closed)?;
        sender
            .send(
                priority,
                Box::new(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    let _ = result_tx.send(result);
                }),
            )
            .map_err(|_| PoolError::Closed)?;
        Ok(result_rx)
    }
}

impl Drop for WorkerPoolInner {
    fn drop(&mut self) {
        // Close the channel first so every worker's recv() returns
        // Err(Closed) and the outer loop exits.  Joining before the
        // close would deadlock: the worker would be parked in
        // `condvar.wait`.
        if let Some(tx) = self.tx.lock().take() {
            tx.close();
        }
        // Collect the handles out of the Mutex so we can join
        // without holding any lock a worker might need. Skip the
        // handle for the current thread — self-join would hang, and
        // the common case that triggers it is an outer pool owner
        // being dropped from inside a job closure.
        let handles: Vec<_> = std::mem::take(&mut *self.workers.lock());
        let current_id = thread::current().id();
        for h in handles {
            if h.thread().id() != current_id {
                let _ = h.join();
            }
        }
    }
}

impl fmt::Debug for WorkerPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerPool")
            .field("name", &self.inner.name)
            .finish()
    }
}

#[derive(Debug)]
struct LazyWorkerPool {
    host_id: i32,
    label: &'static str,
    thread_count: usize,
    pool: OnceLock<WorkerPool>,
}

impl LazyWorkerPool {
    fn new(host_id: i32, label: &'static str) -> Self {
        Self::with_threads(host_id, label, 1)
    }

    fn with_threads(host_id: i32, label: &'static str, thread_count: usize) -> Self {
        Self {
            host_id,
            label,
            thread_count: thread_count.max(1),
            pool: OnceLock::new(),
        }
    }

    fn get(&self) -> &WorkerPool {
        self.pool
            .get_or_init(|| WorkerPool::with_threads(self.host_id, self.label, self.thread_count))
    }

    #[cfg(test)]
    fn is_spawned(&self) -> bool {
        self.pool.get().is_some()
    }
}

/// Pick a sensible parallelism level for the image decode pool.
/// Two threads is a floor so even single-core emulators benefit
/// marginally from overlapping the JNI hop with a second decode;
/// past four threads the contention against host+render+audio
/// threads outweighs the JNI throughput gain on mobile.
///
/// The `num_cpus` crate isn't a workspace dependency; the std-only
/// alternative [`std::thread::available_parallelism`] returns the
/// same answer for our needs (after the OS accounts for the
/// process cgroup limits Android applies).
fn cpu_hint() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
}

fn default_image_thread_count() -> usize {
    let cpu_hint = cpu_hint();
    // Reserve one core for the host thread and one for the render
    // thread; cap at 4 because two extra concurrent JNI
    // `AttachCurrentThread` calls already saturate the BitmapFactory
    // Java-heap pressure on mid-tier devices.
    cpu_hint.saturating_sub(2).clamp(2, 4)
}

#[derive(Debug, Clone)]
pub struct IoPools {
    fs: Arc<LazyWorkerPool>,
    pack: Arc<LazyWorkerPool>,
    image: Arc<LazyWorkerPool>,
    archive: Arc<LazyWorkerPool>,
}

/// Filesystem reads on Android internal storage benefit from light
/// parallelism: the kernel block layer reorders concurrent positional
/// reads, and on warm page-cache hits the cost is dominated by
/// per-call syscall overhead which fans out cleanly across threads.
/// Cocos shop pages submit 30+ readFiles in a single tick.
///
/// Cap raised from 4 → 8 (2026-05) after `[Async] readFile` lines on
/// shop open consistently sat around 70–130 ms while the in-pool
/// `[IOTrace] read slow ≥30ms` warning never fired — meaning the
/// disk read itself was fast and the latency was pool-queue wait.
/// 30 in-flight requests across 4 threads = ~7 reqs per thread,
/// even a 5 ms per-read = 35 ms tail; with 8 threads it's 4 reqs ×
/// 5 ms = 20 ms tail.  Floor stays at 2 for single-core emulators.
fn default_fs_thread_count() -> usize {
    cpu_hint().saturating_sub(2).clamp(2, 8)
}

/// Pack reads decompress zstd chunks (CPU-bound) before returning, and
/// the dominant menu-switch workload is many small entries fired in
/// parallel. Fan out so they decompress concurrently. Cap at 4 because
/// past that we contend with host + render threads on smaller phones.
fn default_pack_thread_count() -> usize {
    cpu_hint().saturating_sub(2).clamp(2, 4)
}

impl IoPools {
    pub fn new(host_id: i32) -> Self {
        Self {
            fs: Arc::new(LazyWorkerPool::with_threads(
                host_id,
                "fs",
                default_fs_thread_count(),
            )),
            pack: Arc::new(LazyWorkerPool::with_threads(
                host_id,
                "pack",
                default_pack_thread_count(),
            )),
            // Image decode is CPU-bound (JPEG IDCT, PNG inflate) and
            // trivially parallel across distinct input buffers — the
            // pool fans out so a cold-start screen of 20 images
            // doesn't serialise behind a single BitmapFactory
            // invocation. Kept lazy: spawning only happens on the
            // first `get()`, so sessions that never draw an image
            // pay zero.
            image: Arc::new(LazyWorkerPool::with_threads(
                host_id,
                "image",
                default_image_thread_count(),
            )),
            // Archive (zip extract) is one-shot startup work that we
            // intentionally serialise so it doesn't compete with
            // foreground reads for CPU during a hot session.
            archive: Arc::new(LazyWorkerPool::new(host_id, "archive")),
        }
    }

    fn get_pool(&self, pool: PoolKind) -> &WorkerPool {
        match pool {
            PoolKind::Fs => self.fs.get(),
            PoolKind::Pack => self.pack.get(),
            PoolKind::Image => self.image.get(),
            PoolKind::Archive => self.archive.get(),
        }
    }

    pub fn run<T, F>(&self, pool: PoolKind, priority: PriorityClass, job: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.get_pool(pool).run(priority, job)
    }

    pub fn submit_async<T, F>(
        &self,
        pool: PoolKind,
        priority: PriorityClass,
        job: F,
    ) -> Result<tokio::sync::oneshot::Receiver<std::thread::Result<T>>, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.get_pool(pool).submit_async(priority, job)
    }

    #[cfg(test)]
    pub(crate) fn spawned_pool_count(&self) -> usize {
        [
            self.fs.is_spawned(),
            self.pack.is_spawned(),
            self.image.is_spawned(),
            self.archive.is_spawned(),
        ]
        .into_iter()
        .filter(|spawned| *spawned)
        .count()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    use crate::{
        pools::{IoPools, PoolError, priority_channel},
        task::{PoolKind, PriorityClass},
    };

    #[test]
    fn priority_channel_delivers_highest_priority_first() {
        let (tx, rx) = priority_channel::<String>();

        tx.send(PriorityClass::Background, "bg".to_string())
            .unwrap();
        tx.send(PriorityClass::ForegroundAsync, "fg-async".to_string())
            .unwrap();
        tx.send(PriorityClass::ForegroundBlocking, "fg-block".to_string())
            .unwrap();

        assert_eq!(rx.recv().unwrap(), "fg-block");
        assert_eq!(rx.recv().unwrap(), "fg-async");
        assert_eq!(rx.recv().unwrap(), "bg");
    }

    #[test]
    fn priority_channel_aging_prevents_background_starvation() {
        use super::PriorityReceiver;
        let (tx, rx) = priority_channel::<String>();
        // Enqueue one Background entry, then flood with Foreground
        // work. Without aging, the Background entry would stay behind
        // the newly-arriving high-priority work forever.
        tx.send(PriorityClass::Background, "bg".to_string())
            .unwrap();
        // Enough foreground entries to exceed the aging interval.
        let flood = PriorityReceiver::<String>::AGING_INTERVAL as usize * 2 + 1;
        for i in 0..flood {
            tx.send(PriorityClass::ForegroundAsync, format!("fg-{i}"))
                .unwrap();
        }
        let mut seen_bg_at: Option<usize> = None;
        for i in 0..flood + 1 {
            let v = rx.recv().unwrap();
            if v == "bg" {
                seen_bg_at = Some(i);
                break;
            }
        }
        let at = seen_bg_at.expect("background entry must be dispatched");
        assert!(
            (at as u64) <= PriorityReceiver::<String>::AGING_INTERVAL + 1,
            "background entry dispatched at {} pops, aging should surface it within AGING_INTERVAL+1",
            at
        );
    }

    #[test]
    fn priority_channel_preserves_fifo_within_same_priority() {
        let (tx, rx) = priority_channel::<u32>();

        tx.send(PriorityClass::ForegroundAsync, 1).unwrap();
        tx.send(PriorityClass::ForegroundAsync, 2).unwrap();
        tx.send(PriorityClass::ForegroundAsync, 3).unwrap();

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);
        assert_eq!(rx.recv().unwrap(), 3);
    }

    #[test]
    fn priority_channel_recv_blocks_until_send() {
        let (tx, rx) = priority_channel::<u32>();
        let received = Arc::new(AtomicBool::new(false));
        let received_clone = Arc::clone(&received);

        let handle = std::thread::spawn(move || {
            let val = rx.recv().unwrap();
            received_clone.store(true, Ordering::SeqCst);
            val
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!received.load(Ordering::SeqCst), "recv should block");

        tx.send(PriorityClass::Background, 99).unwrap();
        assert_eq!(handle.join().unwrap(), 99);
    }

    #[test]
    fn priority_channel_recv_returns_closed_when_sender_dropped() {
        let (tx, rx) = priority_channel::<u32>();
        drop(tx);
        assert!(matches!(rx.recv(), Err(PoolError::Closed)));
    }

    #[test]
    fn pool_executes_higher_priority_job_first() {
        use crate::pools::WorkerPool;

        let pool = WorkerPool::new(110, "prio-test");
        let execution_order = Arc::new(Mutex::new(Vec::<&str>::new()));

        // Block the worker thread
        let (block_tx, block_rx) = std::sync::mpsc::channel::<()>();
        pool.submit(PriorityClass::Background, move || {
            let _ = block_rx.recv();
        })
        .unwrap();

        // Give the blocking job time to start on the worker
        std::thread::sleep(std::time::Duration::from_millis(50));

        // While blocked, enqueue jobs at different priorities
        let o1 = Arc::clone(&execution_order);
        let h1 = pool
            .submit(PriorityClass::Background, move || {
                o1.lock().unwrap().push("bg");
            })
            .unwrap();

        let o2 = Arc::clone(&execution_order);
        let h2 = pool
            .submit(PriorityClass::ForegroundBlocking, move || {
                o2.lock().unwrap().push("fg-block");
            })
            .unwrap();

        let o3 = Arc::clone(&execution_order);
        let h3 = pool
            .submit(PriorityClass::ForegroundAsync, move || {
                o3.lock().unwrap().push("fg-async");
            })
            .unwrap();

        // Unblock the worker
        block_tx.send(()).unwrap();

        // Wait for all jobs
        h1.join().unwrap();
        h2.join().unwrap();
        h3.join().unwrap();

        let order = execution_order.lock().unwrap();
        assert_eq!(&*order, &["fg-block", "fg-async", "bg"]);
    }

    #[test]
    fn submit_async_resolves_job_on_worker_thread() {
        use crate::pools::WorkerPool;

        let pool = WorkerPool::new(120, "async-test");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let result = runtime.block_on(async {
            let rx = pool
                .submit_async(PriorityClass::ForegroundBlocking, || {
                    std::thread::current()
                        .name()
                        .unwrap_or("unnamed")
                        .to_string()
                })
                .unwrap();
            rx.await.unwrap().unwrap()
        });

        assert!(result.contains("io-async-test-host-120"));
    }

    #[test]
    fn io_pools_submit_async_routes_to_correct_pool() {
        let pools = IoPools::new(121);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let name = runtime.block_on(async {
            let rx = pools
                .submit_async(PoolKind::Image, PriorityClass::ForegroundAsync, || {
                    std::thread::current()
                        .name()
                        .unwrap_or("unnamed")
                        .to_string()
                })
                .unwrap();
            rx.await.unwrap().unwrap()
        });

        assert!(name.contains("io-image-host-121"));
    }

    static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn pool_run_preserves_panic_payload() {
        let pools = IoPools::new(3);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = pools.run(
                PoolKind::Pack,
                PriorityClass::ForegroundBlocking,
                || -> usize {
                    panic!("pool boom");
                },
            );
        }))
        .unwrap_err();

        let message = panic.downcast_ref::<&str>().copied().or_else(|| {
            panic
                .downcast_ref::<String>()
                .map(std::string::String::as_str)
        });

        assert_eq!(message, Some("pool boom"));
        assert_eq!(
            pools
                .run(PoolKind::Pack, PriorityClass::ForegroundBlocking, || 7usize)
                .unwrap(),
            7
        );
    }

    #[test]
    fn with_threads_zero_is_clamped_to_one() {
        let pool = crate::pools::WorkerPool::with_threads(200, "zero-clamp", 0);
        assert_eq!(pool.thread_count(), 1);
        // Must still accept jobs.
        let result = pool.run(PriorityClass::ForegroundAsync, || 42i32).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn multi_threaded_pool_runs_jobs_concurrently() {
        use std::time::{Duration, Instant};

        // Three workers + three blocking jobs whose total wall-clock
        // time would be ~300 ms if serialised. Parallel execution
        // puts them below ~150 ms. Generous headroom on the ceiling
        // so CI with noisy neighbours doesn't flake.
        let pool = crate::pools::WorkerPool::with_threads(300, "parallel", 3);
        assert_eq!(pool.thread_count(), 3);

        let start = Instant::now();
        let h1 = pool
            .submit(PriorityClass::ForegroundAsync, || {
                std::thread::sleep(Duration::from_millis(100));
            })
            .unwrap();
        let h2 = pool
            .submit(PriorityClass::ForegroundAsync, || {
                std::thread::sleep(Duration::from_millis(100));
            })
            .unwrap();
        let h3 = pool
            .submit(PriorityClass::ForegroundAsync, || {
                std::thread::sleep(Duration::from_millis(100));
            })
            .unwrap();
        h1.join().unwrap();
        h2.join().unwrap();
        h3.join().unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(250),
            "3 parallel 100ms jobs took {:?}, expected <250ms",
            elapsed
        );
    }

    #[test]
    fn multi_threaded_workers_consume_each_job_exactly_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let pool = crate::pools::WorkerPool::with_threads(301, "exactly-once", 4);
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..64 {
            let c = Arc::clone(&counter);
            handles.push(
                pool.submit(PriorityClass::ForegroundAsync, move || {
                    c.fetch_add(1, Ordering::SeqCst);
                })
                .unwrap(),
            );
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            64,
            "each job must run once, not more, not less"
        );
    }

    #[test]
    fn default_image_thread_count_is_within_safe_bounds() {
        let n = super::default_image_thread_count();
        assert!(n >= 2, "floor of 2 threads");
        assert!(n <= 4, "ceiling of 4 threads");
    }

    #[test]
    fn dropping_last_pool_owner_inside_worker_does_not_self_join() {
        let _hook_guard = PANIC_HOOK_LOCK.lock().unwrap();
        let panics = Arc::new(Mutex::new(Vec::<String>::new()));
        let previous_hook = std::panic::take_hook();
        let panic_messages = Arc::clone(&panics);
        std::panic::set_hook(Box::new(move |info| {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|msg| (*msg).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            panic_messages.lock().unwrap().push(message);
        }));

        let pool = crate::pools::WorkerPool::new(9, "self-drop");
        let captured_pool = pool.clone();
        let handle = pool
            .submit(PriorityClass::Background, move || {
                drop(captured_pool);
            })
            .unwrap();
        drop(pool);

        let result = handle.join();

        std::panic::set_hook(previous_hook);

        assert!(result.is_ok());
        assert!(panics.lock().unwrap().is_empty());
    }
}
