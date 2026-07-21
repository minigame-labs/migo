//! Ad APIs - compatible mock implementation.
//!
//! Provides mock implementations of banner, interstitial, rewarded video,
//! custom, and grid ad components. All ad content is simulated in JavaScript
//! with no native ops required.

use deno_core::Extension;

deno_core::extension!(
    host_v8_ad,
    deps = [host_v8_base],
    esm_entry_point = "ext:host_v8_ad/99_global_scope.js",
    esm = [
        dir "src/ad",
        "01_ad.js",
        "99_global_scope.js",
    ],
);

pub fn ad_extensions() -> Vec<Extension> {
    vec![host_v8_ad::init()]
}
