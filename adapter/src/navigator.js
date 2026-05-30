// navigator stub — fields engines probe at startup. Where we can fill from
// migo.getDeviceInfo() / getSystemInfoSync(), we do; the rest fall back to
// browser-default-ish values.

let _info = {};
try { if (typeof migo.getSystemInfoSync === "function") _info = migo.getSystemInfoSync() || {}; } catch {}

const platform = (_info.platform || "ios").toLowerCase();
const system = _info.system || "";
const version = _info.version || "1.0.0";

const navigator = {
  platform,
  language: _info.language || "zh-CN",
  languages: [_info.language || "zh-CN"],
  appVersion: `5.0 (${platform === "android" ? `Linux; ${system}` : `iPhone; ${system}`})`,
  userAgent: `Mozilla/5.0 (${platform === "android" ? "Linux; Android" : "iPhone; iOS"}; ${system}) AppleWebKit/605.1.15 (KHTML, like Gecko) MiniGame/${version} migo`,
  vendor: "",
  product: "Gecko",
  productSub: "20030107",
  cookieEnabled: false,
  onLine: true,
  hardwareConcurrency: 4,
  maxTouchPoints: 5,

  // Stubs for APIs the runtime doesn't bridge yet.
  geolocation: {
    getCurrentPosition: () => {},
    watchPosition: () => 0,
    clearWatch: () => {},
  },
};

export default navigator;
