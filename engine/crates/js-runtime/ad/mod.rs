//! Ad APIs - WeChat compatible mock implementation.
//!
//! Provides mock implementations of banner, interstitial, rewarded video,
//! custom, and grid ad components. All ad content is simulated in JavaScript
//! with no native ops required.

use deno_core::Extension;

deno_core::extension!(
    host_v8_ad,
    deps = [host_v8_base],
    esm = [
        dir "ad",
        "01_ad.js",
    ],
);

pub fn ad_extensions() -> Vec<Extension> {
    vec![host_v8_ad::init()]
}
