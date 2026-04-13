use std::{
    fmt,
    sync::{Arc, OnceLock, mpsc},
    thread,
};

use parking_lot::Mutex;
use shared::error::{EngineError, ErrorCode};

use crate::task::PoolKind;

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
    tx: Mutex<Option<mpsc::Sender<Job>>>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl WorkerPool {
    pub fn new(host_id: i32, label: &'static str) -> Self {
        let name = format!("io-{label}-host-{host_id}");
        let (tx, rx) = mpsc::channel::<Job>();
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

    pub fn submit<T, F>(&self, job: F) -> Result<JobHandle<T>, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let sender = self.inner.tx.lock().clone().ok_or(PoolError::Closed)?;
        sender
            .send(Box::new(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                let _ = result_tx.send(result);
            }))
            .map_err(|_| PoolError::Closed)?;

        Ok(JobHandle { rx: result_rx })
    }

    pub fn run<T, F>(&self, job: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.submit(job)?.join()
    }
}

impl Drop for WorkerPoolInner {
    fn drop(&mut self) {
        self.tx.lock().take();
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

    pub fn run<T, F>(&self, pool: PoolKind, job: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        match pool {
            PoolKind::Fs => self.fs.get().run(job),
            PoolKind::Pack => self.pack.get().run(job),
            PoolKind::Image => self.image.get().run(job),
            PoolKind::Archive => self.archive.get().run(job),
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
        sync::{Arc, Mutex},
    };

    use crate::{pools::IoPools, task::PoolKind};

    static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn pool_run_preserves_panic_payload() {
        let pools = IoPools::new(3);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = pools.run(PoolKind::Pack, || -> usize {
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
        assert_eq!(pools.run(PoolKind::Pack, || 7usize).unwrap(), 7);
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
            .submit(move || {
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
