// Surface probe: make the runtime report what it publishes to content.
//
// Loaded as ordinary mini-game content, so what it sees is exactly what a game
// sees -- after the snapshot, after namespace construction, after hardening.
// That is the point: parsing `99_global_scope*.js` would report what the
// sources intend, and the two have already disagreed once (`Deno` was mirrored
// onto a published namespace despite hardening deleting it from globalThis).
function names(o) {
  try {
    return Object.getOwnPropertyNames(o).sort();
  } catch (_) {
    return null;
  }
}

// `wx` is reported only when something installed it before this ran. The engine
// never publishes it -- the wx namespace is an adapter's job (it mirrors migo.*
// minus a small exclusion set), so `dump.wx === null` is the normal, correct
// answer for a bare runtime and means "no adapter was loaded", not "the adapter
// is broken". scripts/dump-api-surface.sh --adapter is what puts one in front.
const dump = {
  global: names(globalThis),
  migo: (typeof migo === "object" && migo) ? names(migo) : null,
  wx: (typeof globalThis.wx === "object" && globalThis.wx) ? names(globalThis.wx) : null,
};

// Single line, marker-prefixed, so the driver can extract it from mixed logs.
console.error("__MIGO_SURFACE__" + JSON.stringify(dump));
