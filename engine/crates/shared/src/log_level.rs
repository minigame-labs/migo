//! Which log level applies to the record a thread is about to emit.
//!
//! A log level arrives per session, in `RuntimeConfig`/`InitOptions`, but the sink
//! is process-wide: one logcat, one subscriber. The two facts were reconciled by
//! keeping a single level and letting the newest session overwrite it, so starting
//! a second game with `Off` silenced the first game that was started with `Debug`
//! — diagnostics destroyed by an unrelated session, which is the one direction a
//! verbosity setting must never move.
//!
//! Three tiers answer it, most specific first:
//!
//! 1. **This thread's session**, bound once when a host thread starts. A session's
//!    own work is the majority of what it logs, and that work runs on its host
//!    thread, so this is where a level can be honoured exactly.
//! 2. **The join over live sessions** — the most verbose of them — for threads that
//!    belong to no single session: the IO pool, the platform's callback threads,
//!    the JNI caller. A record there cannot be attributed, so the safe answer is
//!    the level of whichever session asked to see the most. Silence is the answer
//!    that loses evidence.
//! 3. **The process default**, from the build type or from a C host that installs
//!    diagnostics without ever creating a session.
//!
//! Reading it costs a thread-local read and, when the thread has no session, one
//! relaxed atomic load. That matters because it is read on *every* event before
//! anything is formatted: `tracing` allocates roughly 300 bytes per event
//! ([`crate::log_throttle`] documents the measurement), so a session that asked
//! for `Off` must not pay for another session's `Trace`. The registry behind the
//! join is only locked by session bring-up and teardown, the same discipline the
//! per-isolate console sink follows.
//!
//! Ordinals are the ones Java's `RuntimeConfig.LogLevel` uses — `Trace` is 0 and
//! `Off` is 5 — so **more verbose is numerically smaller**, and the join is a
//! minimum.

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::config::LogLevel;

/// The level to use when no session is registered and no thread is bound.
static DEFAULT_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Off as u8);

/// The most verbose level any live session asked for, or `NO_SESSIONS`.
///
/// Cached rather than derived on demand: it is read on the event path, where
/// taking the registry lock would put a cross-session lock on a path the content
/// drives.
static JOINED_LEVEL: AtomicU8 = AtomicU8::new(NO_SESSIONS);

const NO_SESSIONS: u8 = u8::MAX;

/// `(session id, level ordinal)` for every live session.
///
/// A `Vec` because sessions are counted in single digits and this is only ever
/// walked by bring-up and teardown; a map would allocate per entry to answer a
/// question a linear scan of five elements answers first.
static SESSION_LEVELS: Mutex<Vec<(i32, u8)>> = Mutex::new(Vec::new());

thread_local! {
    /// This thread's session level as `ordinal + 1`, or 0 when unbound.
    ///
    /// Offset by one so the unbound case is the zero value and needs no `Option`
    /// discriminant on a path read before every event.
    static THREAD_LEVEL: Cell<u8> = const { Cell::new(0) };
}

fn ordinal(level: LogLevel) -> u8 {
    level as i32 as u8
}

fn from_ordinal(value: u8) -> LogLevel {
    LogLevel::from(value as i32)
}

/// Set the level for threads and processes that have no session to speak for them.
pub fn set_default_level(level: LogLevel) {
    DEFAULT_LEVEL.store(ordinal(level), Ordering::Relaxed);
}

/// Bind `level` to the calling thread for as long as it runs.
///
/// Called once, at host-thread start, before any of that session's work. Not
/// unbound at teardown: the thread ends with the session, so there is no later
/// caller to give a stale answer to, and clearing it would need a guard on a path
/// whose failure mode is a panic during shutdown.
pub fn bind_thread_level(level: LogLevel) {
    THREAD_LEVEL.set(ordinal(level) + 1);
}

/// Record that `id` is live and wants `level`.
///
/// Replaces any previous entry for `id`: a restart keeps the session and may
/// arrive with a new configuration, and two entries for one session would let the
/// stale one hold the join open.
pub fn register_session(id: i32, level: LogLevel) {
    let mut sessions = lock_sessions();
    match sessions.iter_mut().find(|(known, _)| *known == id) {
        Some(entry) => entry.1 = ordinal(level),
        None => sessions.push((id, ordinal(level))),
    }
    republish(&sessions);
}

/// Forget `id`, so its level stops holding the join open.
pub fn unregister_session(id: i32) {
    let mut sessions = lock_sessions();
    sessions.retain(|(known, _)| *known != id);
    republish(&sessions);
}

/// The level that applies to a record this thread is about to emit.
pub fn effective_level() -> LogLevel {
    let bound = THREAD_LEVEL.get();
    if bound != 0 {
        return from_ordinal(bound - 1);
    }
    let joined = JOINED_LEVEL.load(Ordering::Relaxed);
    if joined != NO_SESSIONS {
        return from_ordinal(joined);
    }
    from_ordinal(DEFAULT_LEVEL.load(Ordering::Relaxed))
}

fn republish(sessions: &[(i32, u8)]) {
    let joined = sessions
        .iter()
        .map(|(_, level)| *level)
        .min()
        .unwrap_or(NO_SESSIONS);
    JOINED_LEVEL.store(joined, Ordering::Relaxed);
}

/// The registry, with a poisoned lock treated as usable.
///
/// A panic while holding it leaves the `Vec` structurally intact — every write is
/// a push, a retain or a field assignment — and refusing to read it afterwards
/// would turn one panic into a process with no diagnostics at all, which is the
/// state this module exists to prevent.
fn lock_sessions() -> std::sync::MutexGuard<'static, Vec<(i32, u8)>> {
    SESSION_LEVELS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every state this module keeps is process-wide, and cargo runs these on
    /// several threads at once, so they are serialised. Partitioning session ids
    /// per test would not be enough: the join and the default are single values
    /// every test reads.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// The lock, plus a registry emptied so each test's assertions depend only on
    /// what it registered itself.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut sessions = lock_sessions();
        sessions.clear();
        republish(&sessions);
        drop(sessions);
        THREAD_LEVEL.set(0);
        guard
    }

    #[test]
    fn a_thread_bound_to_a_session_uses_that_session_level() {
        let _exclusive = exclusive();
        register_session(1, LogLevel::Off);
        register_session(2, LogLevel::Trace);
        bind_thread_level(LogLevel::Off);

        // The point of the binding: another session at Trace must not make this
        // thread pay to format records its own session asked not to have.
        assert_eq!(effective_level(), LogLevel::Off);
    }

    #[test]
    fn an_unbound_thread_uses_the_most_verbose_live_session() {
        let _exclusive = exclusive();
        register_session(1, LogLevel::Off);
        register_session(2, LogLevel::Debug);

        assert_eq!(effective_level(), LogLevel::Debug);
    }

    /// The defect: the newest session used to overwrite one level for everybody.
    #[test]
    fn a_new_session_cannot_silence_an_existing_one() {
        let _exclusive = exclusive();
        register_session(1, LogLevel::Debug);
        assert_eq!(effective_level(), LogLevel::Debug);

        register_session(2, LogLevel::Off);
        assert_eq!(
            effective_level(),
            LogLevel::Debug,
            "a second session starting with Off silenced the first"
        );
    }

    #[test]
    fn a_closing_session_stops_holding_the_join_open() {
        let _exclusive = exclusive();
        register_session(1, LogLevel::Trace);
        register_session(2, LogLevel::Warn);
        assert_eq!(effective_level(), LogLevel::Trace);

        unregister_session(1);
        assert_eq!(effective_level(), LogLevel::Warn);
    }

    #[test]
    fn the_default_answers_only_while_no_session_is_live() {
        let _exclusive = exclusive();
        set_default_level(LogLevel::Error);
        assert_eq!(effective_level(), LogLevel::Error);

        register_session(1, LogLevel::Info);
        assert_eq!(effective_level(), LogLevel::Info);

        unregister_session(1);
        assert_eq!(
            effective_level(),
            LogLevel::Error,
            "the last session closing left no answer at all"
        );
    }

    #[test]
    fn re_registering_a_session_replaces_its_level_rather_than_adding_one() {
        let _exclusive = exclusive();
        register_session(1, LogLevel::Trace);
        register_session(1, LogLevel::Error);
        assert_eq!(
            lock_sessions().len(),
            1,
            "a restart left two entries, and the stale one holds the join open"
        );
        assert_eq!(effective_level(), LogLevel::Error);
    }

    #[test]
    fn every_level_survives_the_ordinal_round_trip() {
        for level in [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Off,
        ] {
            assert_eq!(from_ordinal(ordinal(level)), level);
        }
    }
}
