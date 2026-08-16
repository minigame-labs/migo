# V8 Startup Snapshot — Operations & Release Guide

The Migo host runtime can embed a **V8 startup snapshot**: the serialized V8 heap after
all extension JS has been parsed, compiled and executed. Loading from it skips
that work. A separate, default-off Worker snapshot candidate exists alongside the
host one; its latency, memory, package-size, and power tradeoff requires device
A/B evidence before it becomes a shipping default.

A snapshot is **platform-bound (OS + CPU arch)**: it serializes a live V8 heap,
so it must be produced by the *same* `<os>-<arch>` V8 the `.so`/`.a` links —
the wrong OS or the wrong CPU arch yields a snapshot that fails V8's
magic-number check on load and **hard-crashes** (no graceful fallback). We
therefore generate it the "[Deno #27496][deno]" way: build `migo-snapshot-gen`
for the target platform (natively for Linux, cross-compiled and run on an
emulator/device for Android) and run it there.

[deno]: https://github.com/denoland/deno/issues/27496

---

## 1. When to regenerate

The snapshot bakes: the post-execution V8 heap of all extension JS, the op
external-reference table (its *size* drives the V8 magic number), and deno_core
sidecar data (module map, op count, extension names).

| Change | If you DON'T regenerate |
|---|---|
| **Extension JS** (any `*.js` under `crates/runtime-v8`: `99_main.js`, `97_migo_namespace.js`, a feature's ESM, …) | ⚠️ **Silent staleness** — with a snapshot the extension JS is *not* re-loaded from source, so the runtime runs the OLD baked code. |
| **Op/runtime/generator changes** (`runtime-v8` or `snapshot-gen` Rust/Cargo inputs) | 💥 May change the external-ref table, extension assembly, or snapshot options. Schema v3 rejects all such changes conservatively. |
| **Snapshot kind substitution** (host bytes renamed as Worker, or the reverse) | 💥 Different extension/op table and bootstrap heap. Schema v3 binds `snapshot_kind` and rejects substitution. |
| **Extension set/order** (`api-*` feature flags, add/remove an extension) | 💥 Op set / extension list mismatch → crash. |
| **deno_core / V8 version bump** | 💥 builtins + external refs change → magic mismatch. |
| **Rebuilt V8 archive** with different GN flags, for any platform | 💥 May change the external-ref table → magic. |

**Not triggered by:** game JS (loaded at runtime as modules, fully independent),
Rust logic that doesn't touch ops/extension-JS, rendering/graphics changes.

> Rule of thumb = the `paths:` filter in `.github/workflows/build-snapshot.yml`.
> When in doubt, **delete the local snapshot** — the from-source fallback is
> always correct (just slower). The only danger is keeping a *mismatched* one.

The staleness gate below detects the two common dangerous cases automatically.

---

## 2. How to regenerate

Requires the target platform's V8 archive (`bash scripts/fetch-v8-archives.sh`)
and, for Android, the NDK (`ANDROID_NDK_HOME`), `cargo-ndk`, and a connected
emulator/device. Linux runs natively — no device involved.

```bash
# Linux — native, no device
scripts/gen-snapshot.sh x86_64 --os linux --snapshot-kind host --product-profile full
scripts/gen-snapshot.sh x86_64 --os linux --snapshot-kind worker --product-profile full

# Android — on a connected emulator/device (--os android is the default)
scripts/gen-snapshot.sh x86_64 --snapshot-kind host --product-profile full --device emulator-5554
scripts/gen-snapshot.sh x86_64 --snapshot-kind host --product-profile slim --device emulator-5554

# arm64 — on a REAL arm64 device (hosted CI has no arm64 KVM)
scripts/gen-snapshot.sh arm64 --snapshot-kind host --product-profile full --device <device-serial>
scripts/gen-snapshot.sh arm64 --snapshot-kind host --product-profile slim --device <device-serial>

# Worker candidates exist only for the full product.
scripts/gen-snapshot.sh x86_64 --snapshot-kind worker --product-profile full --device emulator-5554
scripts/gen-snapshot.sh arm64 --snapshot-kind worker --product-profile full --device <device-serial>
```

`--os ohos` and `--os windows` are not implemented yet: OpenHarmony needs an
hdc-reachable device/emulator bridge (see `scripts/gen-snapshot.sh`'s own
header for the current state of that gap) and Windows has no snapshot support
at all today — both platforms load extension JS from source unconditionally.

On Android, `gen-snapshot.sh` cross-compiles `migo-snapshot-gen` to the ABI
(arm64 also links `libclang_rt.builtins-aarch64-android` for `__clear_cache`,
which x86_64 does not need), pushes it + the matching `libc++_shared.so` to the
device, runs it, and pulls the result + a manifest into `snapshots/` (§3). On
Linux it builds and runs `migo-snapshot-gen` in place.

---

## 3. Where snapshots live

```
engine/crates/runtime-v8/snapshots/
  SNAPSHOT-full-android-aarch64.bin
  SNAPSHOT-full-android-x86_64.bin
  SNAPSHOT-full-linux-x86_64.bin
  SNAPSHOT-slim-android-aarch64.bin
  SNAPSHOT-slim-android-x86_64.bin
  SNAPSHOT-worker-full-android-aarch64.bin
  SNAPSHOT-worker-full-android-x86_64.bin
  SNAPSHOT-worker-full-linux-x86_64.bin
  SNAPSHOT-<profile>-<os>-<arch>.bin.manifest.json
  SNAPSHOT-worker-<profile>-<os>-<arch>.bin.manifest.json
```

The OS segment exists because `android-x86_64` and `linux-x86_64` are two
different V8 builds that would otherwise collide on the same filename. There
is no `linux-slim` snapshot: the `slim` product profile only ships on Android.

`runtime-v8/build.rs` embeds `SNAPSHOT-<Cargo profile>-<os>-<arch>.bin` for
every `(os, arch)` combination it recognizes via `v8_target_dir` — Android,
Linux (`gnu`), OpenHarmony and Windows (MSVC) are all dispatched the same way,
not just Android. In practice OpenHarmony and Windows never find a matching
file today (§2), so they always take the source-JS fallback; any missing or
mismatched host identity falls back the same way. Worker source bootstrap
remains the shipping default. An explicit `build-aar.sh --product-profile full
--worker-snapshot release` candidate independently requires
`SNAPSHOT-worker-full-<os>-<arch>.bin`; it fails compilation when the exact
schema-v3 artifact does not validate.

The `.bin` files are committed as **ordinary git blobs**, not Git LFS — at
1.4–2.1 MB each they're well under the point where LFS pays for itself (see
`.gitattributes`). The tiny `*.manifest.json` are committed as plain text
alongside them.

---

## 4. Release best practice (the gap, solved)

**Problem:** snapshots are generated artifacts, not produced by every build, and
a stale/mismatched one is dangerous (§1). How does a release guarantee
`snapshots/` is present, fresh, and per-arch correct?

**Design — commit + fingerprint-gate (chosen):**

1. **Commit per-profile, per-OS, per-ABI snapshots directly.** This makes release
   builds **hermetic, reproducible, and offline** (no emulator/device at release
   time) and gives the arm64 snapshot — which hosted CI *cannot* generate (no
   arm64 KVM) — a home.

2. **Schema-v3 identity manifest per snapshot.** It records runtime kind, product profile,
   canonical Cargo features, all extension JS, `runtime-v8` + `snapshot-gen`
   Rust/Cargo inputs, Cargo.lock, `deno_core`, the exact V8 archive, OS,
   architecture, and materialized snapshot bytes/size. Build-time validation
   rejects malformed, stale, or unresolved inputs and falls back to source JS.

3. **CI freshness gate.** `scripts/check-snapshot-freshness.sh` recomputes the
   fingerprint from the current tree and compares it to each profile manifest;
   a mismatch **fails the build** with a "regenerate" message. Wired into both
   `build-snapshot.yml` (runs on every snapshot-relevant push) and `release.yml`
   (blocks a tag from shipping a stale snapshot), so changing extension JS or
   bumping deno_core without refreshing the snapshots is caught. An entirely
   absent kind/profile/OS set remains optional; once either Android ABI is
   present, the gate requires both `aarch64` and `x86_64` for that OS.

4. **Regeneration workflow** (when the gate goes red):
   - **Linux x86_64:** run `scripts/gen-snapshot.sh x86_64 --os linux ...` locally
     (§2) and commit the result — there is no automated Linux regen job.
   - **Android x86_64:** trigger `build-snapshot.yml` manually (Actions → *Build V8
     Snapshot* → **Run workflow**) and select `host|worker` plus the product.
     Worker+slim is rejected. It regenerates on an emulator, writes the
     kind/profile-qualified manifest, and opens an isolated regeneration branch
     (skips if the bytes are unchanged). The slow generate runs only manually.
   - **Android arm64:** regenerate both products on a real device with
     `--product-profile full` and `--product-profile slim`, then commit both
     profile-qualified files and manifests.
   - All fresh → gate goes green → safe to merge / tag.

The gate deliberately rejects after any Rust runtime refactor, even when the
snapshot might remain compatible. This false-positive cost is preferable to
accepting an op table or V8 archive mismatch.

> Local `build-aar.sh` does not hard-gate (to keep JS iteration fast). Run
> `scripts/check-snapshot-freshness.sh` before a local release build, or just
> regenerate after touching extension JS.

---

## 5. CI

`.github/workflows/build-snapshot.yml`:

- **`freshness`** (host, `ubuntu-latest`): checks every present host full/slim
  and Worker full identity against the committed snapshots on each relevant
  push, including two-ABI completeness once an Android set exists. This is the
  only job that runs on push.
- **`snapshot (host|worker / x86_64 / emulator)`** (`ubuntu-latest`, has
  `/dev/kvm`) — **`workflow_dispatch` only**, so the slow (~15 min) generate
  never runs on a push. Cross-compiles the generator for Android x86_64, boots
  an emulator via `reactivecircus/android-emulator-runner`, runs it, writes
  the manifest, and opens a `bot/regen-x86_64-snapshot` PR (skips the PR when
  the regenerated bytes are unchanged).
- **Linux and arm64**: not automated anywhere. Linux runs natively on any dev
  machine (§2); Android arm64 needs a real device (no arm64 KVM on hosted
  runners) — a self-hosted arm64 runner with an attached device could
  automate it.
