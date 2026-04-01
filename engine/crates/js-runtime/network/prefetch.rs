//! Asset prefetch and DNS pre-resolve ops.
//!
//! Provides two ops:
//!
//! - `op_prefetch_dns`: Takes a JSON array of hostnames and pre-resolves them
//!   in the background, warming both the in-process DNS cache and the OS
//!   resolver cache.
//!
//! - `op_prefetch_assets`: Takes a JSON array of URLs and fires off background
//!   HTTP GET requests. Responses are stored in an in-process memory cache
//!   keyed by URL. Subsequent `fetch()` calls do NOT automatically hit this
//!   cache (that would add complexity); the main benefit is warming the HTTP
//!   connection pool and the DNS cache.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bytes::Bytes;
use deno_core::OpState;
use deno_core::op2;
use deno_core::serde_json;
use deno_core::url::Url;
use deno_error::JsErrorBox;
use tracing::debug;

use super::dns_cache;
use super::fetch::get_or_create_client_from_state;

/// Maximum number of concurrent prefetch requests.
const MAX_CONCURRENT_PREFETCH: usize = 6;

/// Maximum response body size to cache (1 MB).
const MAX_CACHED_BODY: usize = 1024 * 1024;

/// TTL for cached prefetch responses (5 minutes).
const PREFETCH_CACHE_TTL: Duration = Duration::from_secs(300);

/// Maximum number of cached prefetch entries.
const MAX_CACHE_ENTRIES: usize = 64;

// ---------------------------------------------------------------------------
// Prefetch response cache
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct CachedResponse {
    status: u16,
    body: Bytes,
    expires: Instant,
}

static PREFETCH_CACHE: Mutex<Option<HashMap<String, CachedResponse>>> = Mutex::new(None);

/// Look up a URL in the prefetch cache.
///
/// Returns `Some((status, body))` if the URL was prefetched, the entry is not
/// expired, and the body was small enough to cache.
#[allow(dead_code)]
pub(crate) fn prefetch_cache_lookup(url: &str) -> Option<(u16, Bytes)> {
    let guard = PREFETCH_CACHE.lock().ok()?;
    let cache = guard.as_ref()?;
    let entry = cache.get(url)?;
    if Instant::now() >= entry.expires {
        return None;
    }
    Some((entry.status, entry.body.clone()))
}

fn prefetch_cache_insert(url: String, status: u16, body: Bytes) {
    if let Ok(mut guard) = PREFETCH_CACHE.lock() {
        let cache = guard.get_or_insert_with(HashMap::new);
        // Evict expired entries if at capacity
        if cache.len() >= MAX_CACHE_ENTRIES {
            let now = Instant::now();
            cache.retain(|_, e| now < e.expires);
        }
        if cache.len() >= MAX_CACHE_ENTRIES {
            return;
        }
        cache.insert(
            url,
            CachedResponse {
                status,
                body,
                expires: Instant::now() + PREFETCH_CACHE_TTL,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// op_prefetch_dns
// ---------------------------------------------------------------------------

/// Pre-resolve a list of hostnames so subsequent fetch() calls are faster.
///
/// Accepts a JSON-encoded string array: `["api.example.com", "cdn.example.com"]`
///
/// Resolution happens in background Tokio tasks. This op returns immediately.
/// Invalid or private/loopback addresses are silently skipped.
#[op2(fast)]
pub fn op_prefetch_dns(
    #[string] hosts_json: String,
) -> Result<(), JsErrorBox> {
    let hosts: Vec<String> = serde_json::from_str(&hosts_json)
        .map_err(|e| JsErrorBox::type_error(format!("prefetchDns: invalid JSON: {}", e)))?;

    if hosts.is_empty() {
        return Ok(());
    }

    debug!("prefetchDns: pre-resolving {} hosts", hosts.len());
    dns_cache::pre_resolve(hosts);
    Ok(())
}

// ---------------------------------------------------------------------------
// op_prefetch_assets
// ---------------------------------------------------------------------------

/// Prefetch a list of asset URLs in the background.
///
/// Accepts a JSON-encoded string array: `["https://cdn.example.com/a.png", ...]`
///
/// Each URL is fetched via HTTP GET using the shared connection pool. The
/// primary benefit is warming the TCP/TLS connection pool and DNS cache.
/// Small responses (under 1 MB) are also cached in memory.
///
/// Security: SSRF checks, domain whitelist, and HTTPS enforcement are applied
/// to every URL, same as regular `fetch()`.
#[op2(async(lazy), fast)]
pub async fn op_prefetch_assets(
    state: Rc<RefCell<OpState>>,
    #[string] urls_json: String,
) -> Result<(), JsErrorBox> {
    let urls: Vec<String> = serde_json::from_str(&urls_json)
        .map_err(|e| JsErrorBox::type_error(format!("prefetchAssets: invalid JSON: {}", e)))?;

    if urls.is_empty() {
        return Ok(());
    }

    // Get the shared HTTP/2 client from OpState
    let client = {
        let mut st = state.borrow_mut();
        get_or_create_client_from_state(&mut st, true)
            .map_err(|e| JsErrorBox::generic(format!("prefetchAssets: {}", e)))?
    };

    // Validate all URLs upfront and apply security checks
    let mut valid_urls: Vec<Url> = Vec::new();
    {
        let st = state.borrow();
        for url_str in &urls {
            let url = match Url::parse(url_str) {
                Ok(u) => u,
                Err(_) => continue, // skip invalid URLs
            };
            // Only prefetch http/https
            if url.scheme() != "http" && url.scheme() != "https" {
                continue;
            }
            // Apply SSRF + domain whitelist + HTTPS enforcement
            if super::fetch::reject_blocked_ip_literal(&url).is_err() {
                continue;
            }
            if super::fetch::check_domain_whitelist(&url, &st).is_err() {
                continue;
            }
            if super::fetch::check_https_policy(&url, &st).is_err() {
                continue;
            }
            valid_urls.push(url);
        }
    }

    if valid_urls.is_empty() {
        return Ok(());
    }

    // Limit concurrency
    let batch_size = valid_urls.len().min(MAX_CONCURRENT_PREFETCH);
    let urls_to_fetch = &valid_urls[..batch_size];

    debug!(
        "prefetchAssets: fetching {} URLs (of {} requested)",
        batch_size,
        valid_urls.len()
    );

    // Spawn all prefetches and collect handles
    let mut handles = Vec::with_capacity(batch_size);
    for url in urls_to_fetch {
        let client = client.clone();
        let url = url.clone();
        let handle = tokio::spawn(async move {
            let url_str = url.to_string();
            match client
                .get(url)
                .timeout(Duration::from_secs(30))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    // Only cache small successful responses
                    if status >= 200 && status < 400 {
                        if let Some(len) = resp.content_length() {
                            if len as usize > MAX_CACHED_BODY {
                                // Too large to cache, but connection is warmed
                                debug!(
                                    "prefetchAssets: {} -> {} (too large to cache: {} bytes)",
                                    url_str, status, len
                                );
                                return;
                            }
                        }
                        match resp.bytes().await {
                            Ok(body) if body.len() <= MAX_CACHED_BODY => {
                                debug!(
                                    "prefetchAssets: {} -> {} ({} bytes cached)",
                                    url_str,
                                    status,
                                    body.len()
                                );
                                prefetch_cache_insert(url_str, status, body);
                            }
                            Ok(body) => {
                                debug!(
                                    "prefetchAssets: {} -> {} ({} bytes, too large)",
                                    url_str,
                                    status,
                                    body.len()
                                );
                            }
                            Err(e) => {
                                debug!("prefetchAssets: {} body read error: {}", url_str, e);
                            }
                        }
                    } else {
                        debug!("prefetchAssets: {} -> {} (not cached)", url_str, status);
                    }
                }
                Err(e) => {
                    debug!("prefetchAssets: {} fetch error: {}", url_str, e);
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all prefetch tasks to complete
    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_insert_and_lookup() {
        let url = "https://example.com/asset.png".to_string();
        let body = Bytes::from_static(b"fake png data");
        prefetch_cache_insert(url.clone(), 200, body.clone());
        let (status, cached_body) = prefetch_cache_lookup(&url).unwrap();
        assert_eq!(status, 200);
        assert_eq!(cached_body, body);
    }

    #[test]
    fn cache_miss_returns_none() {
        assert!(prefetch_cache_lookup("https://missing.example.com/x").is_none());
    }
}
