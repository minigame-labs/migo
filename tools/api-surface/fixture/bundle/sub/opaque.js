const key = "createBannerAd";
wx[key]();                           // computed -- unresolvable
for (const k in wx) { void k; }      // reflection
