// Bundle the ESM adapter into a single IIFE file that the migo runtime can
// inject as a boot prelude script (no module loader needed at runtime).
//
// The adapter is idempotent (guarded by `globalThis.__migoAdapterInjected`),
// so the bundle is safe to evaluate even if the game also `import`s the
// ESM entry point afterwards.

import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");

await build({
  entryPoints: [resolve(root, "src/index.js")],
  bundle: true,
  format: "iife",
  // Plain script: no module loader, runs as a top-level eval against the
  // host realm. The IIFE wraps the entire adapter so its internals stay
  // isolated; only the `globalThis.*` writes leak out, which is exactly
  // what we want.
  platform: "neutral",
  target: ["es2020"],
  outfile: resolve(root, "dist/migo-adapter.bundle.js"),
  sourcemap: false,
  legalComments: "none",
  // Banner makes the output self-describing if a developer looks at it.
  banner: {
    js: "/* @minigame-labs/migo-adapter — IIFE bundle. Source: adapter/src/index.js */",
  },
});

console.log("built dist/migo-adapter.bundle.js");
