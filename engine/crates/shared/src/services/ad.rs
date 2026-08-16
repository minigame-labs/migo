//! Ad service traits for host-authoritative advertising.
//!
//! # Why this trait exists
//!
//! Ad SDKs (Pangle, GDT, Kuaishou Union, ...) are integrated by the *host*
//! application, not by the runtime. The runtime never links an ad SDK, never
//! holds ad account credentials, and never decides whether a reward was
//! earned. It only carries requests out to the host and carries the host's
//! events back to content.
//!
//! # The reward-integrity invariant
//!
//! For incentivised video, the single field that matters commercially is
//! `isEnded` on the `close` event: it is what content uses to decide whether
//! to grant the player a reward. **That field must originate from the host's
//! ad SDK callback and must never be synthesised by the runtime or by content
//! JavaScript.** A runtime that lets content fabricate `isEnded: true` hands
//! out rewards for adverts that were never watched -- the publisher pays the
//! reward and earns nothing.
//!
//! The invariant is enforced structurally rather than by convention:
//!
//! - Content JS has no way to reach the listener groups except through events
//!   delivered on the inbound host bridge (`_internalOnAdEvent`).
//! - When no host ad service is installed, the runtime's fallback path reports
//!   `isEnded: false`. No advert was shown, so no reward is owed. There is
//!   therefore no code path in the runtime that produces `isEnded: true`
//!   without a host event.
//! - `scripts/test-ad-reward-integrity-contract.sh` fails the build if the
//!   embedded JS ever regains the ability to originate a truthy `isEnded`.
//!
//! # Shape
//!
//! Ads are long-lived objects with event *streams*, unlike the one-shot
//! request/response services (auth, payment). So the outbound direction is a
//! set of commands addressed to an `adId` allocated by the content side, and
//! the inbound direction is a single event channel. Every method here is
//! fire-and-forget; results arrive as events.
//!
//! All request payloads are JSON objects that carry at least `adId`
//! (a positive integer allocated by the runtime's JS layer).

use crate::protocol::error::ServiceError;

/// Host-provided advertising service.
///
/// Every method defaults to `not_supported`, so a platform that has no ad
/// integration simply does not override anything, and content falls back to
/// the runtime's no-reward path.
pub trait AdService: Send + Sync {
    /// Create a host-side ad object.
    ///
    /// JSON fields (input):
    /// - `adId`: number, runtime-allocated handle used by all later commands
    /// - `adType`: string, one of `banner`, `custom`, `grid`, `interstitial`,
    ///   `rewardedVideo`, `gameBanner`, `gameIcon`, `gamePortal`
    /// - `adUnitId`: string, the publisher's ad slot id
    /// - `options`: object, the remaining platform-style creation options
    ///   (`adIntervals`, `style`, `adTheme`, `gridCount`, `count`, `multiton`)
    ///
    /// Events delivered via the ad event channel.
    fn create_ad(&self, _request_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("createAd:fail not supported"))
    }

    /// Request that the host load (or reload) ad content for `adId`.
    ///
    /// JSON fields (input): `adId`.
    ///
    /// Success is reported as a `load` event; failure as an `error` event.
    fn load_ad(&self, _request_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("loadAd:fail not supported"))
    }

    /// Request that the host present `adId`.
    ///
    /// JSON fields (input): `adId`.
    ///
    /// For interstitial / rewarded-video / portal ads the host reports a
    /// `close` event when the advert is dismissed. For rewarded video the
    /// `close` payload carries `isEnded`, which **must** reflect the ad SDK's
    /// own completion verdict.
    fn show_ad(&self, _request_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("showAd:fail not supported"))
    }

    /// Request that the host hide `adId` without destroying it.
    ///
    /// JSON fields (input): `adId`.
    fn hide_ad(&self, _request_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("hideAd:fail not supported"))
    }

    /// Update the layout of a positioned ad (banner / custom / grid / icon).
    ///
    /// JSON fields (input): `adId`, `style` (object with `left`, `top`,
    /// `width`, `height` in CSS pixels).
    fn update_ad_style(&self, _request_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "updateAdStyle:fail not supported",
        ))
    }

    /// Release the host-side ad object for `adId`.
    ///
    /// JSON fields (input): `adId`.
    ///
    /// The host must emit no further events for `adId` after this returns.
    fn destroy_ad(&self, _request_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("destroyAd:fail not supported"))
    }
}
