// localStorage — Web Storage API on top of migo.getStorageSync etc.

const localStorage = {
  get length() {
    try { return migo.getStorageInfoSync().keys.length; } catch { return 0; }
  },
  key(i) {
    try { return migo.getStorageInfoSync().keys[i] || null; } catch { return null; }
  },
  getItem(k) {
    try {
      const v = migo.getStorageSync(k);
      // The host returns "" for missing keys. localStorage spec says null. Honor that.
      return v === "" ? null : v;
    } catch { return null; }
  },
  setItem(k, v) {
    try { migo.setStorageSync(k, String(v)); } catch {}
  },
  removeItem(k) {
    try { migo.removeStorageSync(k); } catch {}
  },
  clear() {
    try { migo.clearStorageSync(); } catch {}
  },
};

export default localStorage;
