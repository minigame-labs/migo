// Ad APIs - WeChat compatible mock implementation
// All ad types simulate successful load/show flow with proper event callbacks.

const MOCK_LOAD_DELAY_MS = 100;
const MOCK_REWARDED_VIDEO_DURATION_MS = 500;

// ==================== AdBase (shared listener pattern) ====================

class AdBase {
  #listeners = {};
  #destroyed = false;

  constructor(eventTypes) {
    for (const type of eventTypes) {
      this.#listeners[type] = [];
    }
  }

  _on(type, listener) {
    if (this.#destroyed) return;
    if (typeof listener === "function") {
      this.#listeners[type].push(listener);
    }
  }

  _off(type, listener) {
    if (!this.#listeners[type]) return;
    if (typeof listener === "function") {
      const idx = this.#listeners[type].indexOf(listener);
      if (idx !== -1) {
        this.#listeners[type].splice(idx, 1);
      }
    } else {
      this.#listeners[type].length = 0;
    }
  }

  _fire(type, arg) {
    const list = this.#listeners[type];
    if (!list || list.length === 0) return;
    const snapshot = list.slice();
    for (const fn of snapshot) {
      try {
        fn(arg);
      } catch (e) {
        console.error(`Ad ${type} listener error:`, e);
      }
    }
  }

  _isDestroyed() {
    return this.#destroyed;
  }

  _markDestroyed() {
    this.#destroyed = true;
    for (const type in this.#listeners) {
      this.#listeners[type].length = 0;
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
      return Promise.reject(new Error("BannerAd already destroyed"));
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
      return Promise.reject(new Error("CustomAd already destroyed"));
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
      return Promise.reject(new Error("CustomAd already destroyed"));
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
      return Promise.reject(new Error("GridAd already destroyed"));
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
      return Promise.reject(new Error("InterstitialAd already destroyed"));
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
      return Promise.reject(new Error("InterstitialAd already destroyed"));
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
      return Promise.reject(new Error("RewardedVideoAd already destroyed"));
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
      return Promise.reject(new Error("RewardedVideoAd already destroyed"));
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
    throw new Error("createBannerAd:fail missing adUnitId");
  }
  if (!obj.style) {
    throw new Error("createBannerAd:fail missing style");
  }
  return new BannerAd(obj);
}

function createCustomAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw new Error("createCustomAd:fail missing adUnitId");
  }
  if (!obj.style) {
    throw new Error("createCustomAd:fail missing style");
  }
  return new CustomAd(obj);
}

function createGridAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw new Error("createGridAd:fail missing adUnitId");
  }
  if (!obj.style) {
    throw new Error("createGridAd:fail missing style");
  }
  return new GridAd(obj);
}

function createInterstitialAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw new Error("createInterstitialAd:fail missing adUnitId");
  }
  return new InterstitialAd(obj);
}

function createRewardedVideoAd(obj) {
  if (!obj || !obj.adUnitId) {
    throw new Error("createRewardedVideoAd:fail missing adUnitId");
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
    isInMask: false,
    isInDirectGameAd: false,
  };
}

function getShowSplashAdStatus(obj = {}) {
  const res = {
    status: "success",
    code: 1,
    errMsg: "getShowSplashAdStatus:ok",
  };
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
}

export {
  createBannerAd,
  createCustomAd,
  createGridAd,
  createInterstitialAd,
  createRewardedVideoAd,
  getDirectAdStatusSync,
  getShowSplashAdStatus,
};
