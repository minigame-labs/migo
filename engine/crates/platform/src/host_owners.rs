use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Mutex,
};

use migo_core::{HostId, HostThread};

/// Native-platform ownership for Hosts whose public API routes commands by ID.
///
/// The mutex protects only ownership transfer. Callers remove a Host before
/// requesting shutdown or joining, so no platform operation can wait while the
/// ownership map is locked.
pub(crate) struct HostOwners {
    hosts: Mutex<HashMap<HostId, HostThread>>,
}

impl HostOwners {
    pub(crate) fn new() -> Self {
        Self {
            hosts: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn insert(&self, host: HostThread) -> Result<HostId, HostThread> {
        let host_id = host.id();
        let mut hosts = self
            .hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match hosts.entry(host_id) {
            Entry::Vacant(entry) => {
                entry.insert(host);
                Ok(host_id)
            }
            Entry::Occupied(_) => Err(host),
        }
    }

    pub(crate) fn take(&self, host_id: HostId) -> Option<HostThread> {
        self.hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&host_id)
    }

    /// Transfers one owner into shutdown, restoring that same owner on failure.
    pub(crate) fn shutdown_with<E>(
        &self,
        host_id: HostId,
        shutdown: impl FnOnce(&mut HostThread) -> Result<(), E>,
    ) -> Result<bool, E> {
        let Some(mut host) = self.take(host_id) else {
            return Ok(false);
        };
        match shutdown(&mut host) {
            Ok(()) => Ok(true),
            Err(error) => {
                let restored = self.insert(host);
                assert!(
                    restored.is_ok(),
                    "failed shutdown must restore the unique Host owner"
                );
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::HostOwners;

    fn test_host(host_id: i32, name: &str) -> migo_core::HostThread {
        let join = thread::Builder::new()
            .name(name.to_owned())
            .spawn(|| {})
            .expect("spawn inert Host");
        migo_core::HostThread::from_join_handle_for_test(host_id, join)
    }

    #[test]
    fn insertion_returns_the_owned_host_id() {
        let owners = HostOwners::new();

        assert_eq!(
            owners
                .insert(test_host(101, "Migo-Main-owner-insert"))
                .expect("insert Host"),
            101
        );
        owners
            .take(101)
            .expect("take Host")
            .join()
            .expect("join Host");
    }

    #[test]
    fn duplicate_insertion_does_not_replace_the_live_owner() {
        let owners = HostOwners::new();
        owners
            .insert(test_host(102, "Migo-Main-owner-original"))
            .expect("insert original Host");

        let mut duplicate = owners
            .insert(test_host(102, "Migo-Main-owner-duplicate"))
            .expect_err("duplicate ID must fail closed");
        assert_eq!(duplicate.id(), 102);
        duplicate.join().expect("join rejected duplicate");

        owners
            .take(102)
            .expect("original owner remains")
            .join()
            .expect("join original Host");
    }

    #[test]
    fn terminal_take_transfers_ownership_exactly_once() {
        let owners = HostOwners::new();
        owners
            .insert(test_host(103, "Migo-Main-owner-take"))
            .expect("insert Host");

        let mut host = owners.take(103).expect("first take owns Host");
        assert!(owners.take(103).is_none());
        assert!(owners.take(i32::MAX).is_none());
        host.join().expect("join Host");
    }

    #[test]
    fn failed_shutdown_restores_same_owner_for_retry() {
        let owners = HostOwners::new();
        owners
            .insert(test_host(104, "Migo-Main-owner-retry"))
            .expect("insert Host");

        let first = owners.shutdown_with(104, |_host| Err::<(), _>("transient join failure"));
        assert_eq!(first, Err("transient join failure"));

        let mut duplicate = owners
            .insert(test_host(104, "Migo-Main-owner-retry-duplicate"))
            .expect_err("failed shutdown must retain the original owner");
        duplicate.join().expect("join rejected duplicate");

        assert_eq!(
            owners.shutdown_with(104, |host| host.join().map_err(|error| error.to_string())),
            Ok(true)
        );
        assert!(owners.take(104).is_none());
    }
}
