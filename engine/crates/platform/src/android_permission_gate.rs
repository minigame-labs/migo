use std::collections::HashMap;
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

#[derive(Debug)]
struct Hosts {
    highest_opened_host_id: i32,
    live: HashMap<i32, HostState>,
}

impl Default for Hosts {
    fn default() -> Self {
        Self {
            highest_opened_host_id: -1,
            live: HashMap::new(),
        }
    }
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

    pub(crate) fn open(&self, host_id: i32) -> bool {
        let mut hosts = self.hosts.lock();
        if host_id <= hosts.highest_opened_host_id || hosts.live.contains_key(&host_id) {
            return false;
        }
        hosts.highest_opened_host_id = host_id;
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
        gate.open(41);
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
        gate.open(51);

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
        gate.open(56);
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
        gate.open(61);
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
        gate.open(71);
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
