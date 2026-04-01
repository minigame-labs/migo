//! Simple in-process DNS cache with TTL-based expiration.
//!
//! Used by `op_prefetch_dns` to pre-resolve hostnames so that subsequent
//! `fetch()` calls hit warm DNS entries in the OS resolver cache, and by
//! the SSRF-checking resolver to avoid repeated lookups for the same host.
//!
//! The cache is intentionally simple: a global `Mutex<HashMap>` with a
//! configurable TTL (default 5 minutes). Entries are lazily evicted on
//! `get()` when expired.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tracing::debug;

/// Default time-to-live for cached DNS entries.
const DEFAULT_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// Maximum number of entries to prevent unbounded growth.
const MAX_ENTRIES: usize = 256;

/// Global DNS cache instance, lazily initialized.
static DNS_CACHE: Mutex<Option<DnsCache>> = Mutex::new(None);

#[allow(dead_code)]
struct DnsCacheEntry {
    addrs: Vec<SocketAddr>,
    expires: Instant,
}

pub(crate) struct DnsCache {
    entries: HashMap<String, DnsCacheEntry>,
    ttl: Duration,
}

#[allow(dead_code)]
impl DnsCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            ttl: DEFAULT_TTL,
        }
    }

    fn get(&self, host: &str) -> Option<&[SocketAddr]> {
        let entry = self.entries.get(host)?;
        if Instant::now() >= entry.expires {
            return None;
        }
        Some(&entry.addrs)
    }

    fn insert(&mut self, host: &str, addrs: Vec<SocketAddr>) {
        // Evict expired entries if we are at capacity
        if self.entries.len() >= MAX_ENTRIES {
            let now = Instant::now();
            self.entries.retain(|_, e| now < e.expires);
        }
        // If still at capacity after eviction, skip insertion
        if self.entries.len() >= MAX_ENTRIES {
            return;
        }
        self.entries.insert(
            host.to_string(),
            DnsCacheEntry {
                addrs,
                expires: Instant::now() + self.ttl,
            },
        );
    }
}

/// Look up a host in the DNS cache.
///
/// Returns `Some(addrs)` if the host is cached and not expired, `None` otherwise.
#[allow(dead_code)]
pub(crate) fn cache_lookup(host: &str) -> Option<Vec<SocketAddr>> {
    let guard = DNS_CACHE.lock().ok()?;
    let cache = guard.as_ref()?;
    cache.get(host).map(|s| s.to_vec())
}

/// Insert a resolved host into the DNS cache.
pub(crate) fn cache_insert(host: &str, addrs: Vec<SocketAddr>) {
    if let Ok(mut guard) = DNS_CACHE.lock() {
        let cache = guard.get_or_insert_with(DnsCache::new);
        cache.insert(host, addrs);
    }
}

/// Pre-resolve a list of hostnames in the background using Tokio tasks.
///
/// Each hostname is resolved via `tokio::net::lookup_host` and the result
/// is stored in the global DNS cache. This warms both our cache and the
/// OS resolver cache so that subsequent `fetch()` calls are faster.
///
/// SSRF filtering is applied: resolved addresses pointing to private/loopback
/// ranges are **not** cached (they would be rejected by `SsrfCheckingResolver`
/// anyway).
pub(crate) fn pre_resolve(hosts: Vec<String>) {
    for host in hosts {
        tokio::spawn(async move {
            let lookup_addr = format!("{}:0", host);
            match tokio::net::lookup_host(&lookup_addr).await {
                Ok(addrs) => {
                    let addrs: Vec<SocketAddr> = addrs.collect();
                    // Only cache if ALL addresses pass SSRF checks
                    let all_safe = addrs
                        .iter()
                        .all(|a| !super::address_filter::is_blocked_address(a));
                    if all_safe && !addrs.is_empty() {
                        debug!("dns_cache: pre-resolved {} -> {} addrs", host, addrs.len());
                        cache_insert(&host, addrs);
                    }
                }
                Err(e) => {
                    debug!("dns_cache: failed to pre-resolve {}: {}", host, e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn sa(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), port)
    }

    #[test]
    fn insert_and_lookup() {
        let mut cache = DnsCache::new();
        let addrs = vec![sa(8, 8, 8, 8, 0), sa(8, 8, 4, 4, 0)];
        cache.insert("dns.google", addrs.clone());
        let got = cache.get("dns.google").unwrap();
        assert_eq!(got, &addrs[..]);
    }

    #[test]
    fn missing_returns_none() {
        let cache = DnsCache::new();
        assert!(cache.get("missing.example").is_none());
    }

    #[test]
    fn expired_returns_none() {
        let mut cache = DnsCache::new();
        cache.ttl = Duration::from_millis(0);
        cache.insert("expired.example", vec![sa(1, 1, 1, 1, 0)]);
        // TTL is 0ms, so the entry is immediately expired
        assert!(cache.get("expired.example").is_none());
    }

    #[test]
    fn max_entries_evicts_expired() {
        let mut cache = DnsCache::new();
        cache.ttl = Duration::from_millis(0);
        // Fill to MAX_ENTRIES with expired entries
        for i in 0..MAX_ENTRIES {
            cache.entries.insert(
                format!("host{}.example", i),
                DnsCacheEntry {
                    addrs: vec![sa(1, 0, 0, i as u8, 0)],
                    expires: Instant::now(), // already expired
                },
            );
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);

        // Inserting with a non-zero TTL should succeed after eviction
        cache.ttl = Duration::from_secs(300);
        cache.insert("new.example", vec![sa(8, 8, 8, 8, 0)]);
        assert!(cache.get("new.example").is_some());
    }
}
