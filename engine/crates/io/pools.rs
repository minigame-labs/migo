use std::{
    collections::BinaryHeap,
    fmt,
    sync::{Arc, Condvar, OnceLock, Mutex as StdMutex, mpsc},
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
    closed: bool,
}

pub(crate) struct PrioritySender<T> {
    inner: Arc<(StdMutex<PriorityChannelInner<T>>, Condvar)>,
}

impl<T> Clone for PrioritySender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub(crate) struct PriorityReceiver<T> {
    inner: Arc<(StdMutex<PriorityChannelInner<T>>, Condvar)>,
}

pub(crate) fn priority_channel<T>() -> (PrioritySender<T>, PriorityReceiver<T>) {
    let inner = Arc::new((
        StdMutex::new(PriorityChannelInner {
            heap: BinaryHeap::new(),
            next_seq: 0,
            closed: false,
        }),
        Condvar::new(),
    ));
    (
        PrioritySender {
            inner: Arc::clone(&inner),
        },
        PriorityReceiver { inner },
    )
}

impl<T> PrioritySender<T> {
    pub fn send(&self, priority: PriorityClass, value: T) -> Result<(), PoolError> {
        let (lock, condvar) = &*self.inner;
        let mut state = lock.lock().unwrap();
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
        condvar.notify_one();
        Ok(())
    }

    pub fn close(&self) {
        let (lock, condvar) = &*self.inner;
        let mut state = lock.lock().unwrap();
        state.closed = true;
        condvar.notify_all();
    }
}

impl<T> PriorityReceiver<T> {
    pub fn recv(&self) -> Result<T, PoolError> {
        let (lock, condvar) = &*self.inner;
        let mut state = lock.lock().unwrap();
        loop {
            if let Some(entry) = state.heap.pop() {
                return Ok(entry.value);
            }
            if state.closed {
                return Err(PoolError::Closed);
            }
            state = condvar.wait(state).unwrap();
        }
    }
}

impl<T> Drop for PrioritySender<T> {
    fn drop(&mut self) {
        // Only close if this is the last sender
        if Arc::strong_count(&self.inner) <= 2 {
            self.close();
        }
    }
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
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl WorkerPool {
    pub fn new(host_id: i32, label: &'static str) -> Self {
        let name = format!("io-{label}-host-{host_id}");
        let (tx, rx) = priority_channel::<Job>();
        let worker = thread::Builder::new()
            .name(name.clone())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job();
                }
            })
            .expect("failed to spawn IO worker pool thread");

        Self {
            inner: Arc::new(WorkerPoolInner {
                name,
                tx: Mutex::new(Some(tx)),
                worker: Mutex::new(Some(worker)),
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn submit<T, F>(
        &self,
        priority: PriorityClass,
        job: F,
    ) -> Result<JobHandle<T>, PoolError>
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
        if let Some(tx) = self.tx.lock().take() {
            tx.close();
        }
        if let Some(worker) = self.worker.lock().take() {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
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
    pool: OnceLock<WorkerPool>,
}

impl LazyWorkerPool {
    fn new(host_id: i32, label: &'static str) -> Self {
        Self {
            host_id,
            label,
            pool: OnceLock::new(),
        }
    }

    fn get(&self) -> &WorkerPool {
        self.pool
            .get_or_init(|| WorkerPool::new(self.host_id, self.label))
    }

    #[cfg(test)]
    fn is_spawned(&self) -> bool {
        self.pool.get().is_some()
    }
}

#[derive(Debug, Clone)]
pub struct IoPools {
    fs: Arc<LazyWorkerPool>,
    pack: Arc<LazyWorkerPool>,
    image: Arc<LazyWorkerPool>,
    archive: Arc<LazyWorkerPool>,
}

impl IoPools {
    pub fn new(host_id: i32) -> Self {
        Self {
            fs: Arc::new(LazyWorkerPool::new(host_id, "fs")),
            pack: Arc::new(LazyWorkerPool::new(host_id, "pack")),
            image: Arc::new(LazyWorkerPool::new(host_id, "image")),
            archive: Arc::new(LazyWorkerPool::new(host_id, "archive")),
        }
    }

    pub fn run<T, F>(
        &self,
        pool: PoolKind,
        priority: PriorityClass,
        job: F,
    ) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        match pool {
            PoolKind::Fs => self.fs.get().run(priority, job),
            PoolKind::Pack => self.pack.get().run(priority, job),
            PoolKind::Image => self.image.get().run(priority, job),
            PoolKind::Archive => self.archive.get().run(priority, job),
        }
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
        match pool {
            PoolKind::Fs => self.fs.get().submit_async(priority, job),
            PoolKind::Pack => self.pack.get().submit_async(priority, job),
            PoolKind::Image => self.image.get().submit_async(priority, job),
            PoolKind::Archive => self.archive.get().submit_async(priority, job),
        }
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

        tx.send(PriorityClass::Background, "bg".to_string()).unwrap();
        tx.send(PriorityClass::ForegroundAsync, "fg-async".to_string()).unwrap();
        tx.send(PriorityClass::ForegroundBlocking, "fg-block".to_string()).unwrap();

        assert_eq!(rx.recv().unwrap(), "fg-block");
        assert_eq!(rx.recv().unwrap(), "fg-async");
        assert_eq!(rx.recv().unwrap(), "bg");
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
            let _ = pools.run(PoolKind::Pack, PriorityClass::ForegroundBlocking, || -> usize {
                panic!("pool boom");
            });
        }))
        .unwrap_err();

        let message = panic.downcast_ref::<&str>().copied().or_else(|| {
            panic
                .downcast_ref::<String>()
                .map(std::string::String::as_str)
        });

        assert_eq!(message, Some("pool boom"));
        assert_eq!(pools.run(PoolKind::Pack, PriorityClass::ForegroundBlocking, || 7usize).unwrap(), 7);
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
