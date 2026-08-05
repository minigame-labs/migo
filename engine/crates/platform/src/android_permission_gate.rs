use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use shared::services::Scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Denied;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Active,
    Closing,
}

#[derive(Debug)]
struct HostPermissions {
    lifecycle: Lifecycle,
    scopes: HashMap<Scope, bool>,
    active_operations: usize,
}

#[derive(Debug)]
struct HostControl {
    transition: Mutex<()>,
    permissions: Mutex<HostPermissions>,
    idle: Condvar,
}

type HostState = Arc<HostControl>;

#[derive(Debug, Default)]
struct Hosts {
    /// Ids whose session has been cleared. A tombstone must reject exactly the
    /// ids that were retired -- not every id at or below the highest ever
    /// opened. Ids are allocated on the caller thread but opened from each
    /// session's own thread, so they do not arrive here in allocation order,
    /// and a high-water mark would refuse a live lower-id session outright.
    /// Bounded by the number of sessions the process ever created.
    retired: HashSet<i32>,
    live: HashMap<i32, HostState>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UpdateError<E> {
    Closed,
    Cleanup(E),
}

/// Serializes a session's protected Android calls with permission updates.
///
/// The main JS Host is serialized already, but workers and host permission
/// callbacks can arrive concurrently. Admission is recorded under the
/// per-host mutex, while the external operation runs under a counted lease so
/// revocation can wait without retaining that mutex across JNI. The
/// `parking_lot` primitives are deliberately non-poisoning: a caught panic must
/// not prevent later cleanup.
#[derive(Default)]
pub(crate) struct PermissionGate {
    hosts: Mutex<Hosts>,
}

struct OperationLease<'a> {
    host: &'a HostControl,
}

impl Drop for OperationLease<'_> {
    fn drop(&mut self) {
        let mut state = self.host.permissions.lock();
        state.active_operations -= 1;
        if state.active_operations == 0 {
            self.host.idle.notify_all();
        }
    }
}

#[cfg_attr(test, allow(dead_code))]
impl PermissionGate {
    fn host_state(&self, host_id: i32) -> Option<HostState> {
        self.hosts.lock().live.get(&host_id).cloned()
    }

    /// Admit a session id, or refuse one that is already live or was retired.
    ///
    /// Refusal is not advisory: no `HostControl` exists for a refused id, so
    /// every later permission check for it is denied. Callers must not discard
    /// the answer.
    #[must_use]
    pub(crate) fn open(&self, host_id: i32) -> bool {
        let mut hosts = self.hosts.lock();
        if hosts.retired.contains(&host_id) || hosts.live.contains_key(&host_id) {
            return false;
        }
        hosts.live.insert(
            host_id,
            Arc::new(HostControl {
                transition: Mutex::new(()),
                permissions: Mutex::new(HostPermissions {
                    lifecycle: Lifecycle::Active,
                    scopes: HashMap::new(),
                    active_operations: 0,
                }),
                idle: Condvar::new(),
            }),
        );
        true
    }

    pub(crate) fn run<T>(
        &self,
        host_id: i32,
        required_scope: Option<Scope>,
        operation: impl FnOnce() -> T,
    ) -> Result<T, Denied> {
        let host = self.host_state(host_id).ok_or(Denied)?;
        {
            let mut state = host.permissions.lock();
            if state.lifecycle == Lifecycle::Closing {
                return Err(Denied);
            }
            if let Some(scope) = required_scope
                && state.scopes.get(&scope) != Some(&true)
            {
                return Err(Denied);
            }
            state.active_operations += 1;
        }
        let lease = OperationLease { host: &host };
        let result = operation();
        drop(lease);
        Ok(result)
    }

    pub(crate) fn update<E>(
        &self,
        host_id: i32,
        scope: Scope,
        granted: bool,
        cleanup: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), UpdateError<E>> {
        let host = self.host_state(host_id).ok_or(UpdateError::Closed)?;
        let _transition = host.transition.lock();
        {
            let mut state = host.permissions.lock();
            if state.lifecycle == Lifecycle::Closing {
                return Err(UpdateError::Closed);
            }
            state.scopes.insert(scope, granted);
            if !granted {
                while state.active_operations != 0 {
                    host.idle.wait(&mut state);
                }
            }
        }
        if !granted {
            cleanup().map_err(UpdateError::Cleanup)?;
        }
        Ok(())
    }

    pub(crate) fn scope_state(&self, host_id: i32, scope: Scope) -> Option<bool> {
        let host = self.host_state(host_id)?;
        let state = host.permissions.lock();
        if state.lifecycle == Lifecycle::Closing {
            return None;
        }
        state.scopes.get(&scope).copied()
    }

    pub(crate) fn clear(&self, host_id: i32) {
        self.clear_with(host_id, || {});
    }

    fn clear_with(&self, host_id: i32, while_closing: impl FnOnce()) {
        let Some(host) = self.host_state(host_id) else {
            return;
        };
        let _transition = host.transition.lock();
        {
            let mut state = host.permissions.lock();
            state.lifecycle = Lifecycle::Closing;
            state.scopes.clear();
            while state.active_operations != 0 {
                host.idle.wait(&mut state);
            }
        }
        while_closing();
        let mut hosts = self.hosts.lock();
        if hosts
            .live
            .get(&host_id)
            .is_some_and(|current| Arc::ptr_eq(current, &host))
        {
            hosts.live.remove(&host_id);
            hosts.retired.insert(host_id);
        }
    }

    /// Whether `host_id` belonged to a session that has been cleared.
    ///
    /// Retirement is monotonic, so reading it after a refusal cannot turn a
    /// retired id back into a live one.
    fn is_retired(&self, host_id: i32) -> bool {
        self.hosts.lock().retired.contains(&host_id)
    }

    /// Admit a session id, reporting the one refusal that is a bug.
    ///
    /// A refusal is legitimate when the id is still live: a session restart
    /// rebuilds device services for a host whose grants must survive. A
    /// *retired* id is not -- no `HostControl` will ever exist for it, so every
    /// permission check that session makes is denied for its whole life, which
    /// content cannot distinguish from the user refusing. That is silent, so it
    /// is logged loudly and trips a debug build.
    pub(crate) fn open_or_report(&self, host_id: i32) {
        if !self.open(host_id) && self.is_retired(host_id) {
            tracing::error!(
                host_id,
                "permission gate refused a retired host id; every permission \
                 check for this session will be denied"
            );
            debug_assert!(
                false,
                "permission gate refused retired host id {host_id}; \
                 permissions will be denied for this session"
            );
        }
    }

    #[cfg(test)]
    fn live_host_count_for_tests(&self) -> usize {
        self.hosts.lock().live.len()
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;

    use super::*;

    #[test]
    fn revocation_waits_for_admitted_operation_then_tears_it_down() {
        let gate = Arc::new(PermissionGate::default());
        assert!(gate.open(41), "test setup failed to open the host");
        gate.update(41, Scope::Camera, true, || Ok::<(), ()>(()))
            .unwrap();

        let resource_live = Arc::new(Mutex::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let operation_gate = gate.clone();
        let operation_resource = resource_live.clone();
        let operation = thread::spawn(move || {
            operation_gate
                .run(41, Some(Scope::Camera), || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    *operation_resource.lock().unwrap() = true;
                })
                .unwrap();
        });
        entered_rx.recv().unwrap();

        let (revoked_tx, revoked_rx) = mpsc::channel();
        let revocation_gate = gate.clone();
        let revocation_resource = resource_live.clone();
        let revocation = thread::spawn(move || {
            revocation_gate
                .update(41, Scope::Camera, false, || {
                    *revocation_resource.lock().unwrap() = false;
                    Ok::<(), ()>(())
                })
                .unwrap();
            revoked_tx.send(()).unwrap();
        });

        assert!(
            revoked_rx.try_recv().is_err(),
            "revocation bypassed the live operation"
        );
        release_tx.send(()).unwrap();
        operation.join().unwrap();
        revocation.join().unwrap();
        revoked_rx.recv().unwrap();

        assert!(
            !*resource_live.lock().unwrap(),
            "resource survived revocation"
        );
        assert_eq!(gate.run(41, Some(Scope::Camera), || ()), Err(Denied));
    }

    #[test]
    fn caught_operation_panic_does_not_block_later_cleanup() {
        let gate = PermissionGate::default();
        assert!(gate.open(51), "test setup failed to open the host");

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = gate.run(51, None, || panic!("operation failed"));
        }));
        assert!(panic.is_err());

        let mut cleaned = false;
        gate.update(51, Scope::Camera, false, || {
            cleaned = true;
            Ok::<(), ()>(())
        })
        .unwrap();
        assert!(cleaned, "a prior operation panic prevented cleanup");
    }

    #[test]
    fn protected_operation_does_not_hold_host_mutex_across_external_code() {
        let gate = PermissionGate::default();
        assert!(gate.open(56), "test setup failed to open the host");
        gate.update(56, Scope::Camera, true, || Ok::<(), ()>(()))
            .unwrap();
        let host = gate.host_state(56).expect("open host");

        gate.run(56, Some(Scope::Camera), || {
            assert!(
                host.permissions.try_lock().is_some(),
                "protected operation retained the Rust host mutex across JNI"
            );
        })
        .unwrap();
    }

    #[test]
    fn clear_waits_for_inflight_update_then_leaves_a_tombstone() {
        let gate = Arc::new(PermissionGate::default());
        assert!(gate.open(61), "test setup failed to open the host");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let updating = gate.clone();
        let update = thread::spawn(move || {
            updating.update(61, Scope::Camera, false, || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok::<(), ()>(())
            })
        });
        entered_rx.recv().unwrap();

        let clearing = gate.clone();
        let (cleared_tx, cleared_rx) = mpsc::channel();
        let clear = thread::spawn(move || {
            clearing.clear(61);
            cleared_tx.send(()).unwrap();
        });
        assert!(cleared_rx.try_recv().is_err());

        release_tx.send(()).unwrap();
        update.join().unwrap().unwrap();
        clear.join().unwrap();
        assert_eq!(gate.scope_state(61, Scope::Camera), None);
        assert!(
            gate.update(61, Scope::Camera, true, || Ok::<(), ()>(()))
                .is_err()
        );
    }

    #[test]
    fn update_waiting_behind_clear_cannot_recreate_the_host() {
        let gate = Arc::new(PermissionGate::default());
        assert!(gate.open(71), "test setup failed to open the host");
        let (closing_tx, closing_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let clearing = gate.clone();
        let clear = thread::spawn(move || {
            clearing.clear_with(71, || {
                closing_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
        });
        closing_rx.recv().unwrap();

        let updating = gate.clone();
        let (updated_tx, updated_rx) = mpsc::channel();
        let update = thread::spawn(move || {
            let result = updating.update(71, Scope::Camera, true, || Ok::<(), ()>(()));
            updated_tx.send(result).unwrap();
        });
        assert!(updated_rx.try_recv().is_err());

        release_tx.send(()).unwrap();
        clear.join().unwrap();
        assert!(updated_rx.recv().unwrap().is_err());
        update.join().unwrap();
        assert_eq!(gate.scope_state(71, Scope::Camera), None);
    }

    #[test]
    fn a_lower_host_id_opened_after_a_higher_one_still_gets_its_permissions() {
        // Host ids are allocated on the caller thread but opened from each
        // session's own thread, so two sessions starting together can arrive
        // here in the opposite order. Neither is retired, so both must open.
        let gate = PermissionGate::default();
        assert!(gate.open(7), "the first session was refused");
        assert!(
            gate.open(5),
            "a live session was refused because a higher id opened first"
        );

        gate.update(5, Scope::Camera, true, || Ok::<(), ()>(()))
            .expect("granting a scope on an open host");
        assert_eq!(
            gate.run(5, Some(Scope::Camera), || ()),
            Ok(()),
            "a granted scope surfaced as a denial"
        );
        assert_eq!(gate.scope_state(5, Scope::Camera), Some(true));
    }

    #[test]
    fn a_cleared_id_stays_retired_when_a_higher_id_opens_afterwards() {
        let gate = PermissionGate::default();
        assert!(gate.open(11));
        gate.clear(11);

        // A later, unrelated session must not resurrect the retired id.
        assert!(gate.open(12));
        assert!(!gate.open(11), "a retired id was reopened");
        assert_eq!(gate.run(11, None, || ()), Err(Denied));
        assert_eq!(gate.scope_state(11, Scope::Camera), None);
    }

    #[test]
    fn clearing_one_host_leaves_another_live_host_untouched() {
        let gate = PermissionGate::default();
        assert!(gate.open(22));
        assert!(gate.open(21));
        gate.update(21, Scope::Camera, true, || Ok::<(), ()>(()))
            .expect("granting a scope on an open host");

        gate.clear(22);

        assert_eq!(
            gate.run(21, Some(Scope::Camera), || ()),
            Ok(()),
            "clearing a sibling host denied a live host"
        );
        // A fresh id below the cleared one is still admissible.
        assert!(gate.open(20));
        assert_eq!(gate.run(20, None, || ()), Ok(()));
        assert_eq!(gate.live_host_count_for_tests(), 2);
    }

    #[test]
    fn reopening_a_live_host_is_tolerated_because_a_restart_rebuilds_services() {
        // `on_restart` rebuilds device services for the same, still-live id and
        // does not clear permissions, so its grants must survive untouched.
        let gate = PermissionGate::default();
        gate.open_or_report(31);
        gate.update(31, Scope::Camera, true, || Ok::<(), ()>(()))
            .expect("granting a scope on an open host");

        gate.open_or_report(31);

        assert_eq!(
            gate.scope_state(31, Scope::Camera),
            Some(true),
            "rebuilding services for a live host dropped its grants"
        );
        assert_eq!(gate.run(31, Some(Scope::Camera), || ()), Ok(()));
        assert_eq!(gate.live_host_count_for_tests(), 1);
    }

    /// The report is a `debug_assert!`, so what "reported" means differs by
    /// profile and both are asserted here. A single unconditional
    /// `#[should_panic]` failed the release suite outright, which is worse than
    /// the silence it was written to catch: it makes `cargo test --release`
    /// unusable and so stops anyone running the rest of these tests in the
    /// profile that ships.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "refused retired host id")]
    fn reopening_a_retired_host_trips_a_debug_build() {
        let gate = PermissionGate::default();
        gate.open_or_report(32);
        gate.clear(32);
        gate.open_or_report(32);
    }

    /// In release the report is a log line, so the observable contract is that
    /// the call returns and the retired id is still refused. Without this the
    /// release profile would assert nothing at all about this case.
    #[test]
    #[cfg(not(debug_assertions))]
    fn reopening_a_retired_host_returns_without_admitting_it_in_release() {
        let gate = PermissionGate::default();
        gate.open_or_report(32);
        gate.clear(32);
        gate.open_or_report(32);
        assert_eq!(gate.live_host_count_for_tests(), 0);
        assert!(!gate.open(32));
    }

    #[test]
    fn successful_clear_reclaims_hosts_without_allowing_id_reuse() {
        let gate = PermissionGate::default();
        for host_id in 1000..2000 {
            assert!(gate.open(host_id));
            gate.clear(host_id);
        }

        assert_eq!(gate.live_host_count_for_tests(), 0);
        assert!(!gate.open(1500));
        assert!(gate.open(2000));
        assert!(!gate.open(2000));
        gate.clear(2000);
        assert_eq!(gate.live_host_count_for_tests(), 0);
        assert!(!gate.open(2000));
    }
}
