// Asset prefetch and DNS pre-resolve APIs.
//
// These APIs allow games to warm the HTTP connection pool and DNS cache
// before assets are actually needed, reducing perceived load times.

import { op_prefetch_dns, op_prefetch_assets } from "ext:core/ops";

/**
 * Pre-resolve a list of hostnames to warm the DNS cache.
 *
 * @param {Object} options
 * @param {string[]} options.hostnames - Array of hostnames to pre-resolve.
 */
function prefetchDns(options) {
    if (!options || !options.hostnames) return;
    var hosts = options.hostnames;
    if (!Array.isArray(hosts) || hosts.length === 0) return;
    // Filter to valid string entries
    var valid = [];
    for (var i = 0; i < hosts.length; i++) {
        if (typeof hosts[i] === 'string' && hosts[i].length > 0) {
            valid.push(hosts[i]);
        }
    }
    if (valid.length === 0) return;
    try {
        op_prefetch_dns(JSON.stringify(valid));
    } catch (_) {
        // Pre-resolve is best-effort, do not propagate errors
    }
}

/**
 * Prefetch a list of asset URLs in the background.
 *
 * Fires background HTTP GET requests to warm the connection pool and
 * optionally cache small responses. Returns a Promise that resolves
 * when all prefetch requests have completed (or failed).
 *
 * @param {Object} options
 * @param {string[]} options.urls - Array of URLs to prefetch.
 * @returns {Promise<void>}
 */
function prefetchAssets(options) {
    if (!options || !options.urls) return Promise.resolve();
    var urls = options.urls;
    if (!Array.isArray(urls) || urls.length === 0) return Promise.resolve();
    // Filter to valid string entries
    var valid = [];
    for (var i = 0; i < urls.length; i++) {
        if (typeof urls[i] === 'string' && urls[i].length > 0) {
            valid.push(urls[i]);
        }
    }
    if (valid.length === 0) return Promise.resolve();
    return op_prefetch_assets(JSON.stringify(valid)).catch(function () {
        // Prefetch is best-effort, swallow errors
    });
}

export { prefetchDns, prefetchAssets };
