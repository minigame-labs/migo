use crate::{
    cost::CheapPolicy,
    domain::IoDomain,
    pools::{IoPools, PoolError},
    task::{BackendKind, IoRequest, PoolKind, PriorityClass, ReadSpec, RequestKind},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[cfg(test)]
type WorkerStartHook = std::sync::Arc<dyn Fn() + Send + Sync + 'static>;

#[cfg(test)]
fn run_worker_start_test_hook(slot: &std::sync::Mutex<Option<WorkerStartHook>>) {
    let hook = slot.lock().unwrap().clone();
    if let Some(hook) = hook {
        hook();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteDecision {
    Inline,
    Delegated(PoolKind),
}

#[derive(Clone)]
pub struct IoScheduler {
    host_id: i32,
    pools: IoPools,
    domain: Arc<IoDomain>,
    policy: CheapPolicy,
    metrics: Arc<SchedulerMetrics>,
    #[cfg(test)]
    worker_start_hook: Arc<std::sync::Mutex<Option<WorkerStartHook>>>,
    #[cfg(test)]
    image_job_runs: Arc<std::sync::atomic::AtomicUsize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchedulerMetricsSnapshot {
    pub inline_runs: u64,
    pub delegated_runs: u64,
    pub rejected_runs: u64,
    pub sync_wait_micros: u64,
}

#[derive(Default)]
struct SchedulerMetrics {
    inline_runs: AtomicU64,
    delegated_runs: AtomicU64,
    rejected_runs: AtomicU64,
    sync_wait_micros: AtomicU64,
}

impl IoScheduler {
    pub fn new(host_id: i32) -> Self {
        Self {
            host_id,
            pools: IoPools::new(host_id),
            domain: Arc::new(IoDomain::new()),
            policy: CheapPolicy::default(),
            metrics: Arc::new(SchedulerMetrics::default()),
            #[cfg(test)]
            worker_start_hook: Arc::new(std::sync::Mutex::new(None)),
            #[cfg(test)]
            image_job_runs: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    pub fn host_id(&self) -> i32 {
        self.host_id
    }

    pub fn policy(&self) -> CheapPolicy {
        self.policy
    }

    pub fn pools(&self) -> &IoPools {
        &self.pools
    }

    pub fn domain(&self) -> Arc<IoDomain> {
        Arc::clone(&self.domain)
    }

    pub fn metrics(&self) -> SchedulerMetricsSnapshot {
        SchedulerMetricsSnapshot {
            inline_runs: self.metrics.inline_runs.load(Ordering::Relaxed),
            delegated_runs: self.metrics.delegated_runs.load(Ordering::Relaxed),
            rejected_runs: self.metrics.rejected_runs.load(Ordering::Relaxed),
            sync_wait_micros: self.metrics.sync_wait_micros.load(Ordering::Relaxed),
        }
    }

    pub fn close(&self) {
        self.domain.close_all();
    }

    pub fn ensure_open(&self) -> Result<(), PoolError> {
        if self.domain.is_closed() {
            Err(PoolError::Closed)
        } else {
            Ok(())
        }
    }

    pub fn classify(&self, req: &IoRequest) -> RouteDecision {
        classify_request(req, &self.policy)
    }

    pub fn run_sync<T, F>(&self, req: &IoRequest, job: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        if self.ensure_open().is_err() {
            self.metrics.rejected_runs.fetch_add(1, Ordering::Relaxed);
            return Err(PoolError::Closed);
        }

        match self.classify(req) {
            RouteDecision::Inline => {
                self.metrics.inline_runs.fetch_add(1, Ordering::Relaxed);
                Ok(job())
            }
            RouteDecision::Delegated(pool) => {
                self.metrics.delegated_runs.fetch_add(1, Ordering::Relaxed);
                let started_at = Instant::now();
                let priority = req.priority();
                let domain = Arc::clone(&self.domain);
                #[cfg(test)]
                let worker_start_hook = Arc::clone(&self.worker_start_hook);
                let result = self.pools.run(pool, priority, move || {
                    #[cfg(test)]
                    run_worker_start_test_hook(&worker_start_hook);

                    if domain.is_closed() {
                        return Err(PoolError::Closed);
                    }
                    Ok(job())
                });
                let wait_micros = started_at.elapsed().as_micros() as u64;
                self.metrics
                    .sync_wait_micros
                    .fetch_add(wait_micros, Ordering::Relaxed);
                tracing::debug!(
                    "[IOScheduler {}] sync wait pool={:?} elapsed_us={}",
                    self.host_id,
                    pool,
                    wait_micros
                );
                // Observability: bump the process-global slow-IO
                // counter when wall-clock crosses 100 ms. Uses the
                // IO scheduler's measurement because it includes
                // both pool queue wait + the job itself, which is
                // the latency the JS caller actually sees.
                shared::stats::io_metrics_global().record_if_slow(wait_micros / 1000);
                match result {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(err)) => {
                        self.metrics.rejected_runs.fetch_add(1, Ordering::Relaxed);
                        Err(err)
                    }
                    Err(err) => Err(err),
                }
            }
        }
    }

    pub async fn run_async<T, F>(&self, req: IoRequest, job: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        if self.ensure_open().is_err() {
            self.metrics.rejected_runs.fetch_add(1, Ordering::Relaxed);
            return Err(PoolError::Closed);
        }

        match self.classify(&req) {
            RouteDecision::Inline => {
                self.metrics.inline_runs.fetch_add(1, Ordering::Relaxed);
                Ok(job())
            }
            RouteDecision::Delegated(pool) => {
                self.metrics.delegated_runs.fetch_add(1, Ordering::Relaxed);
                let priority = req.priority();
                let domain = Arc::clone(&self.domain);
                #[cfg(test)]
                let worker_start_hook = Arc::clone(&self.worker_start_hook);
                let rx = self.pools.submit_async(pool, priority, move || {
                    #[cfg(test)]
                    run_worker_start_test_hook(&worker_start_hook);

                    if domain.is_closed() {
                        return Err(PoolError::Closed);
                    }
                    Ok(job())
                })?;
                match rx.await {
                    Ok(Ok(Ok(value))) => Ok(value),
                    Ok(Ok(Err(err))) => {
                        self.metrics.rejected_runs.fetch_add(1, Ordering::Relaxed);
                        Err(err)
                    }
                    Ok(Err(payload)) => std::panic::resume_unwind(payload),
                    Err(_) => Err(PoolError::Closed),
                }
            }
        }
    }
}

#[cfg(test)]
impl IoScheduler {
    // Per-instance, NOT a process-global slot — see IoDomain's hook for the
    // same deadlock rationale: parallel tests each own their own scheduler, so
    // another test's worker starting no longer trips this test's barrier hook.
    pub(crate) fn install_worker_start_test_hook(&self, hook: WorkerStartHook) {
        *self.worker_start_hook.lock().unwrap() = Some(hook);
    }

    // Per-instance counter of image jobs routed through this scheduler. A
    // process-global counter made the image tests' reset→assert(==1) windows
    // race under parallel execution; each test owns its own scheduler.
    pub(crate) fn note_image_job_run(&self) {
        self.image_job_runs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn image_job_run_count(&self) -> usize {
        self.image_job_runs
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for IoScheduler {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Drop for IoScheduler {
    fn drop(&mut self) {
        self.close();
    }
}

pub fn classify_request(req: &IoRequest, policy: &CheapPolicy) -> RouteDecision {
    match req {
        IoRequest::ReadFile {
            backend,
            request: _,
            priority,
            spec,
            estimated_bytes,
        } => classify_read(*backend, *priority, *spec, *estimated_bytes, policy),
        IoRequest::GetFileInfo {
            backend, priority, ..
        } => classify_metadata(*backend, *priority),
        IoRequest::DecodeImage { cache_hit, .. } => {
            if *cache_hit {
                RouteDecision::Inline
            } else {
                RouteDecision::Delegated(PoolKind::Image)
            }
        }
        IoRequest::Unzip { .. } => RouteDecision::Delegated(PoolKind::Archive),
        IoRequest::PackageIngest { .. } => RouteDecision::Delegated(PoolKind::Archive),
        IoRequest::StorageGet {
            request,
            priority,
            estimated_bytes,
        } => classify_storage_get(*request, *priority, *estimated_bytes, policy),
        IoRequest::StorageMutate { .. } | IoRequest::StorageInfo { .. } => {
            RouteDecision::Delegated(PoolKind::Fs)
        }
        // Generic fs ops (write/copy/mkdir/stat/...): sync/foreground-blocking
        // runs inline on the caller (V8) thread — matching the pre-scheduler
        // behaviour where sync fs ops ran directly on the V8 thread; async
        // work fans out to the fs pool, replacing the raw `spawn_blocking`
        // so it becomes bounded, prioritized and domain-close-aware.
        IoRequest::FsOp { priority, .. } => {
            if *priority == PriorityClass::ForegroundBlocking {
                RouteDecision::Inline
            } else {
                RouteDecision::Delegated(PoolKind::Fs)
            }
        }
    }
}

fn classify_read(
    backend: BackendKind,
    priority: PriorityClass,
    spec: ReadSpec,
    estimated_bytes: usize,
    policy: &CheapPolicy,
) -> RouteDecision {
    match backend {
        // Small foreground pack reads (icons, JSON, atlas descriptors)
        // run inline: the bytes are usually already in the OS page
        // cache and a hot inflate cache hit returns instantly, so a
        // pool hop just adds a thread handoff to a sub-millisecond
        // operation. Everything else still fans out to the pack pool.
        BackendKind::Pack => {
            if is_foreground(priority) && estimated_bytes <= inline_read_bytes(spec, policy) {
                RouteDecision::Inline
            } else {
                RouteDecision::Delegated(PoolKind::Pack)
            }
        }
        BackendKind::Archive => RouteDecision::Delegated(PoolKind::Archive),
        BackendKind::Filesystem => {
            if is_foreground(priority) && estimated_bytes <= inline_read_bytes(spec, policy) {
                RouteDecision::Inline
            } else {
                RouteDecision::Delegated(PoolKind::Fs)
            }
        }
    }
}

fn classify_metadata(backend: BackendKind, priority: PriorityClass) -> RouteDecision {
    match backend {
        // Pack metadata is an in-memory hashmap lookup; a pool hop is
        // pure overhead. Inline foreground requests.
        BackendKind::Pack if is_foreground(priority) => RouteDecision::Inline,
        BackendKind::Pack => RouteDecision::Delegated(PoolKind::Pack),
        BackendKind::Archive => RouteDecision::Delegated(PoolKind::Archive),
        BackendKind::Filesystem if is_foreground(priority) => RouteDecision::Inline,
        BackendKind::Filesystem => RouteDecision::Delegated(PoolKind::Fs),
    }
}

fn classify_storage_get(
    request: RequestKind,
    priority: PriorityClass,
    estimated_bytes: usize,
    policy: &CheapPolicy,
) -> RouteDecision {
    if request == RequestKind::Sync
        && priority == PriorityClass::ForegroundBlocking
        && estimated_bytes <= policy.small_copy_bytes
    {
        RouteDecision::Inline
    } else {
        RouteDecision::Delegated(PoolKind::Fs)
    }
}

fn inline_read_bytes(spec: ReadSpec, policy: &CheapPolicy) -> usize {
    match spec {
        ReadSpec::Whole => policy.small_read_bytes,
        ReadSpec::Range { length, .. } => policy.small_read_bytes.min(length),
    }
}

fn is_foreground(priority: PriorityClass) -> bool {
    matches!(
        priority,
        PriorityClass::ForegroundBlocking | PriorityClass::ForegroundAsync
    )
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Arc, Barrier,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use crate::{
        cost::CheapPolicy,
        scheduler::{IoScheduler, RouteDecision, classify_request},
        task::{BackendKind, IoRequest, PoolKind, PriorityClass, ReadSpec, RequestKind},
    };

    #[test]
    fn delegated_sync_pack_reads_use_pack_pool() {
        let scheduler = IoScheduler::new(7);
        // Foreground reads under the inline threshold short-circuit, so
        // size the request comfortably above it to verify delegation.
        let req = IoRequest::ReadFile {
            backend: BackendKind::Pack,
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
            spec: ReadSpec::Whole,
            estimated_bytes: 256 * 1024,
        };

        let thread_name = scheduler
            .run_sync(&req, || {
                std::thread::current()
                    .name()
                    .unwrap_or("unnamed")
                    .to_string()
            })
            .unwrap();

        assert!(thread_name.contains("io-pack-host-7"));
    }

    #[test]
    fn scheduler_does_not_spawn_pools_until_delegated_work_runs() {
        let scheduler = IoScheduler::new(11);

        assert_eq!(scheduler.pools().spawned_pool_count(), 0);

        let req = IoRequest::ReadFile {
            backend: BackendKind::Pack,
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
            spec: ReadSpec::Whole,
            estimated_bytes: 256 * 1024,
        };

        let _ = scheduler.run_sync(&req, || 1usize).unwrap();

        assert_eq!(scheduler.pools().spawned_pool_count(), 1);
    }

    #[test]
    fn scheduler_records_inline_and_delegated_counts() {
        let scheduler = IoScheduler::new(17);
        let inline_req = IoRequest::ReadFile {
            backend: BackendKind::Filesystem,
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
            spec: ReadSpec::Range {
                position: 0,
                length: 32,
            },
            estimated_bytes: 32,
        };
        let delegated_req = IoRequest::ReadFile {
            backend: BackendKind::Pack,
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
            spec: ReadSpec::Whole,
            estimated_bytes: 256 * 1024,
        };

        scheduler.run_sync(&inline_req, || 1usize).unwrap();
        scheduler.run_sync(&delegated_req, || 2usize).unwrap();

        let metrics = scheduler.metrics();
        assert_eq!(metrics.inline_runs, 1);
        assert_eq!(metrics.delegated_runs, 1);
    }

    #[test]
    fn queued_delegated_work_rechecks_domain_closure_before_running() {
        // Archive pool is intentionally single-threaded so the queueing
        // semantics this test exercises (second job blocked behind the
        // first, then sees Closed when the scheduler is shut down) are
        // observable without timing-dependent assertions.
        let scheduler = Arc::new(IoScheduler::new(19));
        let req = IoRequest::Unzip {
            backend: BackendKind::Filesystem,
            priority: PriorityClass::ForegroundAsync,
            compressed_bytes: 64 * 1024,
        };

        let hook_calls = Arc::new(AtomicUsize::new(0));
        let hook_entered = Arc::new(Barrier::new(2));
        let hook_release = Arc::new(Barrier::new(2));
        let hook_calls_clone = Arc::clone(&hook_calls);
        let hook_entered_clone = Arc::clone(&hook_entered);
        let hook_release_clone = Arc::clone(&hook_release);
        scheduler.install_worker_start_test_hook(Arc::new(move || {
            let call = hook_calls_clone.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 2 {
                hook_entered_clone.wait();
                hook_release_clone.wait();
            }
        }));

        let first_scheduler = Arc::clone(&scheduler);
        let first_req = req.clone();
        let first_started = Arc::new(Barrier::new(2));
        let first_release = Arc::new(Barrier::new(2));
        let first_started_thread = Arc::clone(&first_started);
        let first_release_thread = Arc::clone(&first_release);
        let first = std::thread::spawn(move || {
            first_scheduler
                .run_sync(&first_req, move || {
                    first_started_thread.wait();
                    first_release_thread.wait();
                    1usize
                })
                .unwrap()
        });

        first_started.wait();

        let second_scheduler = Arc::clone(&scheduler);
        let second_req = req.clone();
        let second_ran = Arc::new(AtomicBool::new(false));
        let second_ran_thread = Arc::clone(&second_ran);
        let second = std::thread::spawn(move || {
            second_scheduler.run_sync(&second_req, move || {
                second_ran_thread.store(true, Ordering::SeqCst);
                2usize
            })
        });

        first_release.wait();
        hook_entered.wait();
        scheduler.close();
        hook_release.wait();

        assert_eq!(first.join().unwrap(), 1);
        let second_result = second.join().unwrap();

        assert!(matches!(
            second_result,
            Err(crate::pools::PoolError::Closed)
        ));
        assert!(!second_ran.load(Ordering::SeqCst));
    }

    #[test]
    fn scheduler_routes_pack_backed_reads_to_pack_pool() {
        let req = IoRequest::ReadFile {
            backend: BackendKind::Pack,
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
            spec: ReadSpec::Whole,
            estimated_bytes: 128 * 1024,
        };

        assert_eq!(
            classify_request(&req, &CheapPolicy::default()),
            RouteDecision::Delegated(PoolKind::Pack)
        );
    }

    #[test]
    fn scheduler_keeps_small_fs_sync_reads_inline() {
        let req = IoRequest::ReadFile {
            backend: BackendKind::Filesystem,
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
            spec: ReadSpec::Range {
                position: 0,
                length: 1024,
            },
            estimated_bytes: 1024,
        };

        let policy = CheapPolicy {
            small_read_bytes: 4 * 1024,
            small_copy_bytes: 512,
        };

        assert_eq!(classify_request(&req, &policy), RouteDecision::Inline);
    }

    #[test]
    fn scheduler_keeps_small_fs_async_reads_inline() {
        let req = IoRequest::ReadFile {
            backend: BackendKind::Filesystem,
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
            spec: ReadSpec::Range {
                position: 0,
                length: 1024,
            },
            estimated_bytes: 1024,
        };

        let policy = CheapPolicy {
            small_read_bytes: 4 * 1024,
            small_copy_bytes: 512,
        };

        assert_eq!(classify_request(&req, &policy), RouteDecision::Inline);
    }

    #[test]
    fn scheduler_keeps_fs_async_metadata_inline() {
        let req = IoRequest::GetFileInfo {
            backend: BackendKind::Filesystem,
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
        };

        assert_eq!(
            classify_request(&req, &CheapPolicy::default()),
            RouteDecision::Inline
        );
    }

    #[test]
    fn scheduler_keeps_small_sync_storage_get_inline() {
        let req = IoRequest::StorageGet {
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
            estimated_bytes: 128,
        };

        let policy = CheapPolicy {
            small_read_bytes: 4 * 1024,
            small_copy_bytes: 512,
        };

        assert_eq!(classify_request(&req, &policy), RouteDecision::Inline);
    }

    #[test]
    fn scheduler_keeps_sync_fs_ops_inline() {
        // Sync (ForegroundBlocking) generic fs ops run inline on the caller
        // (V8) thread, matching pre-scheduler behaviour.
        let req = IoRequest::FsOp {
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
        };
        assert_eq!(
            classify_request(&req, &CheapPolicy::default()),
            RouteDecision::Inline
        );
    }

    #[test]
    fn scheduler_routes_async_fs_ops_to_fs_pool() {
        // Async generic fs ops (writes/mutations/metadata) fan out to the fs
        // pool instead of tokio's unbounded blocking pool.
        let req = IoRequest::FsOp {
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
        };
        assert_eq!(
            classify_request(&req, &CheapPolicy::default()),
            RouteDecision::Delegated(PoolKind::Fs)
        );
    }

    #[test]
    fn scheduler_routes_unzip_requests_to_archive_pool() {
        let req = IoRequest::Unzip {
            backend: BackendKind::Filesystem,
            priority: PriorityClass::ForegroundAsync,
            compressed_bytes: 32 * 1024,
        };

        assert_eq!(
            classify_request(&req, &CheapPolicy::default()),
            RouteDecision::Delegated(PoolKind::Archive)
        );
    }

    #[test]
    fn scheduler_routes_uncached_image_decode_requests_to_image_pool() {
        let req = IoRequest::DecodeImage {
            backend: BackendKind::Filesystem,
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
            encoded_bytes: 128 * 1024,
            cache_hit: false,
        };

        assert_eq!(
            classify_request(&req, &CheapPolicy::default()),
            RouteDecision::Delegated(PoolKind::Image)
        );
    }

    #[test]
    fn scheduler_keeps_cached_image_decode_requests_inline() {
        let req = IoRequest::DecodeImage {
            backend: BackendKind::Filesystem,
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
            encoded_bytes: 128 * 1024,
            cache_hit: true,
        };

        assert_eq!(
            classify_request(&req, &CheapPolicy::default()),
            RouteDecision::Inline
        );
    }

    #[test]
    fn delegated_async_work_preserves_panic_payload() {
        let scheduler = IoScheduler::new(13);
        let req = IoRequest::ReadFile {
            backend: BackendKind::Pack,
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
            spec: ReadSpec::Whole,
            estimated_bytes: 256 * 1024,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let result = catch_unwind(AssertUnwindSafe(|| {
            runtime.block_on(async {
                let _: Result<(), _> = scheduler
                    .run_async(req, || -> () { panic!("pack worker panic") })
                    .await;
            });
        }));

        let payload = result.expect_err("expected delegated panic to propagate");
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic>");
        assert_eq!(message, "pack worker panic");
    }

    #[test]
    fn run_async_completes_without_tokio_blocking_threads() {
        let scheduler = IoScheduler::new(201);
        let req = IoRequest::ReadFile {
            backend: BackendKind::Pack,
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
            spec: ReadSpec::Whole,
            estimated_bytes: 256 * 1024,
        };

        // Build a multi-thread runtime with exactly 1 blocking thread.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();

        // Saturate the one blocking thread with a long-running task.
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        runtime.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _ = release_rx.recv(); // hold the blocking thread forever
            });
        });
        // Let the blocking task start.
        std::thread::sleep(std::time::Duration::from_millis(100));

        // If run_async still uses spawn_blocking, this will time out because
        // the blocking pool is exhausted. With oneshot-based submit_async it
        // completes immediately via the dedicated worker pool thread.
        let result = runtime.block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                scheduler.run_async(req, || 42usize),
            )
            .await
        });

        // Release the blocked thread.
        let _ = release_tx.send(());

        assert!(
            result.is_ok(),
            "run_async timed out — still using spawn_blocking?"
        );
        assert_eq!(result.unwrap().unwrap(), 42);
    }
}
