// Three forms that look like API references and are not. Each one was found on
// a real customer bundle, and each one made the scanner report a gap that did
// not exist -- on one 31-file bundle, four reported gaps were really one.

// 1. Namespace-shaped text inside a string. A storage key and an Android
//    package name, not calls.
const CACHE_KEY = "wx.cn.minigame.iap";
const PACKAGE = { packageName: "com.wx.minigame" };

// 2. The bundle installing its own shim. An assignment is the opposite of a
//    gap: the runtime was never asked for it.
wx.__loadSubpackage__ = wx.loadSubpackage;

// 3. A binding to a *result*, not to the namespace. `const c = migo.createCanvas()`
//    binds a canvas; treating `c` as an alias for `migo` turns every
//    `c.getContext` into a missing `migo.getContext`.
const c = migo.createCanvas();
const ctx = c.getContext("2d");
void CACHE_KEY, PACKAGE, ctx;
