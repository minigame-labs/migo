# V8 Startup Snapshot — Operations & Release Guide

The Migo runtime embeds a **V8 startup snapshot**: the serialized V8 heap after
all extension JS has been parsed, compiled and executed. Loading from it skips
that work, cutting cold-start `HostJsRuntime::new` from ~75 ms to ~14–24 ms.

A snapshot is **platform-bound (OS + CPU arch)**: it serializes a live V8 heap,
so it must be produced by the *same* `android-<arch>` V8 that the `.so` links —
a host-linux V8 (or the wrong CPU arch) yields a snapshot that fails V8's
magic-number check on load and **hard-crashes** (no graceful fallback). We
therefore generate it the "[Deno #27496][deno]" way: cross-compile
`migo-snapshot-gen` to the target ABI and run it on that ABI's emulator/device.

[deno]: https://github.com/denoland/deno/issues/27496

---

## 1. When to regenerate

The snapshot bakes: the post-execution V8 heap of all extension JS, the op
external-reference table (its *size* drives the V8 magic number), and deno_core
sidecar data (module map, op count, extension names).

| Change | If you DON'T regenerate |
|---|---|
| **Extension JS** (any `*.js` under `crates/js-runtime`: `99_main.js`, `97_wx_namespace.js`, a feature's ESM, …) | ⚠️ **Silent staleness** — with a snapshot the extension JS is *not* re-loaded from source, so the runtime runs the OLD baked code. |
| **Op changes** (add/remove/rename op, change count) | 💥 **Hard crash** at load: external-ref-table size changes → V8 magic mismatch / deno_core op-count mismatch. |
| **Extension set/order** (`api-*` feature flags, add/remove an extension) | 💥 Op set / extension list mismatch → crash. |
| **deno_core / V8 version bump** | 💥 builtins + external refs change → magic mismatch. |
| **Rebuilt android V8 archive** with different GN flags | 💥 May change the external-ref table → magic. |

**Not triggered by:** game JS (loaded at runtime as modules, fully independent),
Rust logic that doesn't touch ops/extension-JS, rendering/graphics changes.

> Rule of thumb = the `paths:` filter in `.github/workflows/build-snapshot.yml`.
> When in doubt, **delete the local snapshot** — the from-source fallback is
> always correct (just slower). The only danger is keeping a *mismatched* one.

The staleness gate below detects the two common dangerous cases automatically.

---

## 2. How to regenerate

Requires: the android V8 archive (`git lfs pull`), Android NDK
(`ANDROID_NDK_HOME`), `cargo-ndk`, and a connected emulator/device.

```bash
# x86_64 — on an x86_64 emulator (or via CI, see §4)
scripts/gen-snapshot.sh x86_64 --device emulator-5554

# arm64 — on a REAL arm64 device (hosted CI has no arm64 KVM)
scripts/gen-snapshot.sh arm64  --device <device-serial>
```

`gen-snapshot.sh` cross-compiles `migo-snapshot-gen` to the ABI (arm64 also
links `libclang_rt.builtins-aarch64-android` for `__clear_cache`, which x86_64
does not need), pushes it + the matching `libc++_shared.so` to the device, runs
it, and pulls the result + a manifest into `snapshots/` (§3).

---

## 3. Where snapshots live

```
engine/crates/js-runtime/snapshots/
  SNAPSHOT-aarch64.bin                 # arm64 snapshot      (Git LFS)
  SNAPSHOT-aarch64.bin.manifest.json   # freshness fingerprint (plain JSON)
  SNAPSHOT-x86_64.bin                  # x86_64 snapshot     (Git LFS)
  SNAPSHOT-x86_64.bin.manifest.json
```

`js-runtime/build.rs` embeds `SNAPSHOT-<CARGO_CFG_TARGET_ARCH>.bin` **only for
android targets** (host builds always use the from-source path). A single
`build-aar.sh release arm64-v8a x86_64` thus embeds each `.so`'s own ABI
snapshot — verified: no cross-contamination.

The `.bin` files are committed via **Git LFS** (same as the V8 archives); the
tiny `*.manifest.json` are committed as plain text.

---

## 4. Release best practice (the gap, solved)

**Problem:** snapshots are generated artifacts, not produced by every build, and
a stale/mismatched one is dangerous (§1). How does a release guarantee
`snapshots/` is present, fresh, and per-arch correct?

**Design — commit + fingerprint-gate (chosen):**

1. **Commit per-arch snapshots via Git LFS.** The repo already commits the
   124 MB V8 archives via LFS; the 1.9 MB snapshots fit the same model. This
   makes release builds **hermetic, reproducible, and offline** (no
   emulator/device at release time) and gives the arm64 snapshot — which hosted
   CI *cannot* generate (no arm64 KVM) — a home.

2. **Fingerprint manifest per snapshot.** `gen-snapshot.sh` writes
   `SNAPSHOT-<arch>.bin.manifest.json` capturing the *platform-independent*
   inputs that determine validity: `js_sources_sha256` (all extension JS) and
   `deno_core_version`. Because these are platform-independent, freshness can be
   checked **on any host, with no device**.

3. **CI freshness gate.** `scripts/check-snapshot-freshness.sh` recomputes the
   fingerprint from the current tree and compares it to each committed manifest;
   a mismatch **fails the build** with a "regenerate" message. Wired into
   `build-snapshot.yml` (host job), so a PR that changes extension JS or bumps
   deno_core without refreshing the snapshots is blocked.

4. **Regeneration workflow** (when the gate goes red):
   - `build-snapshot.yml` auto-generates the **x86_64** snapshot on an emulator
     and uploads it as an artifact — download it into `snapshots/`.
   - Regenerate **arm64** on a real device: `scripts/gen-snapshot.sh arm64 …`.
   - Commit both `SNAPSHOT-*.bin` (LFS) + `*.manifest.json` → gate goes green.

**Residual gaps** (v1 fingerprint does not auto-detect; rely on the on-device
smoke test, which crashes loudly on magic mismatch): a pure op *rename* keeping
the same op count with no JS change; rebuilding the android V8 archive with
different GN flags. Both are rare and deliberate.

> Local `build-aar.sh` does not hard-gate (to keep JS iteration fast). Run
> `scripts/check-snapshot-freshness.sh` before a local release build, or just
> regenerate after touching extension JS.

---

## 5. CI

`.github/workflows/build-snapshot.yml`:

- **`freshness`** (host, `ubuntu-latest`): runs `check-snapshot-freshness.sh`
  against the committed snapshots — the release gate.
- **`snapshot-x86_64`** (`ubuntu-latest`, has `/dev/kvm`): cross-compiles the
  generator, boots an x86_64 emulator via `reactivecircus/android-emulator-runner`,
  runs it, uploads `snapshot-x86_64`.
- **arm64**: not on hosted runners (no arm64 KVM). Generate on a real device
  (§2); a self-hosted arm64 runner with an attached device could automate it.
