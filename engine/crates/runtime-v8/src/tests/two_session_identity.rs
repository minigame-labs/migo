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
    }

    /// A host thread with one live `HostJsRuntime`, driven step by step so the
    /// two Sessions' loads can be ordered instead of raced.
    struct Session {
        steps: Option<mpsc::Sender<Step>>,
        answers: mpsc::Receiver<PathBuf>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Session {
        fn spawn(files: &Path, cache: &Path, id: i32) -> Self {
            let (step_tx, step_rx) = mpsc::channel::<Step>();
            let (answer_tx, answer_rx) = mpsc::channel::<PathBuf>();
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
                                    PathBuf::new()
                                }
                                Step::ReportStorage => storage_of(&js),
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

        fn step(&self, step: Step) -> PathBuf {
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
            self.step(Step::ReportStorage)
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
}
