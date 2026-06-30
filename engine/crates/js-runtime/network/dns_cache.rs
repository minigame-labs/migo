//! Background DNS pre-resolution.
//!
//! `op_prefetch_dns` calls [`pre_resolve`] to warm the **OS** resolver
//! cache so subsequent `fetch()` / socket connects skip the DNS round
//! trip.
//!
//! There used to be an in-process `Mutex<HashMap>` cache here as well,
//! but nothing ever read it: the SSRF-checking resolver deliberately
//! re-resolves per request (so a rebound A-record can't be served from a
//! stale entry), and `fetch` never consulted it. A write-only cache just
//! costs a global lock plus dead memory, so it was removed — warming the
//! OS resolver is the part that actually helps, and that lives on here.

use std::net::SocketAddr;

use tracing::debug;

/// Pre-resolve a list of hostnames in the background, warming the OS
/// resolver cache so later connects are faster.
///
/// SSRF filtering is applied for parity with the request-time resolver:
/// if a host resolves entirely outside the blocked ranges it is treated
/// as a successful warm; hosts that only resolve to private/loopback
/// ranges are logged and skipped. The op returns immediately — each host
/// is resolved on its own background Tokio task.
pub(crate) fn pre_resolve(hosts: Vec<String>) {
    for host in hosts {
        tokio::spawn(async move {
            let lookup_addr = format!("{}:0", host);
            match tokio::net::lookup_host(&lookup_addr).await {
                Ok(addrs) => {
                    let addrs: Vec<SocketAddr> = addrs.collect();
                    let all_safe = addrs
                        .iter()
                        .all(|a| !super::address_filter::is_blocked_address(a));
                    if all_safe && !addrs.is_empty() {
                        debug!("dns_cache: pre-resolved {} -> {} addrs", host, addrs.len());
                    } else {
                        debug!("dns_cache: skipped {} (no safe public address)", host);
                    }
                }
                Err(e) => {
                    debug!("dns_cache: failed to pre-resolve {}: {}", host, e);
                }
            }
        });
    }
}
