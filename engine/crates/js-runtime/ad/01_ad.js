// Ad APIs - compatible mock implementation
// All ad types simulate successful load/show flow with proper event callbacks.

import { createListenerGroup } from "ext:host_v8_base/02_async.js";

const MOCK_LOAD_DELAY_MS = 100;
const MOCK_REWARDED_VIDEO_DURATION_MS = 500;

const _directAdStatus = {
  isInMask: false,
  isInDirectGameAd: false,
};

const _directAdStatusListeners = createListenerGroup("onDirectAdStatusChange");

function _notifyDirectAdStatus(status) {
  _directAdStatusListeners.trigger(status);
}

function onDirectAdStatusChange(listener) {
  if (typeof listener !== "function") return;
  _directAdStatusListeners.on(listener);
  queueMicrotask(() => {
    try {
      listener(getDirectAdStatusSync());
    } catch (e) {
      console.error("onDirectAdStatusChange listener error:", e);
    }
  });
}

function offDirectAdStatusChange(listener) {
  _directAdStatusListeners.off(listener);
}

function _internalTriggerDirectAdStatusChange(status) {
  if (!status || typeof status !== "object") return;
  if (status.isInMask !== undefined) {
    _directAdStatus.isInMask = !!status.isInMask;
  }
  if (status.isInDirectGameAd !== undefined) {
    _directAdStatus.isInDirectGameAd = !!status.isInDirectGameAd;
  }
  _notifyDirectAdStatus(getDirectAdStatusSync());
}

// ==================== AdBase (shared listener pattern) ====================

class AdBase {
  #listeners = {};
  #destroyed = false;

  constructor(eventTypes) {
    for (const type of eventTypes) {
      this.#listeners[type] = createListenerGroup(`Ad ${type}`);
    }
  }

  _on(type, listener) {
    if (this.#destroyed) return;
    const group = this.#listeners[type];
    if (!group) return;
    group.on(listener);
  }

  _off(type, listener) {
    const group = this.#listeners[type];
    if (!group) return;
    group.off(listener);
  }

  _fire(type, arg) {
    const group = this.#listeners[type];
    if (!group) return;
    group.trigger(arg);
  }

  _isDestroyed() {
    return this.#destroyed;
  }

  _markDestroyed() {
    this.#destroyed = true;
    for (const type in this.#listeners) {
      this.#listeners[type].off();
    }
  }

  _scheduleLoad() {
    setTimeout(() => {
      if (!this.#destroyed) {
        this._fire("load");
      }
    }, MOCK_LOAD_DELAY_MS);
  }
}

// ==================== BannerAd ====================

class BannerAd extends AdBase {
  #style;
  #adUnitId;
  #adIntervals;
  #refreshTimer = null;
  #visible = false;

  constructor({ adUnitId, adIntervals, style }) {
    super(["load", "error", "resize"]);
    this.#adUnitId = adUnitId;
    this.#adIntervals = adIntervals;
    const ratio = 0.35;
    const w = Math.max(300, style.width || 300);
    const h = Math.round(w * ratio);
    this.#style = {
      left: style.left || 0,
      top: style.top || 0,
      width: w,
      height: style.height || h,
      realWidth: w,
      realHeight: h,
    };
    this._scheduleLoad();
  }

  get style() {
    return this.#style;
  }

  show() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createBannerAd:fail already destroyed" });
    }
    this.#visible = true;
    this._fire("resize", {
      width: this.#style.realWidth,
      height: this.#style.realHeight,
    });
    if (this.#adIntervals && this.#adIntervals >= 30 && !this.#refreshTimer) {
      this.#refreshTimer = setInterval(() => {
        if (!this._isDestroyed() && this.#visible) {
          this._scheduleLoad();
        }
      }, this.#adIntervals * 1000);
    }
    return Promise.resolve();
  }

  hide() {
    this.#visible = false;
  }

  destroy() {
    if (this.#refreshTimer) {
      clearInterval(this.#refreshTimer);
      this.#refreshTimer = null;
    }
    this.#visible = false;
    this._markDestroyed();
  }

  onResize(listener) { this._on("resize", listener); }
  offResize(listener) { this._off("resize", listener); }
  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
}

// ==================== CustomAd ====================

class CustomAd extends AdBase {
  #style;
  #adUnitId;
  #adIntervals;
  #refreshTimer = null;
  #visible = false;

  constructor({ adUnitId, adIntervals, style }) {
    super(["load", "error", "close", "hide", "resize"]);
    this.#adUnitId = adUnitId;
    this.#adIntervals = adIntervals;
    this.#style = {
      left: style.left || 0,
      top: style.top || 0,
      fixed: style.fixed || false,
    };
    this._scheduleLoad();
  }

  get style() {
    return this.#style;
  }

  show() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createCustomAd:fail already destroyed" });
    }
    this.#visible = true;
    if (this.#adIntervals && this.#adIntervals >= 30 && !this.#refreshTimer) {
      this.#refreshTimer = setInterval(() => {
        if (!this._isDestroyed() && this.#visible) {
          this._scheduleLoad();
        }
      }, this.#adIntervals * 1000);
    }
    return Promise.resolve();
  }

  hide() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createCustomAd:fail already destroyed" });
    }
    if (this.#visible) {
      this.#visible = false;
      this._fire("hide");
    }
    return Promise.resolve();
  }

  isShow() {
    return this.#visible;
  }

  destroy() {
    if (this.#refreshTimer) {
      clearInterval(this.#refreshTimer);
      this.#refreshTimer = null;
    }
    this.#visible = false;
    this._markDestroyed();
  }

  onClose(listener) { this._on("close", listener); }
  offClose(listener) { this._off("close", listener); }
  onHide(listener) { this._on("hide", listener); }
  offHide(listener) { this._off("hide", listener); }
  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onResize(listener) { this._on("resize", listener); }
  offResize(listener) { this._off("resize", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
}

// ==================== GridAd ====================

class GridAd extends AdBase {
  #style;
  #adUnitId;
  #adIntervals;
  #adTheme;
  #gridCount;
  #refreshTimer = null;
  #visible = false;

  constructor({ adUnitId, adIntervals, style, adTheme, gridCount }) {
    super(["load", "error", "resize"]);
    this.#adUnitId = adUnitId;
    this.#adIntervals = adIntervals;
    this.#adTheme = adTheme || "white";
    this.#gridCount = gridCount || 5;
    const w = Math.max(300, style.width || 300);
    const h = style.height || w;
    this.#style = {
      left: style.left || 0,
      top: style.top || 0,
      width: w,
      height: h,
      realWidth: w,
      realHeight: h,
    };
    this._scheduleLoad();
  }

  get style() {
    return this.#style;
  }

  get adTheme() {
    return this.#adTheme;
  }

  get gridCount() {
    return this.#gridCount;
  }

  show() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createGridAd:fail already destroyed" });
    }
    this.#visible = true;
    this._fire("resize", {
      width: this.#style.realWidth,
      height: this.#style.realHeight,
    });
    if (this.#adIntervals && this.#adIntervals >= 30 && !this.#refreshTimer) {
      this.#refreshTimer = setInterval(() => {
        if (!this._isDestroyed() && this.#visible) {
          this._scheduleLoad();
        }
      }, this.#adIntervals * 1000);
    }
    return Promise.resolve();
  }

  hide() {
    this.#visible = false;
  }

  destroy() {
    if (this.#refreshTimer) {
      clearInterval(this.#refreshTimer);
      this.#refreshTimer = null;
    }
    this.#visible = false;
    this._markDestroyed();
  }

  onResize(listener) { this._on("resize", listener); }
  offResize(listener) { this._off("resize", listener); }
  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
}

// ==================== InterstitialAd ====================

class InterstitialAd extends AdBase {
  #adUnitId;
  #loaded = false;

  constructor({ adUnitId }) {
    super(["load", "error", "close"]);
    this.#adUnitId = adUnitId;
    this._scheduleLoad();
  }

  load() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createInterstitialAd:fail already destroyed" });
    }
    return new Promise((resolve) => {
      setTimeout(() => {
        if (!this._isDestroyed()) {
          this.#loaded = true;
          this._fire("load");
        }
        resolve();
      }, MOCK_LOAD_DELAY_MS);
    });
  }

  show() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createInterstitialAd:fail already destroyed" });
    }
    this.#loaded = false;
    // Mock: auto-close after a short delay
    setTimeout(() => {
      if (!this._isDestroyed()) {
        this._fire("close");
      }
    }, MOCK_LOAD_DELAY_MS);
    return Promise.resolve();
  }

  destroy() {
    this._markDestroyed();
  }

  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
  onClose(listener) { this._on("close", listener); }
  offClose(listener) { this._off("close", listener); }
}

// ==================== RewardedVideoAd ====================

const _rewardedVideoSingletons = new Map();

class RewardedVideoAd extends AdBase {
  #adUnitId;
  #loaded = false;

  constructor({ adUnitId }) {
    super(["load", "error", "close"]);
    this.#adUnitId = adUnitId;
    this._scheduleLoad();
  }

  load() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createRewardedVideoAd:fail already destroyed" });
    }
    return new Promise((resolve) => {
      setTimeout(() => {
        if (!this._isDestroyed()) {
          this.#loaded = true;
          this._fire("load", { useFallbackSharePage: false });
        }
        resolve();
      }, MOCK_LOAD_DELAY_MS);
    });
  }

  show() {
    if (this._isDestroyed()) {
      return Promise.reject({ errMsg: "createRewardedVideoAd:fail already destroyed" });
    }
    this.#loaded = false;
    // Mock: simulate video watched to completion, then fire close
    setTimeout(() => {
      if (!this._isDestroyed()) {
        this._fire("close", { isEnded: true });
      }
    }, MOCK_REWARDED_VIDEO_DURATION_MS);
    return Promise.resolve();
  }

  destroy() {
    _rewardedVideoSingletons.delete(this.#adUnitId);
    this._markDestroyed();
  }

  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
  onClose(listener) { this._on("close", listener); }
  offClose(listener) { this._off("close", listener); }
}

// ==================== Factory Functions ====================

function createBannerAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createBannerAd:fail missing adUnitId" };
  }
  if (!obj.style) {
    throw { errMsg: "createBannerAd:fail missing style" };
  }
  return new BannerAd(obj);
}

function createCustomAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createCustomAd:fail missing adUnitId" };
  }
  if (!obj.style) {
    throw { errMsg: "createCustomAd:fail missing style" };
  }
  return new CustomAd(obj);
}

function createGridAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createGridAd:fail missing adUnitId" };
  }
  if (!obj.style) {
    throw { errMsg: "createGridAd:fail missing style" };
  }
  return new GridAd(obj);
}

function createInterstitialAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createInterstitialAd:fail missing adUnitId" };
  }
  return new InterstitialAd(obj);
}

function createRewardedVideoAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createRewardedVideoAd:fail missing adUnitId" };
  }
  // Singleton by adUnitId unless multiton is enabled
  if (!obj.multiton) {
    const existing = _rewardedVideoSingletons.get(obj.adUnitId);
    if (existing && !existing._isDestroyed()) {
      return existing;
    }
  }
  const ad = new RewardedVideoAd(obj);
  if (!obj.multiton) {
    _rewardedVideoSingletons.set(obj.adUnitId, ad);
  }
  return ad;
}

function getDirectAdStatusSync() {
  return {
    isInMask: _directAdStatus.isInMask,
    isInDirectGameAd: _directAdStatus.isInDirectGameAd,
  };
}

function getShowSplashAdStatus(obj = {}) {
  const res = {
    status: "success",
    code: 1,
    errMsg: "getShowSplashAdStatus:ok",
  };
  queueMicrotask(() => {
    try {
      if (typeof obj.success === "function") {
        obj.success(res);
      }
    } catch (e) {
      console.error("getShowSplashAdStatus success callback error:", e);
    }
    try {
      if (typeof obj.complete === "function") {
        obj.complete(res);
      }
    } catch (e) {
      console.error("getShowSplashAdStatus complete callback error:", e);
    }
  });
}

// ==================== GameBanner (createGameBanner) ====================
// Same shape as BannerAd but uses the "game recommend" semantics.

class GameBanner extends AdBase {
  #style;
  #adUnitId;
  #visible = false;

  constructor({ adUnitId, style }) {
    super(["load", "error", "resize"]);
    this.#adUnitId = adUnitId;
    const s = style || {};
    const w = Math.max(300, s.width || 300);
    const h = s.height || Math.round(w * 0.35);
    this.#style = {
      left: s.left || 0,
      top: s.top || 0,
      width: w,
      height: h,
      realWidth: w,
      realHeight: h,
    };
    this._scheduleLoad();
  }

  get style() { return this.#style; }

  show() {
    if (this._isDestroyed()) return Promise.reject({ errMsg: "createGameBanner:fail already destroyed" });
    this.#visible = true;
    this._fire("resize", { width: this.#style.realWidth, height: this.#style.realHeight });
    return Promise.resolve();
  }

  hide() { this.#visible = false; }

  destroy() {
    this.#visible = false;
    this._markDestroyed();
  }

  onResize(listener) { this._on("resize", listener); }
  offResize(listener) { this._off("resize", listener); }
  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
}

function createGameBanner(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createGameBanner:fail missing adUnitId" };
  }
  return new GameBanner(obj);
}

// ==================== GameIcon (createGameIcon) ====================

class GameIcon extends AdBase {
  #style;
  #adUnitId;
  #count;
  #visible = false;

  constructor({ adUnitId, count, style }) {
    super(["load", "error", "resize"]);
    this.#adUnitId = adUnitId;
    this.#count = count || 1;
    const s = style || {};
    this.#style = {
      left: s.left || 0,
      top: s.top || 0,
      width: s.width || 40,
      height: s.height || 40,
    };
    this._scheduleLoad();
  }

  get style() { return this.#style; }
  get count() { return this.#count; }

  show() {
    if (this._isDestroyed()) return Promise.reject({ errMsg: "createGameIcon:fail already destroyed" });
    this.#visible = true;
    this._fire("resize", { width: this.#style.width, height: this.#style.height });
    return Promise.resolve();
  }

  hide() { this.#visible = false; }

  destroy() {
    this.#visible = false;
    this._markDestroyed();
  }

  onResize(listener) { this._on("resize", listener); }
  offResize(listener) { this._off("resize", listener); }
  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
}

function createGameIcon(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createGameIcon:fail missing adUnitId" };
  }
  return new GameIcon(obj);
}

// ==================== GamePortal (createGamePortal) ====================
// Similar to InterstitialAd: load -> show -> close.

class GamePortal extends AdBase {
  #adUnitId;

  constructor({ adUnitId }) {
    super(["load", "error", "close"]);
    this.#adUnitId = adUnitId;
    this._scheduleLoad();
  }

  load() {
    if (this._isDestroyed()) return Promise.reject({ errMsg: "createGamePortal:fail already destroyed" });
    return new Promise((resolve) => {
      setTimeout(() => {
        if (!this._isDestroyed()) {
          this._fire("load");
        }
        resolve();
      }, MOCK_LOAD_DELAY_MS);
    });
  }

  show() {
    if (this._isDestroyed()) return Promise.reject({ errMsg: "createGamePortal:fail already destroyed" });
    setTimeout(() => {
      if (!this._isDestroyed()) {
        this._fire("close");
      }
    }, MOCK_LOAD_DELAY_MS);
    return Promise.resolve();
  }

  destroy() { this._markDestroyed(); }

  onLoad(listener) { this._on("load", listener); }
  offLoad(listener) { this._off("load", listener); }
  onError(listener) { this._on("error", listener); }
  offError(listener) { this._off("error", listener); }
  onClose(listener) { this._on("close", listener); }
  offClose(listener) { this._off("close", listener); }
}

function createGamePortal(obj) {
  if (!obj || !obj.adUnitId) {
    throw { errMsg: "createGamePortal:fail missing adUnitId" };
  }
  return new GamePortal(obj);
}

export {
  createBannerAd,
  createCustomAd,
  createGridAd,
  createInterstitialAd,
  createRewardedVideoAd,
  getDirectAdStatusSync,
  onDirectAdStatusChange,
  offDirectAdStatusChange,
  _internalTriggerDirectAdStatusChange,
  getShowSplashAdStatus,
  createGameBanner,
  createGameIcon,
  createGamePortal,
};
