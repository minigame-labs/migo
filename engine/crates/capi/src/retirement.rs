//! The Hosts an Engine has retired and must join before it may die.
//!
//! Its own module so the `Mutex` is unreachable from the destruction path.
//! `take` is the only way out and it yields owned handles, which makes "join a
//! Host while holding an Engine lock" not expressible rather than merely tested
//! for. That deadlocks for real: a Host on its way out reaches
//! `migo_session_destroy` and `migo_engine_destroy`, both of which take these
//! locks, so a joiner holding one waits for a thread that is waiting for it.
//!
//! A probe test cannot stand in for this. Observing "no lock is held" during a
//! blocking call is sampling, and a sample has no ordering against the call: the
//! probe this replaced ran before the destroying thread had been scheduled and
//! passed 50 runs out of 50 with a lock deliberately held across the join.

use std::sync::{Mutex, MutexGuard, PoisonError};

use migo_core::HostThread;

/// Retired Hosts, owned until someone joins them.
#[derive(Default)]
pub(crate) struct RetirementSet {
    hosts: Mutex<Vec<HostThread>>,
}

impl RetirementSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Ask `host` to stop, and keep it until it is joined.
    ///
    /// A failed request is logged rather than propagated: the handle still has
    /// to be retained, because dropping it here is what would leak the thread.
    pub(crate) fn retire(&self, host: HostThread) {
        if let Err(error) = host.request_shutdown() {
            tracing::error!("failed to request shutdown for Host {}: {error}", host.id());
        }
        self.locked().push(host);
    }

    /// Every retired Host, owned, leaving the set empty.
    ///
    /// `Err` when one of them is the calling thread: a Host cannot join itself,
    /// and the caller has to be able to refuse instead of deadlocking.
    pub(crate) fn take(&self) -> Result<Vec<HostThread>, ()> {
        let mut hosts = self.locked();
        if hosts.iter().any(HostThread::is_current_thread) {
            return Err(());
        }
        Ok(std::mem::take(&mut *hosts))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.locked().len()
    }

    /// Private, and the reason this type is a module: a `MutexGuard` that
    /// escaped could be alive across a join.
    fn locked(&self) -> MutexGuard<'_, Vec<HostThread>> {
        self.hosts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
    };

    use migo_core::HostThread;

    use super::RetirementSet;

    /// A retired Host that parks until released, so a test can hold one across
    /// an observation and still join it.
    fn parked_host(id: i32) -> (HostThread, mpsc::Sender<()>) {
        let (release_tx, release_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name(format!("Migo-Main-retirement-{id}"))
            .spawn(move || {
                let _ = release_rx.recv();
            })
            .expect("spawn retired Host");
        (HostThread::from_join_handle_for_test(id, join), release_tx)
    }

    #[test]
    fn take_hands_over_every_host_and_leaves_the_set_empty() {
        let set = RetirementSet::new();
        let (first, release_first) = parked_host(9_001);
        let (second, release_second) = parked_host(9_002);
        set.retire(first);
        set.retire(second);
        assert_eq!(set.len(), 2);

        let taken = set.take().expect("no retired Host is this thread");

        assert_eq!(taken.len(), 2);
        assert_eq!(
            set.len(),
            0,
            "take must not leave a Host behind to be joined twice"
        );
        release_first.send(()).expect("release first Host");
        release_second.send(()).expect("release second Host");
        for mut host in taken {
            host.join().expect("join released Host");
        }
    }

    #[test]
    fn take_refuses_from_a_retired_host_and_keeps_it_for_a_retry() {
        let set = Arc::new(RetirementSet::new());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let refused = Arc::new(AtomicBool::new(false));

        let set_on_host = Arc::clone(&set);
        let refused_on_host = Arc::clone(&refused);
        let join = thread::Builder::new()
            .name("Migo-Main-retirement-self".to_owned())
            .spawn(move || {
                ready_rx.recv().expect("wait until retired");
                refused_on_host.store(set_on_host.take().is_err(), Ordering::Release);
                release_tx.send(()).expect("publish refusal");
            })
            .expect("spawn retired Host");
        set.retire(HostThread::from_join_handle_for_test(9_003, join));

        ready_tx.send(()).expect("tell the Host it is retired");
        release_rx.recv().expect("refusal published");

        assert!(
            refused.load(Ordering::Acquire),
            "a Host must not be handed its own handle to join"
        );
        let mut taken = set.take().expect("another thread may take it");
        assert_eq!(
            taken.len(),
            1,
            "a refused take must keep the Host for a retry"
        );
        taken[0].join().expect("join the retired Host");
    }
}
