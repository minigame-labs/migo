//! Two live Sessions, two game identities, and what separates their storage.
//!
//! `storage_isolation.rs` proves the resolver gives two `GamePaths` two
//! different files. It hands the resolver a `GamePaths` it built itself, which
//! leaves the more interesting half unproven: that a real `evaluate_module`
//! turns a *game id* into that `GamePaths`, and that two Sessions alive at once
//! hold two different ones rather than sharing a slot. Section 6.4's concurrent
//! isolation is a claim about two Sessions, and every test of it so far has been
//! a claim about one function given distinct inputs.
//!
//! **The recorded obstacle was that a Session with no surface never reaches
//! `evaluate_module`.** It does. `HostJsRuntime::new` takes no surface —
//! `spawn_host_thread`/`Host::new` is what needs one, a layer above where the
//! identity binds. The layer that can *see* this property is below the layer the
//! plan named.
//!
//! **A Session per thread, because V8 makes that the only option.** Two
//! `HostJsRuntime`s on one thread abort the process with
//! `Fatal error in v8::HandleScope::CreateHandle(): Cannot create a handle
//! without a HandleScope` — the current isolate is thread-local and each runtime
//! expects to own it. That is not a limitation of the fixture: a real concurrent
//! Session *is* a host thread, so one runtime per thread is the shape under test.
//! It does mean every observation has to cross a channel, and the two loads have
//! to be *ordered* rather than raced: a shared identity slot written by two
//! concurrent threads is a coin flip, and a kill has to be certain.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::Duration;

    use shared::{
        channel::ThreadWakeup,
        device::gpu_caps::GpuCaps,
        op_state::{AudioSender, HostOpState, NetworkPolicy},
        render_command_sender::CommandSender,
        vfs::game_paths::GamePaths,
    };

    use crate::host_runtime::HostJsRuntime;

    /// Long enough that a loaded V8 isolate on a busy machine is not mistaken for
    /// a hang, short enough that a genuine hang fails the suite instead of
    /// stalling it. Every cross-thread wait in this file is bounded.
    const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

    /// One scratch tree per test, named so a leftover directory says which test
    /// left it. Unique per process and per call: two runs of the suite must not
    /// resolve into the same `games/` root and agree by accident.
    fn scratch(tag: &str) -> PathBuf {
        let unique = format!(
            "migo-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after the epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create scratch root");
        root
    }

    /// The same `GamePaths` the runtime will derive, used both to place the entry
    /// module and to name the namespace each game must land in. Derived rather
    /// than spelled out: the on-disk layout is `game_paths.rs`'s property and has
    /// its own tests, and repeating it here would make this test fail on a layout
    /// change while saying nothing about identity binding.
    fn paths(files: &Path, cache: &Path, game_id: &str) -> GamePaths {
        GamePaths::new(files, cache, game_id).expect("game paths")
    }

    /// A trivial entry module where `evaluate_module` will look for it. It must
    /// exist and must evaluate: a load or evaluation failure returns `Err` before
    /// the identity is observable.
    fn install_entry(files: &Path, cache: &Path, game_id: &str) {
        let code = paths(files, cache, game_id).code_dir().to_path_buf();
        std::fs::create_dir_all(&code).expect("create code dir");
        std::fs::write(
            code.join("game.js"),
            b"// nothing to draw; the identity binds before any of this runs\n",
        )
        .expect("write entry module");
    }

    fn host_op_state(files: &Path, cache: &Path, id: i32) -> HostOpState {
        let (render_tx, _render_rx) = CommandSender::new();
        let (host_tx, _critical_host_tx, _host_rx) = shared::host_channel::channel(1);

        HostOpState {
            id,
            app_cache_dir: cache.to_path_buf(),
            app_files_dir: files.to_path_buf(),
            code_dir: None,
            game_paths: None,
            vfs: None,
            mount_table: None,
            render_tx,
            text_measurer: None,
            audio_tx: AudioSender::new(shared::audio_channel::disconnected(), ThreadWakeup::new()),
            host_tx,
            device_services: None,
            raf_rx: None,
            raf_demand: Arc::new(shared::raf_signal::RafDemand::new()),
            request_vsync: None,
            sub_packages: Vec::new(),
            workers_path: None,
            network_policy: NetworkPolicy::default(),
            backgrounded: Arc::new(AtomicBool::new(false)),
            timer_backgrounded: Arc::new(AtomicBool::new(false)),
            webgl_context_created: Arc::new(AtomicBool::new(false)),
            context_lost: Arc::new(shared::op_state::ContextLostState::default()),
            code_signing_enabled: false,
            gpu_caps: GpuCaps::new(),
        }
    }

    fn storage_of(rt: &HostJsRuntime) -> PathBuf {
        let op_state = rt.op_state();
        let borrowed = op_state.borrow();
        crate::storage::storage_dir(&borrowed).expect("a loaded game must resolve storage")
    }

    enum Step {
        /// Bind this game identity by evaluating its entry module.
        Load(String),
        /// Ask the production resolver, on this thread, over this isolate's own
        /// op state.
        ReportStorage,
        /// Write one key through the production storage path, under the shipped
        /// quota, and report whether it was admitted.
        Write { key: String, bytes: usize },
        /// This game's own byte total, as its own store accounts for it.
        ReportBytes,
    }

    /// What a step answered. One channel carries all three because the steps are
    /// ordered: a Session answers exactly one of these per step it is handed.
    enum Answer {
        Storage(PathBuf),
        /// `Ok(())` when the write was admitted, `Err(message)` when the quota
        /// refused it. The message is carried rather than the error so a refusal
        /// for the wrong reason is visible in the failure.
        Written(Result<(), String>),
        Bytes(u64),
    }

    /// A host thread with one live `HostJsRuntime`, driven step by step so the
    /// two Sessions' loads can be ordered instead of raced.
    struct Session {
        steps: Option<mpsc::Sender<Step>>,
        answers: mpsc::Receiver<Answer>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Session {
        fn spawn(files: &Path, cache: &Path, id: i32) -> Self {
            let (step_tx, step_rx) = mpsc::channel::<Step>();
            let (answer_tx, answer_rx) = mpsc::channel::<Answer>();
            let (files, cache) = (files.to_path_buf(), cache.to_path_buf());

            let thread = std::thread::Builder::new()
                .name(format!("session-{id}"))
                .spawn(move || {
                    let tokio_rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("current-thread tokio runtime");
                    tokio_rt.block_on(async move {
                        let mut js = HostJsRuntime::new(
                            id,
                            host_op_state(&files, &cache, id),
                            &cache,
                            #[cfg(feature = "v8-limits")]
                            crate::host_runtime::V8LimitsConfig::from_max_memory_mb(256),
                            #[cfg(feature = "code-signing")]
                            false,
                            #[cfg(feature = "code-signing")]
                            None,
                        );
                        // Ends when the last `steps` sender drops, which is the
                        // shutdown signal.
                        while let Ok(step) = step_rx.recv() {
                            let answer = match step {
                                Step::Load(game_id) => {
                                    js.evaluate_module(game_id.clone(), "game.js".to_string())
                                        .await
                                        .unwrap_or_else(|e| panic!("{game_id} must evaluate: {e}"));
                                    Answer::Storage(PathBuf::new())
                                }
                                Step::ReportStorage => Answer::Storage(storage_of(&js)),
                                Step::Write { key, bytes } => {
                                    let dir = storage_of(&js);
                                    let value = "x".repeat(bytes);
                                    Answer::Written(
                                        migo_io::storage_ops::storage_set(
                                            &dir,
                                            &key,
                                            &value,
                                            crate::storage::MAX_TOTAL_BYTES,
                                        )
                                        .map_err(|e| e.to_string()),
                                    )
                                }
                                Step::ReportBytes => {
                                    let dir = storage_of(&js);
                                    Answer::Bytes(
                                        migo_io::storage_ops::storage_info(
                                            &dir,
                                            crate::storage::MAX_TOTAL_BYTES,
                                        )
                                        .expect("a loaded game's store reports its own size")
                                        .current_bytes,
                                    )
                                }
                            };
                            if answer_tx.send(answer).is_err() {
                                break;
                            }
                        }
                    });
                })
                .expect("spawn session thread");

            Self {
                steps: Some(step_tx),
                answers: answer_rx,
                thread: Some(thread),
            }
        }

        fn step(&self, step: Step) -> Answer {
            self.steps
                .as_ref()
                .expect("session still running")
                .send(step)
                .expect("session thread accepts steps");
            self.answers
                .recv_timeout(ANSWER_TIMEOUT)
                .expect("session thread answered within the timeout")
        }

        fn load(&self, game_id: &str) {
            let _ = self.step(Step::Load(game_id.to_string()));
        }

        fn storage(&self) -> PathBuf {
            match self.step(Step::ReportStorage) {
                Answer::Storage(path) => path,
                _ => panic!("a storage step must answer with a path"),
            }
        }

        fn write(&self, key: &str, bytes: usize) -> Result<(), String> {
            match self.step(Step::Write {
                key: key.to_string(),
                bytes,
            }) {
                Answer::Written(outcome) => outcome,
                _ => panic!("a write step must answer with its outcome"),
            }
        }

        fn bytes(&self) -> u64 {
            match self.step(Step::ReportBytes) {
                Answer::Bytes(total) => total,
                _ => panic!("a size step must answer with a total"),
            }
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            // Dropping the sender is what ends the loop; the join is only
            // reached after every step was already acknowledged.
            self.steps = None;
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// The property Section 6.4 is actually about: two Sessions running at once,
    /// each writing to its own storage.
    ///
    /// **Both Sessions are given the *same* app directories on purpose.** Handing
    /// each its own `app_files_dir` would make the two paths differ for a reason
    /// that has nothing to do with per-game namespacing, and the test would pass
    /// over an engine that ignored the game id entirely. The game id is the only
    /// input that differs here, so it is the only thing that can separate them.
    ///
    /// **Both loads happen before either read, and that ordering is
    /// load-bearing.** Every way an identity can be shared — a process-wide slot
    /// written at bind time, a resolver that memoises its first answer, one op
    /// state behind two runtimes — shows up as two equal paths only once both
    /// Sessions have bound. Reading `a` before `b` loads would let all of them
    /// through.
    #[test]
    fn two_live_sessions_resolve_storage_under_their_own_game_id() {
        let root = scratch("two-sessions");
        let files = root.join("files");
        let cache = root.join("cache");
        install_entry(&files, &cache, "game-a");
        install_entry(&files, &cache, "game-b");

        let a = Session::spawn(&files, &cache, 1);
        let b = Session::spawn(&files, &cache, 2);
        a.load("game-a");
        b.load("game-b");

        let (sa, sb) = (a.storage(), b.storage());
        let na = paths(&files, &cache, "game-a")
            .user_data_dir()
            .to_path_buf();
        let nb = paths(&files, &cache, "game-b")
            .user_data_dir()
            .to_path_buf();

        assert_ne!(sa, sb, "both live Sessions resolved to {sa:?}");
        // Not merely different: each under the identity it was loaded with, and
        // under no other. Two paths can differ while both are wrong — a counter,
        // a host id, a temp name — and the negative half is what rules out "both
        // took game-b" and "both took a shared root".
        assert!(
            sa.starts_with(&na) && !sa.starts_with(&nb),
            "game-a resolved to {sa:?}, which is not its own namespace {na:?}"
        );
        assert!(
            sb.starts_with(&nb) && !sb.starts_with(&na),
            "game-b resolved to {sb:?}, which is not its own namespace {nb:?}"
        );

        drop(a);
        drop(b);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// One game exhausting its storage quota leaves the other's untouched.
    ///
    /// Section 6.4 lists "per-game filesystem, key-value, and quota isolation
    /// derived from the game identity" as an enforced property, and task 0.62 closed
    /// the *namespace* half: two live Sessions resolve to two directories. Distinct
    /// directories are necessary and not sufficient. Nothing showed that the 10 MB
    /// limit is *each game's* 10 MB, which is a claim about where the accounting
    /// lives, not about where the files do — and the two are separable. The store
    /// handles are kept in a process-wide `HashMap` in `storage_ops`, and a shared
    /// running total, a cache key that lost the directory, or a quota checked against
    /// a global would all leave two distinct directories in place while making one
    /// game's writes count against the other's budget.
    ///
    /// **The exhaustion is the setup and the neighbour's write is the property.**
    /// `a` is filled through the production `storage_set` under the shipped
    /// `MAX_TOTAL_BYTES` until it is refused, and that refusal is asserted: a fixture
    /// that never reached the limit would prove nothing about sharing it. Then `b`
    /// writes, and must be admitted.
    ///
    /// **The byte totals are asserted as well as the outcome**, because "b's write
    /// succeeded" is satisfied by a shared store that simply had room left. `b`'s own
    /// store must account for `b`'s bytes and nothing near `a`'s.
    #[test]
    fn one_game_exhausting_its_quota_leaves_the_other_game_its_own() {
        let root = scratch("two-sessions-quota");
        let files = root.join("files");
        let cache = root.join("cache");
        install_entry(&files, &cache, "game-a");
        install_entry(&files, &cache, "game-b");

        let a = Session::spawn(&files, &cache, 1);
        let b = Session::spawn(&files, &cache, 2);
        a.load("game-a");
        b.load("game-b");

        // One below `MAX_VALUE_SIZE`, so the per-value cap never decides anything
        // here and the only limit in play is the total.
        const CHUNK: usize = 1024 * 1024 - 1;
        // The shipped total is 10 MB, so eleven chunks cannot fit however the store
        // rounds; the loop stops at the first refusal rather than at this bound.
        const ENOUGH_TO_OVERFILL: usize = 16;

        let mut refusal = None;
        for chunk in 0..ENOUGH_TO_OVERFILL {
            if let Err(message) = a.write(&format!("fill-{chunk}"), CHUNK) {
                refusal = Some((chunk, message));
                break;
            }
        }
        let (chunks_admitted, message) = refusal
            .expect("game-a never reached its quota, so nothing here says whose quota it was");
        assert!(
            message.contains("storage limit exceeded"),
            "game-a's write was refused for something other than its quota: {message}"
        );
        assert!(
            chunks_admitted > 1,
            "game-a was refused its {chunks_admitted}th chunk, which is too early for a \
             10 MB limit -- the refusal is not the quota it is named for"
        );

        assert_eq!(
            b.write("after-the-neighbour-filled-up", CHUNK),
            Ok(()),
            "game-b was refused because game-a had filled up"
        );

        let (bytes_a, bytes_b) = (a.bytes(), b.bytes());
        assert!(
            bytes_b < bytes_a,
            "game-b's store accounts for {bytes_b} bytes against game-a's {bytes_a}, so \
             the two are counting the same writes"
        );
        assert!(
            bytes_b >= CHUNK as u64 && bytes_b < 2 * CHUNK as u64,
            "game-b's store accounts for {bytes_b} bytes after writing one {CHUNK}-byte \
             value, so it is not accounting for its own writes alone"
        );

        drop(a);
        drop(b);
        let _ = std::fs::remove_dir_all(&root);
    }
}
