//! The first behavioural tests that run two Sessions at once.
//!
//! Every concurrent-session property was previously true by reading: no test
//! anywhere created two Sessions, so nothing would have failed if a shared
//! process-global had crept back in. Section 6.4 of the four-platform design makes
//! multi-game support a product guarantee and says it is "gated rather than
//! assumed", which is what this module starts.
//!
//! What these can and cannot reach: `migo_session_create` produces a Session
//! without an attached surface, so no Host and therefore no V8 isolate exists yet.
//! Isolate separation and per-game storage rooting are therefore *not* covered
//! here — claiming otherwise would be the same inspection-as-test mistake Section
//! 7.3 warns about. What is reachable is the Session lifecycle itself: coexistence,
//! the Engine's refusal to be destroyed under live Sessions, independent teardown,
//! and driving two Sessions from two host threads.

use std::{
    ptr,
    sync::{Arc, Barrier},
};

use migo_capi_abi::{MIGO_ERROR_INVALID_STATE, MIGO_OK};

use crate::{
    MigoEngine, MigoSession, migo_engine_create, migo_engine_destroy, migo_session_create,
    migo_session_destroy,
    test_support::{engine_config, scratch_dirs, session_config},
};

/// A raw engine pointer that is safe to move into another thread.
///
/// The C API is explicitly callable from several host threads, and that is the
/// property under test, so the wrapper exists to let the test express it rather
/// than to paper over anything.
#[derive(Clone, Copy)]
struct EnginePtr(*mut MigoEngine);
unsafe impl Send for EnginePtr {}

#[derive(Clone, Copy)]
struct SessionPtr(*mut MigoSession);
unsafe impl Send for SessionPtr {}

/// Accessors rather than direct field reads inside the thread closures. Closure
/// capture is per-field since edition 2021, so writing `engine.0` in the closure
/// captures the raw pointer and the `Send` wrapper never applies; a method call
/// captures the wrapper.
impl EnginePtr {
    fn get(self) -> *mut MigoEngine {
        self.0
    }
}

impl SessionPtr {
    fn get(self) -> *mut MigoSession {
        self.0
    }
}

fn create_engine(tag: &str) -> *mut MigoEngine {
    let dirs = scratch_dirs(tag);
    let config = engine_config(
        &dirs,
        size_of::<crate::MigoEngineConfig>() as u32,
        migo_capi_abi::MIGO_ABI_VERSION_CURRENT,
    );
    let mut engine: *mut MigoEngine = ptr::null_mut();
    assert_eq!(unsafe { migo_engine_create(&config, &mut engine) }, MIGO_OK);
    assert!(!engine.is_null());
    engine
}

fn create_session(engine: *mut MigoEngine) -> *mut MigoSession {
    let config = session_config();
    let mut session: *mut MigoSession = ptr::null_mut();
    assert_eq!(
        unsafe { migo_session_create(engine, &config, &mut session) },
        MIGO_OK
    );
    assert!(!session.is_null());
    session
}

#[test]
fn two_sessions_coexist_on_one_engine() {
    let engine = create_engine("two-sessions-coexist");
    let first = create_session(engine);
    let second = create_session(engine);
    assert_ne!(
        first, second,
        "each session must be its own allocation, not a shared handle"
    );
    assert_eq!(unsafe { migo_session_destroy(first) }, MIGO_OK);
    assert_eq!(unsafe { migo_session_destroy(second) }, MIGO_OK);
    assert_eq!(unsafe { migo_engine_destroy(engine) }, MIGO_OK);
}

/// Section 6.4 lists this among the properties that become gated requirements.
#[test]
fn engine_destruction_is_refused_while_either_session_is_live() {
    let engine = create_engine("engine-destroy-refused");
    let first = create_session(engine);
    let second = create_session(engine);

    assert_eq!(
        unsafe { migo_engine_destroy(engine) },
        MIGO_ERROR_INVALID_STATE,
        "two live sessions must block destruction"
    );
    assert_eq!(unsafe { migo_session_destroy(first) }, MIGO_OK);
    assert_eq!(
        unsafe { migo_engine_destroy(engine) },
        MIGO_ERROR_INVALID_STATE,
        "one remaining live session must still block destruction"
    );
    assert_eq!(unsafe { migo_session_destroy(second) }, MIGO_OK);
    assert_eq!(
        unsafe { migo_engine_destroy(engine) },
        MIGO_OK,
        "destruction must succeed once the last session is gone"
    );
}

/// Destroying one Session must leave the other fully usable, which is the part a
/// shared-state regression would break: a process-global cleared on the first
/// teardown would take the survivor's state with it.
#[test]
fn destroying_one_session_leaves_the_other_usable() {
    let engine = create_engine("one-session-teardown");
    let doomed = create_session(engine);
    let survivor = create_session(engine);

    assert_eq!(unsafe { migo_session_destroy(doomed) }, MIGO_OK);

    // The survivor still answers, and the engine still refuses to go.
    assert_eq!(
        unsafe { migo_engine_destroy(engine) },
        MIGO_ERROR_INVALID_STATE
    );
    let replacement = create_session(engine);
    assert_ne!(replacement, survivor);

    assert_eq!(unsafe { migo_session_destroy(survivor) }, MIGO_OK);
    assert_eq!(unsafe { migo_session_destroy(replacement) }, MIGO_OK);
    assert_eq!(unsafe { migo_engine_destroy(engine) }, MIGO_OK);
}

/// Section 6.4 requires that two Sessions may be driven concurrently from two host
/// threads. The barrier makes the two creations actually overlap rather than
/// merely happen on different threads, which is what a lock held across session
/// construction would show up as.
#[test]
fn two_sessions_are_created_concurrently_from_two_host_threads() {
    let engine = EnginePtr(create_engine("two-host-threads"));
    let barrier = Arc::new(Barrier::new(2));

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                SessionPtr(create_session(engine.get()))
            })
        })
        .collect();

    let sessions: Vec<SessionPtr> = handles
        .into_iter()
        .map(|handle| handle.join().expect("session thread must not panic"))
        .collect();
    assert_ne!(
        sessions[0].0, sessions[1].0,
        "concurrent creation must not hand back one shared session"
    );

    // Teardown also overlaps, each session from its own thread.
    let barrier = Arc::new(Barrier::new(2));
    let teardown: Vec<_> = sessions
        .into_iter()
        .map(|session| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                assert_eq!(unsafe { migo_session_destroy(session.get()) }, MIGO_OK);
            })
        })
        .collect();
    for handle in teardown {
        handle.join().expect("teardown thread must not panic");
    }

    assert_eq!(unsafe { migo_engine_destroy(engine.get()) }, MIGO_OK);
}

/// Section 6.4 asks whether a process may create more than one Engine. It may, and
/// two Engines must not share the live-session accounting that gates destruction.
#[test]
fn two_engines_account_for_their_own_sessions() {
    let first = create_engine("two-engines-first");
    let second = create_engine("two-engines-second");
    let session = create_session(first);

    assert_eq!(
        unsafe { migo_engine_destroy(second) },
        MIGO_OK,
        "a session on one engine must not block another engine's destruction"
    );
    assert_eq!(
        unsafe { migo_engine_destroy(first) },
        MIGO_ERROR_INVALID_STATE
    );
    assert_eq!(unsafe { migo_session_destroy(session) }, MIGO_OK);
    assert_eq!(unsafe { migo_engine_destroy(first) }, MIGO_OK);
}
