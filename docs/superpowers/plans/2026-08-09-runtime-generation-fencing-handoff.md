# Runtime Generation Fencing — Handoff, 2026-08-09

**Bootstrap on another machine:** read
`docs/superpowers/plans/2026-08-08-four-platform-delivery-handoff.md` §0 first.
It lists the git-ignored prerequisites (Android V8 archives, host linux-gnu V8,
`MIGO_HOST_V8_DIR`) without which every host suite fails for a reason unrelated
to your change. Nothing in that section has changed.

**Branch:** `delivery/x11-and-mutation-evidence`, unpushed at `2102594`.
Verify before believing: `git log --oneline 79dc831..HEAD`.

**Ledger:** `docs/superpowers/plans/2026-08-03-four-platform-delivery/part-phase-0.md`.
Item 0.9's task-7 entries carry every decision below with its reasoning. This
document is the short form plus the things not yet written there.

---

## 1. What landed today

Seven commits, all on top of `79dc831`:

| commit | what |
|---|---|
| `105605f` | `RuntimeGenerationBoundary` (Java) + `RuntimeGenerationNotifier` (engine); the soft keyboard is the first fenced Android producer |
| `841e149` | `RuntimeScoped` + `liveEntry`: a manager whose runtime was replaced is destroyed and rebuilt, not handed over |
| `486f320` | the restart completion is authoritative, so a lost `begin` notification cannot wedge a session forever |
| `f3f435d` | per-session toast/loading overlay slots, released at teardown |
| `6419b00` | the concurrent-session audit, ranked, as item **0.68** |
| `bf9bba5` | the fence field became `Option<NonZeroI64>`; sensors + screenshot observer fenced; restart releases what it fences |
| `0822d46` | the network monitor joins the restart sweep, and the real sweep rule is stated |
| `2102594` | camera, microphone and video fenced; their hardware is released at restart |

---

## 2. The two rules that are easy to get wrong

**A fence and a cache sweep are one change, not two.** Managers are cached per
*session*, not per *runtime*. Fencing a producer without routing its cache
lookups through `RuntimeGenerationBoundary.liveEntry` converts "events reach the
wrong runtime" into "events reach nothing at all" — the feature comes up and
reports nothing, which reads as never having been wired. This happened once
already; see `841e149`.

**What decides whether a group may be swept at restart is its own teardown.**
Destroying a manager can *report* — the keyboard emits `onKeyboardComplete`, a
camera emits `stop`. Those land on the queue while `on_restart` is still running
on the engine thread, so they are dispatched to the runtime that *replaces* this
one, as if it had produced them. A group qualifies if its teardown reports
nothing (`NetworkMonitor`) **or** if what it reports is fenced (input, sensors,
media). Sweeping an unfenced group that reports injects exactly the cross-talk
the fence exists to remove.

**`HostCommand` sits on its own 64-byte cap.** Measured: reverting one media
variant to `Option<i64>` fails the build with `HostCommand grew past 64 bytes`.
Any new fenced variant uses `Option<NonZeroI64>` and `captured_generation`.

---

## 3. State of task 7's Android half

| group | fenced | cache swept | released at restart |
|---|---|---|---|
| keyboard, scan code (`InputExports`) | keyboard yes | yes | yes |
| sensors, screenshot observer (`SensorExports`) | yes | yes | yes |
| network (`NetworkExports`) | **no, by design** | n/a | yes |
| camera, recorder, video (`MediaExports`) | yes | yes | yes |
| overlays (`InteractionUI`) | n/a | per-session slots | yes |
| **bluetooth (`BluetoothExports`)** | **no** | **no** | **no** |

Network and `OnDeviceOrientationChange` are deliberately unfenced: a network
status and a screen rotation are *current facts about the device*, so the
replacement isolate needs them as much as the retired one did. Dropping them
would leave a fresh runtime believing the phone is online and portrait with
nothing to correct it. Do not "finish the job" by fencing them.

---

## 4. Bluetooth — everything already established, so it is not re-derived

I researched this group and then chose not to do it; the reasoning and the
findings are both here.

**Why it was left:** lowest value of the six groups (BLE is rare in mini-games),
highest cost, and its notification path is the one this repository already tuned
to zero allocations (item 0.67), so touching it risks a gate someone worked hard
for.

**What the work is, concretely:**

* Seven variants, all comfortably under the size cap:
  `OnBluetoothAdapterStateChange{bool,bool}`, `OnBluetoothDeviceFound{String}`,
  `OnBLEConnectionStateChange{String,bool}`,
  `OnBLECharacteristicValueChange(Recycled<BleCharacteristicData>)` — a tuple
  variant, so it becomes a struct variant or the generation goes beside the
  pooled handle — `OnBLEMTUChange{String,u32}`, `OnBeaconUpdate{String}`,
  `OnBeaconServiceChange{bool,bool}`.
* Seven JNI handlers in `inbound.rs`, seven descriptors in
  `profile_contract.rs`, seven `NativeBridge` + `NativeMethods` pairs.
* A `jlong` parameter allocates nothing, and `token.generation()` is a primitive
  field read, so the zero-allocation gate should stay green — **but re-run
  `BluetoothNotificationAllocationTest` and do not assume it.**

**The one thing Bluetooth uniquely offers.** `BluetoothManager` is the *only*
producer that is device-free constructible: its package-private constructors take
reporter lambdas instead of an `Activity`. Every other manager needs a `Context`
or a main `Looper`, which is why three ledger entries say "the wiring is
untested, named rather than implied". If its `GattEventReporter` /
`ConnectionStateReporter` interfaces carry the generation, a host unit test can
finally assert the whole property end to end:

> restart the session behind a live manager's back, then fire a notification, and
> the manager must still report the **retired** generation — not the current one.

That is the property the entire design rests on, and nothing tests it today at a
real producer.

**The cost that made me stop:** 29 `new BluetoothManager(...)` sites across three
test files, each with a distinct literal session id, all of which need a
registered session once the constructor calls `acquire`. A `@BeforeClass` helper
in the test tree that registers a range (tolerating an already-registered id) is
the cheap way; do not add production API for it.

---

## 5. Recommended next piece, and it is not Bluetooth

**A structural gate over the fenced call sites.** It covers every producer,
including the five no test can construct, which is strictly more than the one
test Bluetooth would buy. Design:

1. Derive the fenced set from the authority rather than a hand-written list:
   parse `engine/crates/platform/src/android/jni/profile_contract.rs` for every
   `NATIVE_*` descriptor matching `^\(IJ` — that is exactly "this callback
   carries a generation".
2. For each such method, find every call to `NativeMethods.<name>(` under
   `platforms/android/library/src/main/java/` and assert the second argument is
   `token.generation()` or `RuntimeGenerationBoundary.UNFENCED` — never a
   literal, never a re-read of the current generation, which is the failure that
   always matches and proves nothing.
3. Assert `NativeMethods.<name>` forwards its own `generation` parameter to
   `NativeBridge.<name>` rather than dropping it.
4. **Assert the derived set is non-empty.** An empty scan and a clean scan are
   indistinguishable; this repository has shipped that mistake before.
5. Normalise whitespace inside the call's parentheses before matching — several
   call sites wrap the argument list across lines.
6. Wire it into `.github/workflows/pr-ci.yml`. A gate that is not in a workflow
   does not exist, and `scripts/verify-change.sh` derives its contract lane from
   that file (`scripts/lib/ci_contract_gates.py`), so adding it there is what
   makes it run locally too.

After that, item **0.68** (concurrent sessions) is the ranked list, worst first:
`GamePaths.cleanupTemp` keyed by `gameId` deletes a same-`gameId` session's live
temp directory; three JNI exports resolve an Activity through
`RuntimeRegistry.getAny()` because their signatures have no `sessionId`; the log
level is one process-wide switch on both sides.

---

## 6. Verifying, on a machine that is not this one

**Iterate narrowly.** Java-only changes: `:library:testFullDebugUnitTest
:library:testSlimDebugUnitTest`. Rust-only: the affected crates' `--lib`, both
profiles. Anything touching `cfg(target_os = "android")` code **must** run
`bash scripts/build-android-so.sh --compile-only arm64-v8a` — the host lane
cannot see that code, and this has cost several sessions.

**The full gate is for handoffs**, not for every change:
`bash scripts/verify-change.sh --base <ref>`.

**Windows is machine-specific and will read as NOT PROVEN on your machine, which
is honest.** On the machine this was written on, the MSVC toolchain is present
and a real compile is two commands:

```bash
bash platforms/windows/spike/sync-worktree.sh
bash platforms/windows/spike/probe-layer.sh migo-platform   # then migo-capi
```

It refuses a dirty tree (`require_synced_worktree`, exit 91/92) because the
Windows copy is cloned over `file://` and carries committed refs only — which is
also why it cannot be a `verify-change.sh` lane, whose scope is HEAD plus the
working tree. If you cannot run it, say NOT PROVEN; do not infer it from the
Linux build.

**Mutation harness rules, learned the hard way today.** A mutation that fails to
apply must abort loudly: the script ran the suite against the *unmutated* file,
it passed, and that was printed as `SURVIVED`. And a mutant "killed" by a syntax
error was killed by nothing — every `killed` line must name the test that failed,
or state honestly that the type system caught it (which is legitimate: the
`Option<NonZeroI64>` niche assertion has no other guard).
