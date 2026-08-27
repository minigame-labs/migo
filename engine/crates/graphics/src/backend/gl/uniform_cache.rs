//! Allocation-reusing storage update for the per-canvas WebGL uniform cache.

use std::collections::HashMap;

const MAX_CACHED_UNIFORM_VALUE_BYTES: usize = 64 * 1024;

/// Compare-and-store against the shadow for `(program, location)`.
///
/// Returns `true` when the upload is not redundant and must be issued.
///
/// **One hash probe on each of the two paths a frame actually takes.** Both
/// deduping an unchanged value and overwriting a changed one are resolved by
/// the single `get_mut` below, because a value that changed is written into the
/// allocation the comparison just read. Only a location this program has not
/// cached yet pays a second probe, and that happens once per location per
/// program rather than once per frame.
///
/// The previous shape probed `get` to compare, `contains_key` to decide whether
/// eviction would displace this very key, and `entry` to store — three probes
/// to change a uniform in a full cache, two in a warm one, for a path a
/// shader-heavy frame runs thousands of times.
pub(crate) fn update(
    cache: &mut HashMap<(u32, u32), Vec<u8>>,
    maximum_entries: usize,
    program: u32,
    location: u32,
    value: &[u8],
) -> bool {
    assert!(
        maximum_entries > 0,
        "uniform cache must retain at least one entry"
    );
    let key = (program, location);
    if value.len() > MAX_CACHED_UNIFORM_VALUE_BYTES {
        cache.remove(&key);
        return true;
    }

    if let Some(stored) = cache.get_mut(&key) {
        if stored.as_slice() == value {
            return false;
        }
        // Same key, new bytes: reuse the allocation rather than reinserting.
        stored.clear();
        stored.extend_from_slice(value);
        return true;
    }

    // New key. Eviction can no longer displace it — the lookup above proved
    // it absent — so the `contains_key` the old shape needed here is gone.
    if cache.len() >= maximum_entries
        && let Some(evicted) = cache.keys().next().copied()
    {
        cache.remove(&evicted);
    }
    cache.insert(key, value.to_vec());
    true
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::update;

    #[test]
    fn changed_value_reuses_existing_allocation() {
        let mut cache = HashMap::new();
        assert!(update(&mut cache, 8, 1, 5, &[1, 2, 3, 4]));
        let first = cache.get(&(1, 5)).unwrap();
        let first_allocation = first.as_ptr();
        let first_capacity = first.capacity();

        assert!(update(&mut cache, 8, 1, 5, &[5, 6, 7, 8]));
        let second = cache.get(&(1, 5)).unwrap();
        assert_eq!(second.as_ptr(), first_allocation);
        assert_eq!(second.capacity(), first_capacity);
        assert_eq!(second.as_slice(), &[5, 6, 7, 8]);
    }

    #[test]
    fn identical_value_is_deduplicated() {
        let mut cache = HashMap::new();
        assert!(update(&mut cache, 8, 1, 5, &[1, 2]));
        assert!(!update(&mut cache, 8, 1, 5, &[1, 2]));
    }

    #[test]
    fn cache_never_exceeds_configured_entry_limit() {
        let mut cache = HashMap::new();
        for location in 0..16 {
            assert!(update(&mut cache, 4, 1, location, &[location as u8]));
            assert!(cache.len() <= 4);
        }
    }

    #[test]
    fn oversized_value_is_not_retained() {
        let mut cache = HashMap::new();
        let oversized = vec![7u8; 64 * 1024 + 1];

        assert!(update(&mut cache, 8, 1, 5, &oversized));
        assert!(
            !cache.contains_key(&(1, 5)),
            "pathological uniform values must not become retained cache capacity"
        );
    }
}
