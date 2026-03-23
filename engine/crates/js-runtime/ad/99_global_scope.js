// Global scope registration for host_v8_ad APIs (api-system feature gate).

import * as adApi from 'ext:host_v8_ad/01_ad.js';

import { primordials, core } from "ext:core/mod.js";
const { ObjectDefineProperties } = primordials;

ObjectDefineProperties(globalThis, {
    // Ad
    createBannerAd: core.propNonEnumerable(adApi.createBannerAd),
    createCustomAd: core.propNonEnumerable(adApi.createCustomAd),
    createGridAd: core.propNonEnumerable(adApi.createGridAd),
    createInterstitialAd: core.propNonEnumerable(adApi.createInterstitialAd),
    createRewardedVideoAd: core.propNonEnumerable(adApi.createRewardedVideoAd),
    getDirectAdStatusSync: core.propNonEnumerable(adApi.getDirectAdStatusSync),
    getShowSplashAdStatus: core.propNonEnumerable(adApi.getShowSplashAdStatus),
    createGameBanner: core.propNonEnumerable(adApi.createGameBanner),
    createGameIcon: core.propNonEnumerable(adApi.createGameIcon),
    createGamePortal: core.propNonEnumerable(adApi.createGamePortal),
});
