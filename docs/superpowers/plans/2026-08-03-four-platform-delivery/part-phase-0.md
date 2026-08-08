> Part of the [Four-Platform Delivery Ledger](../2026-08-03-four-platform-delivery.md).

## Phase 0 — Correctness Foundation

- [ ] 0.1 Close the BLE permission-path locking debt. **Implementation landed,
  spec review NOT approved, item stays open.** Cancellation now runs outside the
  permission-session monitor (`6c00295`) and session lookup is lock-free with a
  dedicated guard for the monotonic open invariant (`8841ef7`). Verified at 89
  tests per profile with no failures, errors, or skips; permission coverage
  contract passes at 30 gated, 8 cleanup, and 38 sensitive operations; Rust suites
  pass; `git diff --check` clean. Both mutations were killed: restoring the
  monitor around cancellation fails
  `closeCancellationDoesNotRetainTheSessionMonitor`, and neutralising `awaitIdle`
  fails all three drain fixtures.
  The independent spec review confirmed no lost drain, no timeout escape, no
  polling, no reopenable tombstone, and no false-success path in the three entry
  points, and raised four findings now tracked as tasks 0.22 through 0.25 plus the
  contention-test correction recorded under task 5.1. Specification Sections 6.1
  and 7.3 may **not** be recorded as closed by `8841ef7`.
  **The independent code-quality review is now done.** Both of this item's commits
  (`6c00295`, `8841ef7`) were reviewed as part of the five-commit correctness batch
  isolated in its own worktree, and no finding was raised against either; the two
  findings that round produced were against the text texture cache (task 0.16) and
  the permission gate's release-profile test (task 0.18), both since fixed.

  Remaining for this item: task 5.1's contention test, which the spec review
  withdrew as provably unable to fail, plus the reviews of the sub-findings tracked
  as 0.23 through 0.25. Task 0.22 is complete. Section 7.3's contention gate now
  exists for the Rust per-event paths (task 0.27), but the permission gate's half is
  JVM-side and a Rust probe observes nothing about a Java monitor — so this item still
  cannot be closed by its own tests alone.
  Detailed plan: `docs/superpowers/plans/2026-08-03-p0-1-ble-permission-debt.md`.
  Prior state: `docs/superpowers/plans/2026-08-03-p1-ble-admission-status.md`.
- [ ] 0.2 A1: own and join every Host thread; make Engine destruction the final
  lifecycle barrier. Plan: `2026-07-29-a1-owned-host-lifecycle.md`.

  **Implementation and behavioural tests are already in the tree; this item is
  stale bookkeeping, not open work.** Audited 2026-08-08 against `06e358a`. The
  plan's four unchecked implementation steps each carry an "Expected: FAIL
  because…" sentence, and all four premises are false:

  | Plan says missing | Actually at |
  |---|---|
  | "no owning handle or join API exists" | `core/src/runtime/thread.rs:41` `HostThread`, with `join`, `shutdown_and_join`, `request_shutdown`, self-join rejection and a fail-safe `Drop` |
  | "Engine has no retirement set and Session stores only an integer Host ID" | `capi/src/lib.rs:118` `retired_hosts: Mutex<Vec<HostThread>>`; `retire_host`/`take_retired_hosts`; `:502-512` drains before joining, so no engine lock is held across a join |
  | "JNI discards `HostThread` and stores only the returned ID" | `platform/src/android/jni/inbound.rs:475` `host_owners().insert(host)`; `:839` `shutdown_with(id, HostThread::shutdown_and_join)` |
  | "the player discards the `JoinHandle`" | `tools/player/src/main.rs:260` `shutdown_before_drop(host, window)`, which joins before the native window drops |

  Named tests already asserting the properties: `thread.rs`
  `join_waits_for_named_host_and_observes_sentinel_drop`,
  `failed_start_joins_the_spawned_thread`,
  `self_join_rejection_preserves_owner_for_another_thread`; `capi/src/lib.rs`
  `engine_refuses_to_die_while_sessions_are_live`,
  `engine_destroy_waits_for_retired_host_sentinel`,
  `session_destroy_transfers_host_without_joining_it`,
  `engine_destroy_from_its_retired_host_is_rejected_and_retryable`;
  `platform/src/host_owners.rs` `terminal_take_transfers_ownership_exactly_once`,
  `failed_shutdown_restores_same_owner_for_retry`; `tools/player/src/main.rs`
  `teardown_joins_host_before_dropping_native_resource`.

  **The mutant this entry asked for was taken, 2026-08-09, and the guard it named
  did not fail. It could not.** `engine_destroy_holds_no_engine_lock_while_joining`
  used to hold a Migo lock deliberately across the join and still pass: **50 runs
  out of 50**. A positive control rules out a stale binary — flipping
  `join_failed` two lines above the same edit failed the test immediately with
  `left: -11, right: 0`, so the mutated function was in the binary both times.

  The reason is not a race to be tightened. The probe sampled
  `retired_hosts.try_lock()` once, from the very thread being joined, and that
  sample has **no ordering** against the join: it ran before the destroying thread
  had been scheduled at all. A second attempt — spin until the retirement set is
  observed drained *and* unlocked — failed the same way for a sharper reason:
  `take_retired_hosts` releases the lock before the mutant re-acquires it, so the
  state the probe waits for genuinely occurs, and the probe reliably catches that
  window. **No sampling probe can establish "this thread is inside a blocking
  call and holding nothing"**; that is a property of the code, not of an instant.

  **So the bad state was made unexpressible instead.** `capi/src/retirement.rs`
  holds `RetirementSet`: the `Mutex` is a private field of a private type in its
  own module, `take` is the only way out and yields owned handles, and the one
  function that produces a `MutexGuard` is private to that module. The engine's
  destruction path can no longer name a lock to hold. `EngineInner::retire_host`
  and `take_retired_hosts` are gone; their callers use the set directly, so there
  is one place that knows how retirement is stored.

  This is a **deadlock**, not a hygiene rule: a Host on its way out reaches
  `migo_session_destroy` and `migo_engine_destroy`, which take these locks, so a
  joiner holding one waits for the thread that is waiting for it.

  **Evidence, and it is compile-time — deliberately labelled, because a mutant the
  compiler kills is not a test kill.** Both shapes the defect could take now fail
  to build, at `crates/capi/src/lib.rs:496`:

  | Mutant | Result |
  |---|---|
  | `engine_ref.inner.retired_hosts.lock()` held across the join loop | `error[E0599]: no method named 'lock' found for struct 'RetirementSet'` |
  | the same through the accessor, `.locked()` | `error[E0624]: method 'locked' is private` |

  What replaces the deleted probe as a *runnable* guard is
  `retirement.rs::take_hands_over_every_host_and_leaves_the_set_empty` and
  `take_refuses_from_a_retired_host_and_keeps_it_for_a_retry`. Nothing else was
  lost with it: its other two assertions — destruction waits for its Host, and
  returns `MIGO_OK` once the Host exits — are `engine_destroy_waits_for_retired_host_sentinel`'s,
  and the self-join refusal is `engine_destroy_from_its_retired_host_is_rejected_and_retryable`'s.

  `cargo test -p migo-capi --lib`: **147 passed**, up from 146 — one unfalsifiable
  test removed, two falsifiable ones added. `cargo-mutants --file
  crates/capi/src/retirement.rs`: **10 mutants, 2 caught, 0 missed, 8 unviable** —
  the eight are the tool trying to fabricate a `MutexGuard` for the private
  accessor, which is the encapsulation showing up as "cannot even be written".

  **Still not closed:** neither independent review has run.
- [ ] 0.3 A2: immutable `PlatformIdentity` with synchronous rejection of
  incompatible reattachment, including the new HarmonyOS identity row.
  Plan: `2026-07-29-a2-platform-identity.md`.

  **Also implemented; also stale bookkeeping.** Audited 2026-08-08. The plan's
  "Add failing platform-pair tests" step is unchecked, but those tests landed in
  `c6645bd` alongside the step below it:
  `platform_identity_distinguishes_linux_domain_and_display`,
  `platform_identity_rejects_mixed_linux_provider_and_factory`,
  `provider_and_factory_share_backend_id` in `platform/src/linux/presenter.rs`, with
  a `platform_identity_is_stable_*` case in each of the Android, Windows and ohos
  presenters.

  **The synchronous-rejection property holds, checked by reading the order rather
  than trusting the note.** In `capi/src/surface.rs::migo_session_attach_surface`:
  `validate_platform_identity` at `:285` → `lease_surface_tracked` at `:320` →
  `HostCommand::UpdateSurface` at `:334`, and the failure path runs
  `rollback_surface_transition` and returns rather than falling through.
  `scripts/test-surface-attachment-contract.sh:158-160` asserts
  `identity_check_line < first_lease_line` statically, and that gate is in the local
  contract lane as of task T.6.

  One claim checked separately because "the resize path quietly carries a new native
  handle" is exactly this ledger's recurring shape: `migo_surface_update` at `:520`
  also sends `UpdateSurface`, at `:607`, with **no** identity check. It is sound —
  its signature is `(attachment, metrics)` and `MigoSurfaceMetrics` carries only
  size, scale and generation, so it cannot change identity, and it is additionally
  gated on `ptr::eq(active, attachment_ref)` plus `validate_update_generation`.

  **Mutant taken 2026-08-09, and it is the cleanest discriminating case in this
  ledger.** `validate_platform_identity` was moved from before
  `lease_surface_tracked` to immediately after it — a reordering that still
  rejects the incompatible surface, just one step too late.

  | Scope | Under the mutant |
  |---|---|
  | `scripts/test-surface-attachment-contract.sh` | **FAIL**: "C ABI reattachment identity is not rejected before Surface lease/enqueue" |
  | `cargo test -p migo-capi --lib` | 147 passed |
  | `cargo test -p migo-shared --lib surface::attachment` | 20 passed |

  The static contract is the **only** thing that sees it, which is what an
  ordering property looks like: no unit test can observe "before" — both orders
  return the same code to the same caller. It is also the concrete case for T.6,
  since until that item this gate ran in CI and not locally. Restored from a copy
  taken before mutating and verified by `sha256sum`.

  **Not closed:** neither independent review has run.
- [ ] 0.4 A3: Migo-owned X11 connection; remove the undocumented `XInitThreads`
  precondition. Plan: `2026-07-29-a3-owned-x11-connection.md`.

  Audited 2026-08-08. `XInitThreads` is absent from the tree,
  `scripts/test-x11-owned-connection-contract.sh` passes, and the connection layer
  is covered by `platform/src/linux/x11_connection.rs`
  `owner_opens_private_connection_and_closes_it_before_api_drop`,
  `server_mismatch_closes_candidate_and_returns_error`,
  `reuse_requires_a_live_connection_to_the_same_server`, plus
  `presenter.rs::x11_context_binds_identity_surface_and_factory_to_one_owned_connection`.

  **The gap is closed at the C ABI, 2026-08-09.** The plan's Task 3 Step 1 asked for
  platform-context tests at the `capi` layer; `capi/src/platform/linux.rs` had
  Wayland cases only, so nothing drove `build_target` down the X11 arm and the
  reattachment properties were asserted at the connection layer and nowhere at the
  boundary that decides them. Three tests now drive it:
  `x11_reattachment_to_the_same_server_reuses_the_one_owned_connection`,
  `x11_window_from_a_foreign_server_is_refused_and_opens_nothing`, and
  `a_stored_x11_context_refuses_a_wayland_descriptor`.

  **The seam this entry named did not exist from `capi`, which is the fifteenth
  recorded obstacle of that shape.** `LinuxX11Context::from_render_display_for_test`
  is real, but it was `#[cfg(test)] fn` — private, and `cfg(test)` does not cross a
  crate boundary, so `capi` compiles `platform` without it. Asking *which layer can
  see this property* rather than *how do I reach this code* gives the answer: the
  decision is `capi`'s, the evidence is the connection's, and a test seam has to
  cross between them. It now does, the way `migo-core/test-support` already does —
  `migo-platform/test-support`, enabled only from `capi`'s `[dev-dependencies]`,
  which resolver 2 keeps out of every shipped build.

  `X11TestServers` (`platform/src/linux/x11_connection.rs`) declares which
  `Display*` reaches which X server and opens the render connection through
  production `open_with_api` — no Xlib, no socket, no server. It **replaced**
  `from_display_for_test`/`NoopX11Api` rather than joining them: two ways to
  fabricate one object is the "one rule, two implementations" shape this repository
  keeps digging out. The eight presenter tests that used the old helper (eleven call
  sites) are unchanged and now run through the new one.

  **Mutation evidence.** Restores verified by `sha256sum` against copies taken
  before mutating, not by `git checkout` — the work was uncommitted.

  | Mutant | Killed by | Old scope under the same mutant |
  |---|---|---|
  | Delete the `supports_host_display` check in the X11 reuse arm | `x11_window_from_a_foreign_server_is_refused_and_opens_nothing`, `left: None` vs `right: Some(-5)` | **All 10 `migo-platform` X11 tests pass**, including `reuse_requires_a_live_connection_to_the_same_server`, which asserts the deleted property one layer down. That is the gap, made concrete. |
  | Wayland arm: `Some(_) => Err` becomes a fallthrough | `a_stored_x11_context_refuses_a_wayland_descriptor` **and** the pre-existing `platform_context_rejects_display_or_kind_change_before_native_access` | Old scope **also** kills it, so this mutant does not justify the new test. Recorded because it is the discriminating question. |
  | Wayland arm guards only `Some(Wayland{..})`, letting an installed X11 context through | `a_stored_x11_context_refuses_a_wayland_descriptor` alone | `platform_context_rejects_display_or_kind_change_before_native_access` **passes** — it starts from a Wayland context, so it never exercises the kind it does not install. The §"guards cover the side they were designed for" shape, and what earns the third test. |

  **What is still device-blocked, stated rather than papered over.** The *cold* arm
  (`existing: None`) calls `LinuxX11Context::open`, which loads Xlib and
  dereferences the host `Display*`; no seam changes that, so "a cold attach opens
  exactly one context" is asserted at the connection layer
  (`owner_opens_private_connection_and_closes_it_before_api_drop`) and, at the C
  ABI, only for a context the test itself opened. Task 2 Step 4's
  `native_owned_connection_round_trip` is `#[ignore]`d pending a live X server, and
  Task 4 Step 4 runs the real C host under `xvfb-run`. Both need an X server this
  machine does not have.

  **Verification, 2026-08-09** — `bash scripts/verify-change.sh --base master`,
  covering this item together with 0.2, 0.3, T.6 and T.8:

  ```
  VERIFIED SCOPE  master..HEAD plus the working tree (15 files)
  PASS  host      19 steps
  PASS  contract  23 gates (1 CI ONLY: test-local-verification-contract.sh)
  PASS  android compile  bash scripts/build-android-so.sh --compile-only arm64-v8a
  PASS  ohos compile     bash scripts/build-ohos-sdk.sh --compile-only x86_64
  [verify] verified for every target this change touches
  ```

  The `ohos compile` line is evidence rather than a static argument for the first
  time; see T.8 for why it used to read `NOT PROVEN`.
- [x] 0.5 A4: lossless terminal input transitions under bounded saturation.
  Plan: `2026-07-29-a4-lossless-input-state.md`. Evidence recorded 2026-07-29.
- [ ] 0.6 A5: correct desktop pointer and button semantics, including Qt hover
  and pressed-button state. **One shipped defect found and fixed; neither
  independent review has run, so the item stays open.**

  **Hover reported a button nobody was holding.** `include/migo/input.h:150-151`
  defines the field: "button follows DOM MouseEvent.button… On a move it names the
  button being held." `MigoQtX11SurfaceView` answered that from `held_button_`, a
  cached ordinal written on press and on release and **never cleared**. So after any
  right-click, every subsequent hover move reported `button = 2`, and content saw a
  secondary-button drag that was not happening. The same field got the two-button
  chord wrong too: press primary, press secondary, release secondary, and moves
  reported the released button while the primary was still down.

  The field's own doc comment is where this is visible in hindsight — it read "the
  mouse button currently held, tracked so a focus loss can retract the press", which
  is the rationale for `mouse_pressed_`, the boolean beside it. One comment covered
  two fields and the second inherited a reason that never applied to it.

  **Fixed by deletion.** `QMouseEvent` already carries both halves — `button()` is
  what changed state, `buttons()` is what is still down — so `deliverPointer` asks
  the event: `button()` for DOWN and UP, `dom_button_held(buttons())` for MOVE, zero
  when nothing is held. `held_button_` is gone, so the stale state is
  unrepresentable rather than reset, and the file is shorter than before. Lowest
  ordinal wins when several are down, because the ABI's field is one button while
  Qt's is a mask.

  Two new tests, both pressing the **secondary** button on purpose: the primary's
  ordinal is 0, which is also what a hover must report, so a test that presses the
  primary passes whether the view reads the event or a stale field.
  `a_move_after_a_release_names_no_held_button` walks press → drag → release → hover;
  `releasing_one_of_two_buttons_leaves_the_other_named` covers the chord.

  Mutation evidence, two mutants killing the same test at **different assertion
  lines**, which is what shows both of its claims carry weight:

  - **M-A5-1** restores the cached ordinal exactly as it was. **20 passed, 2 failed** —
    both new tests die at `pointers[3].button: 2, expected 0` (lines 307 and 332),
    while the two pre-existing pointer tests
    (`a_press_drag_and_release_reach_both_streams_in_css_pixels`,
    `motion_without_a_button_reaches_the_mouse_stream_but_not_touch`) **pass**. That
    pair passing is the proof the defect was unguarded rather than newly introduced.
  - **M-A5-2** a move asks `event.button()` instead of `event.buttons()` — the
    wrong-accessor slip that produces this bug class. **21 passed, 1 failed**, dying at
    `pointers[1].button: 0, expected 2` (line 301), i.e. the drag no longer names the
    held button.

  Worth keeping about the harness: the host kit builds with
  `-Werror=unused-function`, so M-A5-1 had to remove `dom_button_held` as well. Left
  in place it became a *compile* failure, and a mutant killed by the compiler yields
  no named test failure and therefore no evidence.

  Verified: `bash scripts/test-linux-qt-host-kit.sh` → `Linux Qt Host Kit contract:
  PASS`, 22 input tests and 13 managed-session tests, zero failures, both under the
  offscreen platform and under xcb. This lane links a fake C ABI and builds neither
  the engine nor V8, so it is not covered by `verify-change.sh` and is run on its own.

- [ ] 0.7 A7: finish Android capability enforcement and revocation across all 30
  protected and 8 cleanup operations.

  Starting note, 2026-08-09, from checking the numbers rather than the sentence:
  they are current and they are already **derived**, not counted by hand.
  `scripts/test-permission-coverage-contract.sh` reports "30 gated op(s), 8 cleanup
  op(s), 38 permission-sensitive op(s)" and fails if an op is in both tables,
  neither, or carries the wrong wrapper. So the inventory and its classification are
  enforced; what is open is the enforcement and revocation *behaviour* at those
  points. This item has no detailed plan of its own — write one before starting, and
  scope it against that gate's output rather than a fresh survey.
- [ ] 0.8 A8: retained-intrinsic host bridge, mounted-module URL validation,
  ad-event authority, late callback rejection, reliable asynchronous host-result
  lane.
- [ ] 0.9 A9: runtime restart as a callback and resource ownership boundary.
  Plan: `2026-08-02-runtime-restart-generation-boundary.md` (12 tasks).
- [ ] 0.10 A10: Canvas recovery as one transactional resource operation.
- [ ] 0.11 A11: permission product contract, including the public Session API
  that seeds standing host decisions before content startup.
- [ ] 0.12 A12: reject invalid host pixel ratios, canonicalise Windows game
  identity, and settle a missing ad handler through its documented error path.
  **Implementation, tests, mutation evidence and fresh verification are done and
  recorded below for the two live clauses; the third was already satisfied and is
  corrected in place. Neither independent review has run, so the item stays
  open.**

  **Clause 1, pixel ratios: already satisfied, and this clause was stale.**
  `PixelRatio::new` (`engine/crates/shared/src/surface/geometry.rs:13`) requires
  finite and positive; `engine/crates/capi-abi/src/surface.rs` rejects invalid
  scale factors at the ABI boundary, with
  `generation_dimensions_and_scale_are_strictly_validated` iterating
  `[0.0, -1.0, NaN, INFINITY, -INFINITY]`. Every construction site validates and
  none has an `unwrap_or` fallback. No work was needed; the clause was describing
  a defect that had been fixed.

  **Clause 2's premise was false, and the defect underneath it was case, not
  Windows.** There is no Windows-specific game-identity code to canonicalise:
  every platform runs one shared `validate_game_id`
  (`engine/crates/shared/src/vfs/game_paths.rs`) with **zero `#[cfg]`**, reached
  from both entry paths (C ABI, Android JNI), and the Java SDK duplicates the
  same rule. What actually diverged was that the rule was **case-preserving**.
  Every other hazard was already blocked by the allowlist (`\`, `/`, `..`, ADS
  `name:stream`, trailing dots and spaces, `\\?\`, 8.3 short names, which always
  contain `~`); reserved device names passed it and cost availability only. Case
  was the live one: `PuzzleQuest` and `puzzlequest` both validated and, on NTFS
  or a default APFS volume, resolved to one directory — so the second title read
  the first's saves, its `wx.clearStorage()` wiped them, they shared the 10 MB
  quota, and `code_dir` collided too. A host that deduplicates ids by exact
  string was correct on three platforms and wrong on the fourth.

  **Rejecting beat folding, and the reason is that folding leaves two spellings
  of one identity.** The id space is now lower-case only, so a pair that folds
  together is unrepresentable rather than resolved per platform. A fold in
  `GamePaths::new` would have left `game_id()` and the directory name disagreeing,
  and every other producer of that path — `GamePathStrings` across the JNI
  boundary, the Java SDK's own `GamePaths` — would have had to repeat the fold or
  drift. A `#[cfg(windows)]` branch was rejected outright: it would make one id
  resolve differently per platform, so content moved between platforms would lose
  its data. Reserved device names (`con`, `prn`, `aux`, `nul`, `com0`-`com9`,
  `lpt0`-`lpt9`) are now rejected on **every** platform, for the same reason the
  rest of the rule is portable: a rule that admits an id one platform cannot
  create a directory for makes the same content fail on one platform only. The
  two refusals are separate variants (`InvalidGameId`, `ReservedGameId`) because
  a reserved id satisfies the character rule, and reporting it with the character
  rule's message sends a host looking for a character that is not there.

  **The lockstep requirement is now checked rather than hoped.** Both
  implementations read one table,
  `engine/crates/shared/src/vfs/game-id-vectors.txt` — Rust by `include_str!`, so
  a moved file is a compile error, and Java by walking up from the test's working
  directory, failing with the paths it searched rather than skipping. Widening one
  side without the other leaves that side's vector test red. Two independent
  per-language tables could not do this: both would stay green while the rules
  diverged. Each guard also has a case only it can see — the Rust suite is
  exhaustive over every byte `0x00`-`0xFF`, which the vectors do not enumerate,
  and the vectors see Java's agreement, which no Rust test can.

  **No disk-touching test is needed, and that is a consequence of the design
  rather than a gap.** The collision requires two *valid* ids that differ only in
  case; the validator makes that unrepresentable, so the property reduces to "no
  accepted id differs from its own ASCII lower-case spelling", which is checked
  directly on both sides. The allowlist is ASCII-only, so only ASCII folding can
  apply, which is standard on NTFS and APFS. A `PathBuf` comparison — what
  `storage_isolation.rs` and `two_session_identity.rs` do — is structurally blind
  to a case-insensitive filesystem and would not have seen this either way.

  **Clause 3's defect was live but not where the wording suggested, and it was
  the ordinary state of an integration.** Android returns `Some(AndroidAd)` gated
  only by the `api-commerce` feature (`platform/src/android/services/mod.rs`),
  never by handler registration, so `op_ad_is_supported` is `true` for every
  full-profile Java-SDK session and content takes the hosted path with no local
  fallback whether or not the embedder ever registered an `AdHandler`. Three of
  the six ad entry points settled (`createAd`/`loadAd`/`showAd` reported an
  error); `hideAd`/`updateStyle`/`destroyAd` did a bare `sAdHandlers.get` and
  returned. The larger defect was a contract mismatch: `AdHandler`'s javadoc
  documented the **no-service** contract ("incentivised video closes with
  `isEnded = false`") for the **service-installed-but-no-handler** path, where a
  rewarded `show()` emitted `error` and **never** `close`. Content following the
  wx idiom — `onClose` decides the payout and resumes the game — waited forever,
  and the SDK's own documentation had promised otherwise.

  **The same hole existed one layer over, in the interface defaults, and fixing
  only the no-handler path would have left it.** `AdHandler.showAd`'s default
  emitted an error and no close, and `hideAd`'s default was a no-op comment, so a
  registered handler that does not sell a format stalled content exactly as a
  missing handler did. There is now one settlement, `AdOp.settleWithoutAdvert`,
  and `theInterfaceDefaultsSettleExactlyAsTheMissingHandlerDoes` asserts the two
  paths produce identical event sequences. `AdEventSink.emitShowFailed` pairs the
  error with `close{isEnded:false}` in one call so the pairing cannot be
  forgotten, and the verdict is `false`, so reporting the absence of an advert
  mints nothing.

  **The fix is in the Java adapter because that is the only layer that can see
  the fact, and two existing tests prove the alternatives are wrong.** A JS-side
  timeout is prohibited by `ad_reward_integrity.rs:488
  hosted_ads_do_not_self_close_on_a_timer` and `:503
  hosted_ads_do_not_self_report_load_on_a_timer`, correctly: a slow real host
  would be raced by a fabricated close. An op-layer fix is impossible —
  `ad_command_op!` knows only whether a *service* exists. Routing
  `op_ad_is_supported` through handler registration was considered and rejected:
  registration is live session state a host may change, and the JS memoises
  `_hostAdsAvailable()` once per isolate, so a host that registers a moment late
  would silently lose every ad already constructed. Answering per call in the
  adapter has none of that, and needs no snapshot regeneration.

  **`AdOp.settleWithoutAdvert` is abstract, which is what covers a seventh ad
  command.** A shared default would have answered the question for whoever adds
  one; an abstract method makes the enum refuse to compile until the settlement is
  stated, and `AdSettlementTest` asserts its expectation table covers
  `values().length`, so the addition cannot be made silently either. The enum is
  nested but touches nothing in `NativeExports`, deliberately: that class holds
  `android.os.Handler` statics and cannot be loaded in a host JVM, and the same
  constraint is why `NativeExports.SessionPermissionSink` is shaped that way.

  **What the JVM harness cannot see is source-checked, and the mutant proves the
  gate earns its place.** *Which* resolver an entry point calls is invisible to a
  test that cannot load the class, so
  `scripts/test-ad-reward-integrity-contract.sh` grew check 5: every
  `ad*(int, String)` entry point must route through `adHandlerOrSettle`, none may
  read `sAdHandlers` directly, and the resolver must still settle. Reverting
  `adHide` to the bare lookup — the exact historical defect — leaves **every unit
  test green** and fails that gate with both messages.

  Mutation evidence, Rust half (`cargo mutants --file
  crates/shared/src/vfs/game_paths.rs --package migo-shared`): 46 mutants, 33
  caught, 6 unviable, 7 missed. No survivor is in the changed code —
  `validate_game_id` and `is_reserved_device_name` are fully caught, including
  both function-level replacements (`-> true`, `-> false`) and every boundary
  mutation on the length check. Hand mutants, one per claim, each killed at its
  own assertion:
  - Java regex re-admits `A-Z` → `theJavaGateAgreesWithTheEnginesVectorTable`
    (`GameIdVectorTest.java:56`) **and**
    `everyAcceptedIdIsItsOwnLowerCaseSpelling` (`:72`).
  - Java reserved-name check deleted → the vector test only (`:56`); the
    case-canonical test cannot see a reserved name, which is why both exist.
  - `AdOp.HIDE` settles nothing (the historical silent drop) →
    `everyAdCommandSettlesWhatContentIsWaitingFor` (`AdSettlementTest.java:109`)
    **and** `theInterfaceDefaultsSettleExactlyAsTheMissingHandlerDoes` (`:143`).
  - `emitShowFailed` drops the close → `aRewardedVideoThatCannotBeShownStillCloses`
    (`:123`), while `theSettlementNeverMintsAReward` stays green.
  - `emitShowFailed` closes with `true` → `theSettlementNeverMintsAReward`
    (`:136`), the discriminating pair for the previous line. Both claims were in
    one test first; two mutants killing the same assertion line is what showed
    the reward claim was riding along, and the test was split.
  - `adHide` reverts to `sAdHandlers.get` → all unit tests pass, ad reward
    integrity contract FAILS. That is the gate's whole justification.

  **Residual, recorded and not closed:** the same mutation run reports seven
  survivors elsewhere in `game_paths.rs`, all pre-existing and none in the
  changed rule — `Display::fmt` for `GamePathError` is now pinned by
  `the_two_refusals_name_the_rule_they_broke`, but `clean_cache -> Ok(())`,
  `user_data_size -> 1`, `cache_size -> 0` and three guards in
  `prepare_code_tree_for_removal` (`:330` `NotFound` match guard, `:330` `==`/`!=`,
  `:333` `||`/`&&`) survive. The last three are the symlink and not-found paths of
  sealed-tree removal, which `delete_all_removes_a_sealed_code_tree_and_receipt`
  exercises without discriminating; that is a real gap in a security-adjacent
  path and belongs to its own item, not to A12.

  **The mutation run also found a fixture that poisons the whole suite, and it is
  fixed here.** `delete_all_removes_a_sealed_code_tree_and_receipt` built its tree
  at a *fixed* `/tmp` path and cleared write bits on it. One mutant died between
  sealing and `delete_all`, leaving a `code` directory `remove_dir_all` cannot
  traverse — after which **every later run of the entire `migo-shared` suite
  failed**, and the failure read as a regression in the change under test. The
  root is now unique per invocation, so a leak is inert. Two lessons worth
  keeping: a test that removes permissions needs a path no later run will reuse,
  and `let _ = remove_dir_all(&root)` at the top of a test is not cleanup, it is a
  swallowed error that turns a leak into a permanent failure.

  Verification: the authoritative verdict block for this change is recorded once,
  in task T.6, because a single `scripts/verify-change.sh --base HEAD` run covers
  the whole working tree -- 41 verdict lines, 40 PASS and one `CI ONLY`, "verified
  for every target this change touches". The steps that carry this item are
  `test -p migo-shared` (424 passed), `android-java compile` over both product
  flavours (126 Full and 126 Slim, including this item's seven new cases:
  `GameIdVectorTest` 3 and `AdSettlementTest` 4), and
  `scripts/test-ad-reward-integrity-contract.sh` with check 5 added. No Android
  native compile was required and none is claimed: the change touches no
  `#[cfg(target_os = "android")]` Rust, and the selector's plan says so.

  **A gap in the local verifier, found by this item's own mutant, and fixed as task
  T.6.** `verify-change.sh` did not run the contract scripts at all — they lived
  only in `.github/workflows/pr-ci.yml`. So the M-A4 mutant above, which
  reintroduces the exact silent-drop defect, made the **local** gate print
  "verified for every target this change touches" while CI rejected it: the same
  shape as task T.1, one layer out again, a lane the verifier had no concept of.
  That lane is now derived from the workflow and runs on every invocation, and
  M-A4 re-run against it fails the local run. See T.6, including the way the lane's
  own first version under-ran silently.

- [ ] 0.13 HarmonyOS correctness. Expose a complete ArkTS-to-native lifecycle
  (today only `start` exists, and the ability's foreground and background hooks
  never reach the engine); give `OHNativeWindow` the same ownership and release
  barrier discipline as the Migo-owned X11 connection; verify multi-touch; apply
  the restart, thread-ownership, and shutdown barriers to HarmonyOS.
- [ ] 0.14 A13: re-run the post-master integration audit and record exact Full
  and Slim test counts.
- [ ] 0.15 A6: run lifecycle, reattachment, input saturation, ABI, and header
  contract suites with both product profiles. **The Slim host suite now runs, is
  green, and is part of the local gate; it found one product defect, which is fixed
  here. Neither independent review has run, so the item stays open.**

  **Nothing had ever run a Slim host suite, and the first one reported 36
  failures.** `crates/{core,platform,runtime-v8,graphics,capi}` all declare
  `default = ["profile-full"]`, so every `cargo test`, every `cargo check` and the
  whole of `verify-change.sh` compiled `api-media`, `api-commerce`,
  `api-connectivity`, `api-sensors` and `api-system` **on**. Not one
  `cfg(not(feature = ...))` branch had been built by any gate. CI builds both
  Android profiles through `build-aar.sh`, so a Slim *compile* failure would have
  been caught -- a Slim *behaviour* failure never could be.

  **Six of the failures were a real defect: on a Slim build no canvas ever followed
  its surface.** `_internalTriggerWindowResize` is the ingress
  `core/src/runtime/host.rs` calls when the window changes, and it lived in
  `runtime-v8/src/system/12_window_resize.js` -- inside the extension
  `api-connectivity` gates out. A Slim build therefore had no such hook at all:
  rotate the device or resize the window and the canvas kept the size it had
  before, forever. The file's own comment argued the opposite of where it sat,
  saying the canvas adoption runs "first, and unconditionally... a host with no
  window-info service still has a surface", which is exactly the reasoning that
  makes `api-connectivity` the wrong home for it.

  Fixed by splitting the ingress along that argument rather than by moving the file
  wholesale. `handleSurfaceResized` is now in the always-compiled
  `web/03_canvas.js` and registered by `98_global_scope_window.js`, next to
  `_internalTriggerWebglContextEvent`, which is the same shape of always-on host
  hook. The optional half -- reading window geometry, de-duplicating it and
  triggering `wx.onWindowResize` -- stays in the `system` extension and *subscribes*
  through `setWindowResizeReporter`. So the profile can delete what content
  subscribes to and cannot delete what the canvas needs. `wx.onWindowResize` and
  `offWindowResize` stay registered where they were, because a profile without a
  window-info service genuinely has no geometry to report.

  **The other 30 were the suite asserting against APIs the profile does not ship,
  and are now gated at the module.** `ad_reward_integrity` (19) needs
  `api-system`, `permission_reporting` (6) needs `api-connectivity`, and
  `permission_revocation` (4) needs both `api-media` (camera, recorder) and
  `api-connectivity` (bluetooth). A `#[cfg]` there says the profile does not ship
  the API; it does not say the test is optional.

  `published_namespace_isolation`'s one failure was neither: it asserted more than
  300 published names and the presence of `wx.getSystemInfoSync`, both of which are
  Full-only. Deleting it in Slim would have dropped the property; instead it is
  profile-aware -- Slim publishes 127 `wx` names and 132 `migo` names, measured, so
  the floor is 100 there, and the API probed in both profiles is one neither can
  drop (`createCanvas`, plus `getStorageSync` in place of `getSystemInfoSync`).
  A threshold that asserts the Full numbers everywhere asserts the product profile,
  not the publication.

  **Both Slim suites are now host steps in `verify-change.sh`,** so the branch
  cannot silently reacquire this gap: `runtime-v8` and `core` are the two crates
  whose capability surface the profile selects, `graphics` takes its profile from
  `core`, and `capi`/`platform` do not build on the host at all.

  Measured: `runtime-v8` Full 522 passed, Slim 471 passed (from 36 failures);
  `core` Full 62, Slim 59; both zero failures. The engine JS in this fix is inert on
  a device until `scripts/gen-snapshot.sh` is re-run there -- `cargo` only selects a
  committed V8 startup snapshot, never regenerates it -- so the on-device handoff
  must say so. The host suites run the JS from source, which is why they can see the
  fix at all.

  Still owed by this item: the lifecycle, reattachment, input-saturation, ABI and
  header contract suites named in A6 have not been run under Slim; only the two
  crate suites have. The ABI and header ones need the C package, which is a target
  build rather than a host suite.

  **All five are now run, and both sentences above were wrong.** They are corrected
  in place rather than deleted, because what they got wrong is the pattern this
  ledger keeps repeating.

  - **The lifecycle, reattachment and input-saturation suites are `migo-capi` lib
    tests**, not suites of their own: `lifecycle_calls_are_accepted_before_a_surface_exists`,
    `lifecycle_state_and_session_levels_are_retained_without_a_surface`,
    `returning_to_the_created_state_is_still_rejected` and
    `unknown_lifecycle_states_are_told_apart_from_unsupported_transitions` for
    lifecycle; `platform_context_rejects_display_or_kind_change_before_native_access`
    and `platform_context_reuses_the_exact_wayland_graphics_domain` for reattachment;
    `input_saturation_callback_is_once_per_episode` for saturation. Both profiles now
    run them: **capi Full 143, Slim 143; platform Full 53, Slim 53** (52 before this
    item's new test, plus one ignored X11 case needing a live server).
  - **The ABI and header suites need no C package.** `migo-capi-abi` has no
    dependencies and no features -- that is why it was split out of `capi` -- and its
    **60 tests** across nine binaries, including
    `tests/header_validation.rs`, run on the host in 0.01s. "Both product profiles"
    is satisfied there by construction rather than by running twice: a crate that
    declares no features has exactly one build.
  - `verify-change.sh` had **no step for that crate at all**, and the comment
    justifying the absent Slim steps read "`capi` and `platform` do not build on the
    host at all" -- four lines below the two steps that build and test them. Four
    steps were added: `capi-abi --all-targets` and Slim for `runtime-v8 --tests`,
    `capi`, `platform`. `graphics` gets no Slim step for a reason worth stating:
    its `profile-full` and `profile-slim` both expand to exactly `["embed_icudtl"]`,
    so the two builds are the same build.

  Pulling that thread found a larger gap than the item -- 95 tests in thirteen
  integration binaries that no local step ran -- and the audit that makes it
  unrepresentable is **task T.7**, with its mutation evidence.

  **The Slim steps are not decoration, and one defect proves it.** `crates/capi/src`
  contains no `cfg(feature)` at all, so a Slim capi run differs only in the graph
  beneath it; the profile-conditional host-compiled code is
  `jni_profile_contract::active_methods`, which selects the Android JNI surface with a
  chain of five `#[cfg(feature)]` attributes. **It had no test.** Every profile test
  asserted over `methods_for`, a `#[cfg(test)]`-only restatement of the same rule, so
  the production chain and the declared rule were two implementations of one rule with
  only the unshipped one checked. `the_registered_surface_is_the_one_this_profile_declares`
  equates them, and `platform/src/lib.rs` now rejects a build with *neither* profile
  feature so "which profile is this" is a total question -- every dependent already
  forwards one through its own `default`, so only a bare `--no-default-features` is
  refused, and that is not a product.

  Two mutants, and the pair is the point:

  - **M-A6-5** delete `#[cfg(feature = "api-system")]` and its `extend_from_slice`, so
    a Full build registers no System JNI methods. The suite **as it was** -- the same
    run with the new test skipped -- reports **52 passed, 0 failed**. With the new test:
    FAILED. A Full build would have shipped four JNI methods short, and content calling
    one would get `UnsatisfiedLinkError` at the moment it used it.
  - **M-A6-6** drop the `#[cfg(feature = "api-sensors")]` attribute so Sensors is
    registered unconditionally. **Full passes** -- that feature is on there, so no Full
    run can see it -- and **Slim fails**. That is the evidence the Slim step earns its
    place: a defect no Full suite can observe.

- [x] 0.16 Fix the process-global text texture cache. **Closed: implementation,
  both reviews, Section 7.3's gate, the recorded residual, and — last — the
  two-live-Session behavioural test Section 6.4 requires** (`b73ac60`). The cache is now per session: a registry
  hands out a reference-counted per-session cache with its own lock, byte budget,
  trim accounting and font generation, and both the JavaScript and render sides
  resolve their handle once at bring-up so the registry lock never appears on a
  render path. Adding a session field to the key was rejected because it would
  have fixed only the collision and left a process-global mutex on a per-frame
  path, which Section 7.3 forbids, plus a shared eviction budget. Teardown drops
  the cache after the render thread joins and the isolate is gone; the startup
  guard clears it too, because the cache registers lazily from whichever side
  reaches the session first and so has no explicit registration edge. The canvas
  op state now has exactly one constructor and it always registers, so the
  invariant is enforced by construction rather than by a documentation note.
  Verified at shared 373, runtime-v8 497, graphics 519, core 46, capi 136,
  platform 50 — 1621 passed, no failures; `cargo fmt --all --check` and
  `git diff --check` clean. Mutation-tested: a shared lookup ignoring session
  identity fails all four isolation tests while the mechanics tests still pass.
  Noted while fixing, tracked under task 0.19: `Host::drop` clears the shared
  image cache and the global io cache, so one session ending still wipes those for
  every live session, and `render_diagnostics::set_text_cache_gauges` remains a
  process-global accumulator so two sessions' gauges interleave. **Both cache halves
  are since fixed** — the io `clear()` removed, the alias table partitioned per
  Session — ~~leaving only the gauges.~~ **and the gauges are fixed too, which this
  line went on claiming after the fact.** The accumulator and its sink are both
  thread-local (`HOT` and `SINK` in `render_diagnostics`), and
  `a_gauge_set_before_another_session_sets_its_own_still_reports_its_own` pins it in
  four ordered phases — set A, set B, flush A, flush B — precisely because two threads
  setting gauges at once would catch a merge only on a lucky interleaving, and because
  a counter test cannot stand in: counters publish with `fetch_add`, gauges with
  `store`, so a merge means the session that flushed last silently speaks for both.

  **Independent code-quality review done; spec review still outstanding.** Reviewed
  as a five-commit batch isolated in its own worktree, so the diff was the
  correctness work alone and not the V8 build changes that followed it.

  One finding, and a real one: the registry could be **repopulated after teardown**.
  On a GPU startup timeout `Host::new` calls `render.shutdown_detached()`, which does
  not join, and the startup guard then unregisters the cache — but the still-running
  render thread had not yet reached its own `text_cache_for_host`, so it recreated
  the entry with no `Host` left to remove it. Every failed startup leaked one entry
  for process life. The render thread no longer resolves the cache at all: the host
  resolves it before spawning and passes the handle in, so the guard's unregister is
  final. Verified structurally — `text_cache_for_host` no longer appears anywhere in
  the graphics crate, and the only two callers left are the host's render service
  (before the spawn) and `CanvasOpState::for_host` on the JS side, which is dropped
  before the guard runs.

  The recorded mutation evidence was re-verified independently rather than taken on
  trust: replacing the per-session lookup with one shared cache fails exactly the
  four isolation tests and leaves all eight mechanics tests passing, as claimed.
  Suites after the fix: migo-shared 373, migo-platform 50 with 1 ignored,
  migo-core 46, migo-capi 136, `cargo fmt --all --check` clean.

  **Spec review outcome: compliant with Section 6.4 defect 1, NOT yet gated by
  Section 7.3.** Section 6.4 names two halves — the session-blind cache key holding
  another context's GL texture names, and the process-wide font generation counter
  that let one game's font reload invalidate every other game's cached text. Both are
  fixed, and `font_generation_is_per_session` covers the second half specifically,
  which the earlier summary of this work did not call out.

  Section 7.3's "no cross-session lock on a per-event path" was a different matter.
  The implementation satisfied it structurally — both sides resolve the handle once
  at bring-up, so the registry `RwLock` never appears on a render path — but 7.3 says
  these are "enforced by tests, not by inspection", and structural satisfaction is not
  the gate. **Now gated behaviourally** by
  `a_per_frame_text_cache_hit_does_not_reach_the_session_registry`, which holds
  `SESSION_CACHES` in write mode against a frame (task 0.27). This line previously
  misnamed that obligation as task 0.26's allocation gate.

  **What closed this item was the last thing on Section 6.4's own list: two live
  Sessions.** Every isolation test the cache had takes two host ids from
  `text_cache_for_host` and shows the registry separates them — a claim about a
  registry given distinct keys, which is the exact shape task 0.62 replaced for
  storage. The step before it was never executed: that a Session *binds* its own
  cache, through `CanvasOpState::for_host` in the `web` extension's state init, so two
  live Sessions land on two caches without anyone choosing a key.
  `two_live_sessions_hold_their_own_text_texture_cache` reads the cache out of each
  live op state rather than resolving one, and caches the **same label** from both —
  two different labels would separate the entries by key and pass over a cache with no
  session identity at all.

  Two details are the discipline restated. Both entries are written before either
  lookup, because a process-wide slot written at bind time shows up only once both
  Sessions have written. And the step that caches is deliberately assertion-free: its
  first version asserted the insert evicted nothing, and the mutant then killed the
  test **inside that helper**, detecting the defect while never evaluating the claim
  the test is named for.

  Mutation: binding the cache from a constant host id instead of the Session's own —
  the plumbing a test that resolves a cache itself cannot see — fails only this test,
  at its own assertion, with all 414 `migo-shared` tests and both its siblings in the
  file passing.

  **Not covered, named rather than implied.** The GL half stays unobservable here: the
  entries hold texture names, and whether a name minted in one Session's EGL context is
  refused in another's needs two live GL contexts, which no host test has. What is
  gated is that the two Sessions never reach the same entry, which is what makes the
  question moot rather than answered.
- [x] 0.17 Arbitrate device-exclusive resources across Sessions.
  **Implementation complete, reviews outstanding.** Camera, microphone and the
  Bluetooth adapter were acquired independently by each Session's own manager, so the
  second acquirer silently broke the first: a capture already streaming would go
  black or corrupt with nothing for the incumbent to observe.

  **Policy, decided with the user rather than assumed: first come, first served.**
  The incumbent keeps the device and the newcomer is refused. Revoking the incumbent
  instead was considered and rejected — arrival order is not evidence of who should
  own the device, since Migo cannot see which game the user is looking at, so
  preferring the newcomer would break a running capture on the strength of nothing.
  The loser learns through the existing `<op>:fail <reason>` channel
  (`createCamera:fail in use by another game`,
  `recorderManager.start:fail in use by another game`,
  `openBluetoothAdapter:fail in use by another game`), so no callback and no header
  change was needed.

  `ExclusiveDeviceArbiter` holds the ownership, process-wide, because a per-Session
  structure cannot arbitrate between Sessions. Two design points worth stating:

  Resource identity is **granular, not one key per device class**. A phone has
  several physical cameras and `CameraManager` is constructed per camera id, so
  keying on "camera" would have refused two Sessions using *different* cameras, which
  is legitimate. The key is the resolved position, and a fixture covers front and
  back being independently ownable.

  Ownership is a single owner per key, **not a count**. A Session re-acquiring what it
  already holds succeeds and needs no extra release: within one Session the manager
  owns its own lifecycle, and counting would turn a Session's own double-release bug
  into a leak that outlives the Session.

  Release happens on the normal paths — `closeAdapter`, `stopInternal` so a session
  that merely stops recording does not keep the microphone, and `CameraManager.destroy`
  — plus unconditionally in `GameSession.close` via `releaseAll(sessionId)`. That last
  one cannot rely on each manager having released cleanly, because a failed release may
  be the reason the Session is being torn down.

  **Audio focus is deliberately not arbitrated here**, and this was verified rather
  than assumed. Android already owns that decision, transfers focus to the latest
  requester, and each Session registers its own listener — so when a second Session
  takes focus, the first's `onAudioFocusChange` fires `AUDIOFOCUS_LOSS` and
  `NativeMethods.onAudioInterruptionBegin(sessionId)` reaches that Session. The
  documented outcome for the loser therefore already exists; adding a second policy
  on top would either contradict the platform or duplicate it.

  Seven fixtures, and the important one is a race: 200 rounds of two threads
  acquiring the same key behind a start latch, asserting exactly one winner. A
  check-then-act arbiter passes every sequential fixture and fails only that one,
  which is what the mutation confirms — replacing `putIfAbsent` with get-then-put
  fails exactly 1 of 7.

  Verified at 103 tests per flavour, Full and Slim, up from 96, with no failures,
  errors, or skips; permission coverage contract still 30 gated, 8 cleanup, 38
  sensitive.
- [ ] 0.18 Fix the Rust permission gate open ordering race. **Implementation
  complete, reviews outstanding** (`0d4bfe9`). The tombstone was a high-water
  mark, which refuses any id at or below the highest ever opened rather than the
  ids actually retired; those coincide only when ids arrive in allocation order,
  and they do not, because ids are allocated on the caller thread while the gate
  is opened from each session's own thread. It is now an explicit retired-id set
  recorded on clear, so admission is order-independent while a cleared id still
  cannot be resurrected. `open` is `#[must_use]`, and a refusal is reported only
  when the id was retired: refusing a merely-live id is normal because restart
  recreates device services for the same live session and must keep its standing
  grants, which the inherited design requires. Verified at migo-platform 50
  passed with 1 ignored, from a 45 baseline, plus migo-shared 369, migo-core 46,
  and migo-capi 136 with no regressions; `cargo fmt --all --check` and
  `git diff --check` clean.

  **Independent code-quality review done; spec review still outstanding.**

  One finding, and a real one: the `#[should_panic]` test asserted a
  `debug_assert!`, so `cargo test --release -p migo-platform --lib` failed
  outright — worse than the silence the test was written to catch, because it made
  the release profile unusable and so stopped anyone running the rest of these tests
  in the profile that ships. There are now two tests: the debug one still requires
  the panic, and a release one requires the call to return while the retired id stays
  refused, so each profile asserts its own contract. Both pass.

  **Correction to this item's recorded mutation evidence.** It claimed
  "reintroducing high-water-mark semantics alongside the retired set re-fails both
  ordering tests", which is imprecise and, on one reading, false. Reproduced
  faithfully — a high-water mark over every id *ever opened*, which is the semantics
  `0d4bfe9` removed — the mutation is killed by exactly two tests:
  `a_lower_host_id_opened_after_a_higher_one_still_gets_its_permissions` and
  `clearing_one_host_leaves_another_live_host_untouched`. The third candidate,
  `a_cleared_id_stays_retired_when_a_higher_id_opens_afterwards`, **passes** under the
  mutant and so does not discriminate: a high-water mark satisfies its expectation
  too. A first attempt that computed the mark over the *retired* set only killed one
  test, which is why reproducing the claimed mutation exactly matters before judging
  the claim.

  **Spec review outcome: compliant with Section 6.4 defect 3, NOT yet gated by
  Section 7.3.** Section 6.4 names two halves — `open` rejecting any id at or below a
  process-wide high-water mark, and its return value being discarded at the call
  site. Both are fixed: the mark is gone, and `open` is `#[must_use]` with
  `open_or_report` as the reporting call site. ~~Section 7.3's allocation and
  contention gates remain absent for this path: both mechanisms now exist (tasks 0.26
  and 0.27) but neither has been pointed here, and what needs gating is the
  *notification* traffic rather than arbitration itself — an acquire and a release
  happen once per session, not per event.~~

  **The contention half is now gated, and the sentence above named the wrong thing.**
  Pointing the probe at this path found a live violation, not a covered one: it is not
  only the acquire and release that are per session. Every *gated device call* went
  through `permission_jni_call` to `PermissionGate::run(host_id, ..)`, whose first act
  was `host_state(host_id)` — a `Mutex<Hosts>` on a process singleton — and that
  includes the Bluetooth characteristic writes Section 6.1 names as a steady hot path.
  Fixed under task 0.66, whose entry carries the evidence.

  Resolved while fixing this: `HostCommand::Restart` is a payload-free unit
  variant, so restart cannot swap to different content, which is why preserving
  standing grants across restart is safe as well as specified.
- [x] 0.66 Take the process-wide permission map off the per-event device-call path.
  Found by pointing task 0.27's contention probe at the one path Section 7.3 says the
  requirement was first written for, which task 0.18 had recorded as needing nothing.

  **The recorded obstacle named the wrong thing, and this is the seventh time.**
  Section 7.3 said "the BLE notification path's Rust half is
  `cfg(target_os = "android")`, so a host test binary never compiles it", and task 0.18
  said "what needs gating is the *notification* traffic rather than arbitration itself
  — an acquire and a release happen once per session, not per event". Both are about
  the wrong object. The lock is in `crates/platform/src/android_permission_gate.rs`,
  which is `cfg(any(target_os = "android", test))` and therefore compiles and runs its
  tests on a host binary; and what is per event is neither the acquire nor the
  notification but the *gated call*. `permission_jni_call` reaches
  `PermissionGate::run(host_id, ..)`, which begins `host_state(host_id)` —
  `self.hosts.lock()` on a `OnceLock` singleton. Two sessions writing BLE
  characteristics serialised there on every call.

  Useful reflex again: ask which layer can *see* the property. The effect is on an
  android-only path; the lock is not.

  **Red first.** `a_gated_device_call_does_not_reach_the_process_wide_live_host_map`
  holds `PermissionGate::hosts` and requires a granted Bluetooth call on another thread
  to finish anyway. Against the pre-fix implementation it timed out for the full two
  seconds and reported at the probe's own message, naming the live-host map. The gate
  needed one mechanism change to reach it: the probe took `&RwLock<L>` and this lock is
  a `Mutex`, so `assert_completes_while_mutex_locked` now shares a body with the
  `RwLock` form. Factoring that body out inverted a lock order — the guard became an
  argument, so it was taken *before* `ONE_GATE_AT_A_TIME` — and the mechanism's own
  `two_gates_never_overlap_and_so_cannot_blame_each_other_s_lock` failed immediately.
  The guard is a closure now, and the reason is recorded where it is taken.

  **The fix is the move two other paths already made**: a `SessionGate` resolved once
  when a session's device services are built, holding the `Arc<HostControl>` directly,
  exactly as task 0.16 did for the text texture cache and the input path did for the
  debug-stats registry. `PermissionGate::run` and `PermissionGate::scope_state` — the
  id-taking forms — are **deleted** rather than left beside the handle, so the
  defective call cannot be written; the nine service types that make gated calls hold
  the handle, and `permission_jni_call` takes it. `open_or_report` became
  `open_session`, which returns the handle, and a non-reporting `session` exists
  because the report is a `debug_assert!` and a test asserting that a *retired* id
  stays refused must not trip it.

  **What the fix moved, which is the interesting part.** `clear` removing the
  live-host entry used to be enough to refuse on its own, because every call looked the
  id up and got nothing; a handle keeps the control block alive, so the `Closing`
  lifecycle flag — previously belt and braces behind the map — is now the whole of the
  refusal. Nothing tested that:
  `clear_waits_for_inflight_update_then_leaves_a_tombstone` and
  `a_cleared_id_stays_retired_when_a_higher_id_opens_afterwards` both ask the gate for a
  handle *after* the clear, so they hold an empty one and are satisfied by the map alone.

  **A mutant walked, and the fixture was the reason.** The first version of
  `a_handle_taken_before_teardown_is_refused_after_it` asserted a *scoped* call, and
  removing `state.lifecycle = Lifecycle::Closing` from `clear` left all 52 tests
  passing — because `clear` empties the scope map too, so the call was refused for want
  of a grant either way. An **unscoped** protected call is the only one whose
  post-teardown refusal can come from nothing but the flag, and `close_adapter` and
  `stop_devices_discovery` are real unscoped gated calls. Rewritten that way, the same
  mutant kills that one test and 51 others pass.

  **Mutation evidence, files byte-identical by sha256 after each restore.**

  | Mutant | Kills | Survivors |
  | --- | --- | --- |
  | `clear` stops marking `Closing` (scoped fixture) | nothing | 52 |
  | `clear` stops marking `Closing` (unscoped fixture) | `a_handle_taken_before_teardown_is_refused_after_it` | 51 |
  | `run` inverts the required-scope check | 7 tests, including the contention gate's own `Ok(0xB1E)` | 45 |

  The third is not a pin — it kills too much to attribute — but it is the control that
  says the contention gate's burst really was an *admitted* call: a refusal completes
  instantly and would satisfy the timing assertion while proving nothing.

  **Not covered, named rather than implied.** What the contention test cannot see is a
  regression inside `SessionGate::run`: the handle holds no path back to the gate, so
  reintroducing the map lookup is a design change rather than a mutant, and the test's
  red half is the pre-fix implementation rather than a repeatable mutant. It stands as
  the guard against a future acquisition of anything process-wide on that path — which
  is exactly how the input send's stats-registry defect was found. The nine service
  types are `cfg(target_os = "android")`: they compiled for `aarch64-linux-android` and
  were not run. The JVM `PermissionOperationGate` is a different object with a different
  key and stays ungated (task 5.1). The allocation half of Section 7.3 is still not
  pointed at this path. `scripts/test-permission-coverage-contract.sh` matched the
  wrapper by its first argument, so it failed on the new call shape and its pattern and
  self-check fixture moved with it — 30 gated, 8 cleanup, 38 sensitive, unchanged.
  Verified by `scripts/verify-change.sh --base HEAD`: every host target plus the
  arm64-v8a Android compile, migo-platform 52 tests from a 51 baseline.

- [ ] 0.67 Take the BLE notification path off the heap and off the shared registry,
  on both sides of the JNI boundary. The last path Section 6.1's second bullet names,
  and the one both task 0.26 and task 0.27 recorded as remaining.
  **Implementation, tests, mutation evidence and fresh verification are all done and
  recorded below; neither independent review has run, so the item stays open** — this
  document's own status convention requires both, and a completion mark this ledger
  cannot support is the one thing worse than an open item.

  **It was recorded as ungated. It was also unmet, nine times per notification, and
  the two are different states.** Task 0.26 had already counted the Rust half —
  three `String`s, a `Vec<u8>` and a `Box` per notification — and left it as
  "uncovered". The count was five and the path also took a process-wide lock:
  `send_command_to_host` reads `HOST_SENDERS` to find the Session's sender, so every
  notification of every Session met every other one there. That is the same defect
  task 0.27 found on the input path, on a stream whose rate a peripheral chooses
  rather than a finger. The Java half added four more — a connection wrapper, two
  capturing lambdas and two `UUID.toString()` calls, one of which
  Section 6.1 names verbatim.

  **The recorded obstacle was "no host test binary compiles it", and that was true of
  the wrong half.** `onBLECharacteristicValueChange` is `cfg(target_os = "android")`,
  so the enclosing JNI function is indeed unreachable from a host test — but nothing
  that decides whether the path allocates or takes a shared lock has to live inside
  it. Reading three strings and a byte array out of the JVM is platform glue; filling
  a slot and enqueuing it is not. Splitting there produced
  `HostIngress::try_send_ble_characteristic_value`, which is portable, is what the
  gates call, and is the whole of the path that was defective. This is the eighth
  recorded obstacle on this branch to name something other than the real one, and the
  question that dissolved it is the same one as last time: *which layer can see this
  property?* — not *how do I reach this code?*

  **The fix's shape was already predicted here, and the prediction was wrong in a way
  worth keeping.** This ledger recorded the three identifiers as "interning
  candidates". Interning was rejected on measurement grounds: a hash of a 36-byte
  UUID costs about what copying it costs, and it buys a bounded cache, an eviction
  policy and a shared structure that two notification threads would contend on. A
  pooled slot that keeps its own buffers gets the same zero allocations from
  `clear` + `push_str`, with no cache, no policy and no sharing. What made this
  visible is that the pool had to exist anyway for the value bytes; once it did,
  interning was solving a problem the slot already solved.

  **`RecyclePool` is a second pool rather than a change to the first, and the
  difference is the point.** `PayloadPool` pools the slot and drops the value in it,
  which is right for a touch batch — a fixed array owns nothing — and wrong for a
  payload whose fields are buffers, because the value's `Drop` frees exactly what the
  next event would have reused. `RecyclePool` keeps the value alive and resets it in
  place, and it grows on demand instead of preallocating: touch input starts on the
  first frame of every Session, so preallocation charges a Session for what it is
  certainly about to use, while BLE charges every Session for a peripheral most
  content does not have. An unused pool is now one empty channel. Capacity is still
  the queue's, so the pool never becomes the tighter bound — a peripheral streaming a
  firmware image must not lose packets while every other command still flows.

  **`BleCharacteristicData`'s fields are now private, and that is a gate rather than
  taste.** `device_id: id.to_owned()` reads as obviously correct and is the exact
  defect being removed. The only way to fill one is `overwrite`, and the mutation
  that reintroduces the owned identifier had to be written *inside* that method
  because there is nowhere else it can be written.

  **Three JNI calls per notification also went, which nothing had noticed.**
  `JNIEnv::get_string` does a `FindClass("java/lang/String")` plus an assignability
  test before every read, so three identifiers cost nine JNI calls and two local
  references each notification to re-derive what the `native` declaration already
  guarantees. `get_string_unchecked` is `unsafe` for a condition the JVM has already
  checked at the call site. The value now lands in a 512-byte stack buffer through
  `get_byte_array_region` — 512 because that is the ATT maximum attribute value
  length, so no conforming notification spills, and a larger one is delivered from a
  heap buffer rather than truncated: a silently shortened value is a payload the
  content would misread.

  **Mutation evidence, Rust.** Seven mutants, each killing the named gate and leaving
  the rest of migo-core green; all three files restored byte-identically (sha256).

  | Mutant | Kills | Measured |
  | --- | --- | --- |
  | An owned identifier per notification — the shape the code had | the allocation gate, and the buffer-identity test | 64 events / 64 iterations, 1088 bytes |
  | An owned value per notification | the same two | 64 events, 1280 bytes |
  | `recycle` keeps nothing | the allocation gate alone | 256 events, 6976 bytes |
  | `recycle` parks an absurd buffer forever | the retention test alone | — |
  | A loan that never returns to its pool | the pool-growth test and the allocation gate | — |
  | The `HOST_SENDERS` lookup restored | the host-registry contention gate alone | blocked for the full 2s |
  | A `shared::stats` lookup on the path | the stats contention gate alone | blocked for the full 2s |

  **The behavioural tests survive every allocation and contention mutant**, which is
  the argument for the gates existing: delivery is correct under all seven, so
  nothing else in the suite can see the defect.

  **Mutation evidence, Java, and it found a hole in the instrument.** Six mutants.
  The pre-fix dispatch — two capturing lambdas — costs **64 bytes per notification**,
  and re-formatting one UUID costs **80**.

  | Mutant | Kills |
  | --- | --- |
  | Two capturing lambdas per notification — the shape the code had | the dispatch gate alone, at 4096 bytes over 64 |
  | `uuidText` formats every time | the identifier gate alone, at 5120 bytes over 64 |
  | The UUID cache is unbounded | the bound test alone |
  | The carrier keeps the delivered event | both carrier-emptiness tests |
  | The probe skips its own self-check | **nothing, before the control below existed** |
  | The probe stops warming itself | the identifier gate, reproducing the false failure below |

  **The self-check mutant killing nothing is the recurring failure of this repository
  and it appeared again here.** `AllocationProbe` refuses to trust a zero until it has
  watched the counter observe a known allocation — the property that stops a JVM with
  counting disabled from turning every gate into a permanent silent pass — and no test
  covered the refusal. `AllocationProbeControlTest` manufactures a silent instrument by
  switching the counter off, requires the burst to refuse rather than pass, and requires
  the refusal to name the instrument rather than the path. Bursts are serialised inside
  the probe so that process-wide switch cannot redden another gate, for the same reason
  the contention probe serialises.

  **A real instrument defect, found by measurement rather than review.** The first run
  of the identifier gate reported **11048 bytes over 64 iterations on a path that
  allocates nothing**. The path was innocent: these tests compile against `android.jar`,
  which has no `java.lang.management`, so the counter is reached reflectively, and
  `Method::invoke` *spins a generated accessor class* after about fifteen calls. That
  class landed inside the measured window. The body's warm-up cannot cover it, because
  the instrument is not the body — the probe now warms itself, and the mutant that stops
  it reproduces 11048 exactly. Attribution was done by measuring three bodies against
  each other rather than by reading code: an empty body, a bare map lookup and the real
  one all reported the same 24 bytes, which is the instrument's own cost, while
  `UUID.toString` reported 5144.

  **Not covered, named rather than implied.** The `BluetoothGattCallback` body itself —
  where the connection wrapper is memoised — takes a `BluetoothGatt` and a
  `BluetoothGattCharacteristic`, framework classes a plain JVM test cannot obtain, so
  that one removed allocation has no gate on either side. Nothing here has run against a
  peripheral: no device evidence exists for this path at all, which is a gap task 2.2
  owns and this item does not close.

  **Verified.** migo-core 62 tests from 54, migo-shared 414 unchanged, the Android Java
  suite 115 with no failures or errors across both product variants, both variants'
  `compile{Full,Slim}DebugJavaWithJavac`, and `scripts/verify-change.sh --base master`
  reporting every host target plus **`PASS android compile`** — required here, because
  the change touches `cfg(target_os = "android")` code that no host run compiles.

  **One thing this change did not make worse, recorded because the next reader will
  wonder.** The six V8 snapshots are stale, and were already stale on `master` before
  this branch: the freshness gate reports the same two host profiles with `master`'s own
  `Cargo.lock` in place. This branch adds one line to that lock — a dev-dependency edge
  — which moves the fingerprint again but changes nothing about what has to happen,
  which is one regeneration round for the whole batch. Per this document's own rule,
  that round belongs last.

- [ ] 0.26 Build the allocation-count gate Section 7.3 requires, then apply it.
  **Mechanism built and applied to three paths; the two paths Section 6.1 names
  remain uncovered, so this item stays open.**

  The spec required "an allocation-count regression test" per hot path and claimed
  the requirement "already exists for the input transport". Audited while spec
  reviewing tasks 0.16 and 0.18: **it existed nowhere.** What
  `scripts/test-input-transport-contract.sh` does is grep the sources for structural
  properties — `VecDeque::with_capacity(`, a fixed payload-pool capacity formula, a
  non-zero reliable reserve, no `unbounded_channel`. Those assert the code is
  *written* not to allocate, which cannot observe an allocation and so cannot fail
  when one appears. The tree had no counting allocator and no `#[global_allocator]`
  at all.

  **What now exists.** `engine/testing/alloc-probe` — a new third crate group, since
  `crates/` is the engine and `tools/` drives it — holds a counting `GlobalAlloc`
  and `assert_no_steady_state_allocation(Burst { path, warmup, measured }, body)`.
  Design decisions and why each one is load-bearing:

  - **A separate crate reached only through `[dev-dependencies]`.** A
    `#[global_allocator]` is unique per binary, so one declared in a shipped crate
    would follow it into every cdylib. Cargo enforces the separation; a comment
    would not. Each consuming crate declares it under `#[cfg(test)]`, scoping it to
    that crate's own test binary.
  - **Per-thread counters, const-initialised and `Drop`-free.** `cargo test` runs
    tests concurrently against one allocator, so process-wide counters would
    attribute other tests' allocations to the burst. Const initialisation matters
    because a lazily initialised thread-local allocates on first touch — from inside
    the allocator. `try_with` and `wrapping_add` keep the allocator from ever
    panicking.
  - **Reallocation counts as an allocation event** (the open question in this item's
    earlier notes: resolved as yes). A container outgrowing its reserved capacity
    calls `realloc`, never `alloc`, so ignoring resizes would miss a `with_capacity`
    that became a `new`. Frees are reported but do not fail a burst; a burst that
    only releases is not allocating.
  - **Warm-up is mandatory and non-zero, and so is the measured span.** First-use
    lazy initialisation is not steady state; a zero-length measured burst is
    vacuous. Both are asserted.
  - **Every burst proves the allocator is installed before trusting a zero.** This
    is the important one. Without it, deleting one `#[global_allocator]` line turns
    every gate in that binary into a permanent silent pass — the exact failure mode
    this project has repeatedly found in guards that could not fail. The probe
    crate's own unit-test binary installs no counting allocator *on purpose*, making
    it the negative control that proves the refusal fires; `tests/harness.rs`
    installs one and is the positive control. One binary could not host both.

  **Applied to three paths, all in `migo-shared`:** the ordered host queue
  (coalescible motion, reliable and terminal transitions, plus the drain), the input
  payload pool at full occupancy including one request too many, and the
  per-`fillText` text texture cache hit. `test-input-transport-contract.sh` now also
  requires the first two gates to exist, since deleting a test is the one failure a
  test cannot report about itself.

  **Applying it found a real per-frame defect, which is the whole point.** The text
  cache's `pin` recorded pins in a `HashMap<TextCacheKey, u32>` beside the LRU.
  `HashMap::entry` needs an owned key, and a pin lasts exactly one frame — JS pins on
  a `fillText` hit, the render thread unpins after the copy — so every cached label
  allocated and freed the key's two `String`s on every frame. Measured at 128
  allocation events over 64 iterations, 1024 bytes. The pin count now lives on the
  resident entry, where a pin is an increment. Two consequences, both tested: a pin
  on a key with no resident entry is now refused rather than recorded for a future
  occupant (`pin` returns `bool` and is `#[must_use]`; no production caller ever did
  that, and `op_text_cache_peek_pin` collapses to a single lookup because a
  successful pin *is* the hit answer), and a replacement carries the outgoing entry's
  pins so an in-flight command keeps its protection.

  **Mutation evidence.** Each mutant killed exactly one test, at that test's own
  assertion, and every other suite stayed green:

  - A per-event `Vec` collect in `try_send_coalescible`: 128 events over 64
    iterations. Only the host-queue gate failed.
  - `retain` replaced by `drain(..).filter(..).collect()` in the terminal
    supersession path: 64 events. Pins the terminal half specifically.
  - A per-pop `Vec` in `SharedQueue::pop`: 128 events. Pins the drain half, so all
    three halves of that burst are separately attributed.
  - The payload pool returning a freshly boxed slot instead of the one it took: 256
    events. **The three pre-existing behavioural tests all passed under this
    mutant** — including one named
    `exhaustion_returns_the_original_value_without_a_heap_fallback`, which cannot
    observe a heap fallback. The gate is the only guard.
  - The text cache's key clone reintroduced: reproduces the original defect's exact
    128 events and 1024 bytes.
  - `let pins = 0` in place of the pin carry-over: only
    `replacing_a_pinned_entry_carries_the_pin_to_its_successor` failed.
  - Harness mutants: dropping the installation self-check killed the probe crate's
    negative control; folding warm-up into the measured window killed both the
    warm-up boundary test and the exact-count assertion; not counting `realloc`
    killed the resize test alone.

  **Verified.** migo-shared 391 from a 386 baseline (three gates plus two new text
  cache invariants), migo-alloc-probe 3 unit + 8 harness, migo-io 259/5 ignored,
  migo-runtime-v8 505/2, migo-graphics 519, migo-core 49, migo-capi 141,
  migo-platform 50/1, python CI suites 117 from 113, the verify-change contract 37
  from 36, `cargo fmt --all --check` and `git diff --check` clean.
  `scripts/verify-change.sh --base HEAD` reports every host step PASS and requires no
  target build: every changed file is portable, which the selector confirms rather
  than assumes.

  **Also extended, because introducing `engine/testing` would otherwise have opened a
  blind spot:** the module-walk audit only ever walked `engine/crates`, so its
  "across the whole tree" claim would have become false. It now walks a
  `CRATE_GROUPS` list, and the verify-change fixture grows a faithful `testing/` stub
  rather than folding the new crate into `crates/`.

  ~~**Remaining for this item.** The BLE notification path Section 6.1 names is not
  covered on either side.~~ **Both sides are now covered, by task 0.67.** What that
  item found, and this one had recorded otherwise:

  - The count was **five and a lock**, not five. `send_command_to_host` reads the
    process-wide `HOST_SENDERS` on every notification, which the allocation gate
    could not have seen — a registry read allocates nothing.
  - "A host test binary never compiles it" was true of the JNI function and false of
    the property. The allocating core was lifted into
    `HostIngress::try_send_ble_characteristic_value`, which is portable, and the gate
    calls that.
  - The prediction recorded here that the three identifiers are **interning
    candidates** was not what the fix used, and the reasoning is in 0.67: a pooled
    slot that keeps its own buffers gets the same zero allocations without a cache,
    an eviction policy, or a structure two notification threads share.
  - The Java half's JVM mechanism now exists —
    `platforms/android/.../AllocationProbe`, `ThreadMXBean` reached reflectively
    because these tests compile against `android.jar` — with a negative control that
    mutation testing proved was missing.

  ~~Still unmeasured: the render command path and the audio path.~~ **Both are since
  covered** — the render command path's two enqueues under tasks 0.38 and 0.41, and the
  audio path under 0.43 (the graph's per-quantum render), 0.47 (the thread's own tick),
  0.48 (the hardware output callback) and 0.49 (the streaming refill), all listed in
  Section 7.3's "Covered so far". This is the second time this item's hand-maintained
  remaining list named work already done, and the same correction as the
  `io::image_cache` line below. **`io::image_cache`
  no longer belongs on that list** — the prediction recorded here was right and was
  acted on: task 0.34 measured it (the pin/unpin pair allocated, the lookup did
  not) and task 0.36 removed the two owned keys on the layer above it. This
  sentence named work already done for one session, which is the failure mode a
  hand-maintained "remaining" list has in this repository; it is corrected rather
  than left for the next reader to redo.
- [ ] 0.27 Build the cross-session contention gate Section 7.3 requires.
  **Mechanism built and applied to the Rust per-event paths; the permission gate's
  JVM half stays with task 5.1, so this item stays open.**

  **The Rust permission gate is now among them, and it was a live violation rather
  than a covered path.** This entry said the gate had no such test and pointed at task
  5.1 for the replacement — which is the *JVM* gate. The Rust gate is a different
  object, and pointing this mechanism at it under task 0.66 found every gated Android
  device call taking a process-wide `Mutex` on the live-host map. The mechanism grew a
  `Mutex` form for it, sharing a body with the `RwLock` one; factoring that body out
  inverted a lock order and the mechanism's own overlap self-test caught it, which is
  the second time one of these probes' controls has earned its place.

  No such test covered the permission gate. The first attempt was withdrawn because it
  was provably unable to fail: the path it exercised took the shared lock inside the
  very helper the test called, so it passed with and without the property. The
  replacement is designed around `ThreadMXBean` blocked-time and is tracked as task
  5.1; this task is the wider obligation, because Section 7.3 requires the test for
  *each* covered per-event path, and task 0.16's render path needed one too — its
  freedom from the registry lock was structural, which Section 7.3 does not accept.

  **What now exists.** `engine/testing/contention-probe` holds the shared lock in
  *write* mode and requires the per-event operation, run on a thread of its own, to
  complete anyway. It observes an acquisition rather than reasoning about one, and it
  manufactures the contention instead of waiting for load — so an *uncontended*
  acquisition fails the gate too. Design decisions, each pinned by a control test:

  - **A write guard, not a read guard.** An `RwLock` admits concurrent readers, so a
    held read guard would let a per-event `read()` straight through.
  - **The operation runs on another thread.** On the guard holder's own thread a
    re-entrant `parking_lot` acquisition parks forever, so the defect would hang the
    suite rather than fail it. This is the one property whose mutant is not run: it
    hangs instead of failing, which is exactly the reason the spawn is not optional.
  - **A panicking body is re-raised as its own failure**, never reported as a block.
    Fidelity assertions belong in the body, and misattributing them would hide them.
  - **Only one gate runs at a time**, enforced inside the mechanism rather than asked
    of call sites. Two gates holding two different process-wide locks each blame
    *their* lock for the other's guard: the operation really did block, on something
    the report does not name. This was not hypothetical — it appeared during this
    item's own mutation testing before the serialisation existed.
  - **The bound cannot produce a false pass.** Shortening it can only make a correct
    path look blocked, which fails closed. An exclusivity self-assertion was written
    and then removed: it restated `parking_lot`'s own semantics, and it duplicated
    what the read-guard control already catches, which would have left neither pinned.

  **Applied to three gates.** The per-event input send, gated separately against
  `HOST_SENDERS` and against `shared::stats`'s `STATS` so a failure names which
  registry the path reached for; and the per-frame text cache hit against
  `SESSION_CACHES`, which converts task 0.16's structural claim into a behavioural
  one.

  **Applying it found a real defect on the hottest path in the engine.**
  `HostIngress::map_input_result` runs on the success path of *every* input event and
  called `shared::stats::get_stats(host_id)`, which takes a read lock on a
  process-wide `RwLock<HashMap<i32, Arc<DebugStats>>>` and clones an `Arc` out of it.
  Every touch move, pointer sample and gamepad frame of every Session took a lock
  shared with every other Session — and `HostIngress`'s own doc comment claimed calls
  through it "never acquire the global Host/VSync registries", which was true of those
  two and silent about this third one. The allocation gate from task 0.26 could not
  see it: an `Arc` clone is a refcount bump, not an allocation.

  Fixed the way the same struct already handles the queue, the payload pools and the
  saturation flag: the `Arc<DebugStats>` is captured at bring-up and held. Two
  supporting changes were needed and are the reason this is a fix rather than a
  move. `register_stats` became `stats_for`, get-or-create, because two bring-up paths
  reach for a Session's stats — host registration, so the input path can hold it, and
  the render thread — and neither is ordered before the other; always-fresh would have
  left whichever ran second holding a handle the other could not see, silently
  dropping its counters. And `register_sender` resolves the handle *before* taking the
  registry write lock.

  **That lock-ordering fix came out of the mutation testing, not out of review.** The
  first version resolved the stats handle while already holding `HOST_SENDERS`, which
  makes a lock cycle between the two process-wide registries. It was invisible until a
  mutant put a registry read back on the input path: a concurrent test then held
  `HOST_SENDERS` while blocked on `STATS`, the gate's own operation blocked on
  `HOST_SENDERS`, and the `STATS` gate failed naming the wrong lock. Acquiring one
  process-wide lock while holding another is now avoided rather than ordered.

  **Mutation evidence.** Each mutant killed the gates named and no others:

  - A per-event `get_stats` lookup restored in `map_input_result`: only
    `a_touch_send_does_not_reach_the_stats_registry` failed, and the host-registry
    gate still passed — which is also the evidence that the serialisation keeps
    failures attributable.
  - A bare `host_senders().read().len()` on the touch path: only
    `a_touch_send_does_not_reach_the_host_registry` failed. A first attempt at this
    mutant returned `Closed` when the host was absent, which changed behaviour as well
    as locking and so died at fidelity assertions in four tests instead; the mutant
    was narrowed until it changed only the property under test.
  - A `SESSION_CACHES.read().len()` in `TextTextureCache::peek`: only the render-path
    gate failed. The three allocation gates survived it, since a read lock allocates
    nothing — the two mechanisms see different things.
  - Probe mutants: a read guard in place of the write guard killed the read-guard
    control and the message control while correctly leaving the write-guard control
    passing; folding `Disconnected` into the timeout arm killed the panicking-body
    control; removing the serialisation killed the overlap control alone.

  **Verified.** migo-shared 392 from 391, migo-core 51 from 49, migo-contention-probe
  7, migo-alloc-probe 3 + 8, migo-io 259/5 ignored, migo-runtime-v8 505/2,
  migo-graphics 519, migo-capi 141, migo-platform 50/1, python CI suites 117, the
  verify-change contract 38 from 37, `cargo fmt --all --check` and `git diff --check`
  clean. `scripts/verify-change.sh --base HEAD` reports every host step PASS **and
  `PASS android compile bash scripts/build-android-so.sh --compile-only arm64-v8a`** —
  required this time, because the change touches `crates/core` and `crates/graphics`.

  **How another crate's test holds a private lock, and why that is not a leak.**
  `shared::stats`'s registry lock is exposed by
  `registry_lock_for_contention_probe()` behind a `contention-probe` feature that only
  `migo-core`'s `[dev-dependencies]` requests. Resolver 2 keeps dev-dependency
  features out of non-dev builds, and that was checked rather than assumed:
  `cargo tree --edges normal` resolves `migo-shared` as
  `[code-signing, default, v8-limits]`, while the dev-edge resolution adds
  `contention-probe`.

  **Remaining for this item.** The permission gate, which is what Section 7.3's
  contention requirement was first written for, is JVM-side: a Rust probe observes
  nothing about a Java monitor, so it needs `ThreadMXBean` blocked-time and stays with
  task 5.1. ~~The BLE notification path's Rust half is `cfg(target_os = "android")`, so
  no host test binary compiles it.~~ **The BLE notification path is now gated against
  both process-wide registries** (task 0.67): the `cfg` covers the JNI function, not the
  property, and the property moved to a portable `HostIngress` method that the probe
  calls directly. It was a live violation, not a covered path — `send_command_to_host`
  took `HOST_SENDERS` on every notification, which is the same defect this item found on
  the input path, on a stream a peripheral paces.

  **The audio path stays ungated, and that is now a decision with a reason rather than
  a gap.** Enumerated on 2026-08-08: the audio crate's *only* process-wide state is
  `streaming.rs`'s `OnceLock<tokio::runtime::Runtime>`, reached on the cold streaming
  path and never on a tick; the real-time paths hold no session id at all, so they
  cannot reach `shared::stats`, the console registry or the text-cache registry even by
  mistake — there is no argument to pass. A gate holding one of those locks around an
  audio tick would pass today and could only fail after a change that first plumbs a
  session id through the audio thread, which is a design change rather than a regression
  a gate catches. Writing one would satisfy the requirement's letter with a test that
  cannot fail for a real reason, which this document rejects elsewhere. The enumeration
  is the deliverable; if a session id ever does reach the audio thread, the gate becomes
  writable and required.

  **The enumeration also found two dead counters, and they are deleted rather than
  wired.** `DebugStats::audio_queue_hwm` and `io_queue_hwm` had no writer and no reader
  anywhere in the tree — their own comments said "placeholder — wiring to actual sender
  is deferred". A diagnostic field that is always zero is worse than a missing one: the
  first HUD to read it reports a queue depth of zero and is believed. Same disposition
  as this item's `vsync::send_vsync`, for the same reason.

  **Deleted rather than documented:** `vsync::send_vsync` took a process-wide read lock
  per frame and had no caller at all — every per-frame producer already goes through
  `HostIngress::try_send_vsync` — yet it stayed exported from `migo_core`. A gated
  defect left in the tree waiting for its first caller is worse than one that is live,
  because nothing fails while it waits.
- [x] 0.29 Decide how strong a pack-backed cache identity has to be. Task 0.28 took
  the identity from a 32-bit aggregate CRC32 plus app-chosen labels to a SHA-256 over
  every entry's path, size and CRC32. That makes accidental collision negligible and
  makes a crafted one much harder, but it is still metadata: the package format's
  per-entry integrity primitive is a CRC32, so a package built to match another's
  per-entry paths, sizes and CRC32s produces the same identity and can still be served
  another game's decoded pixels.

  **Decided, and stated in Section 6.5: a game's package is untrusted with respect to
  every other game's.** Which makes the property to implement a sharper one than
  "harder to collide" — producing a shared key must require holding the content, so
  agreeing on a key implies agreeing on bytes and sharing can disclose nothing. The
  identity is now a SHA-256 over the package file's bytes.

  **Scoped, and it is cheaper than it first looked — neither a startup cost nor a
  format change is needed.** The two options recorded earlier were hashing the package
  at mount and extending the package format. Both are avoidable, because
  `install_package_signed` **already reads the whole package into memory**, to hand it
  to the signature verifier. A SHA-256 taken there is free of extra I/O, and the
  install record it belongs in already exists: `ManifestEntry` is a serde struct in
  `manifest.json`, so a `#[serde(default)] content_digest: Option<String>` field is
  backward compatible. `restore_installed_packages` then reads the digest at session
  start with no extra work and hands it to the `PackSource`, whose `source_identity`
  prefers it over the metadata digest.

  Two things to settle when doing it. A manifest written before the field existed has
  no digest, and the right answer is to compute it on first mount and persist, so the
  cost is paid once per already-installed package rather than every session. And the
  scope is narrower than assumed, which is worth knowing: production never mounts a
  pack as the **base** — `swap_base` with a `PackSource` appears only in tests, the
  base is directory-backed, and the only production pack mounts are subpackages via
  `install_package` and `restore_installed_packages`. So the manifest covers every
  pack-backed mount that exists, and the metadata digest remains only as the fallback
  for a backend that has no install record.

  **Checked, because it could have made the whole task unnecessary, and it does not:
  nothing registers a signature verifier.** `register_signature_verifier` has no
  caller anywhere in the tree, so `verify_package_signature` takes its no-verifier
  branch — one warning, then accept. Package signatures are therefore not enforced
  today, and "a crafted package cannot be installed" is not available as the reason
  this identity may stay weak.

  What the attack needs, stated so its cost is visible rather than assumed: a game
  installs its own subpackages through `op_install_subpackage`, so it controls its
  package's bytes, its mount prefix and its entry paths, and can match another game's.
  The remaining barrier is the digest covering **every** entry's path, size and CRC32 —
  the attacker has to reproduce the victim package's whole entry table, so it needs
  that package in hand, and then hit a per-entry CRC32 while holding each size, which
  CRC32's linearity makes straightforward. Feasible against a package the attacker can
  obtain; not reachable by accident. The pay-off is another game's decoded pixels for
  that path.

  So this is real work rather than a formality, and registering a verifier would be a
  second, independent reason to want it done — the two are not substitutes, since a
  signature says who published a package and this identity says which package's bytes
  a cache entry holds.

  **The plan above was unsound as written, and checking its premise is what found the
  reason.** It rested on trusting a digest read back from `manifest.json`, and that
  file is writable by the game it describes: `VirtualFS` maps `/cache` read-write onto
  the per-game cache **root**, and the install store is `<root>/packages/`. A recorded
  digest there would have been a label the installing app picks — exactly the defect
  0.28 removed when it stopped keying on `name` and `version` — and worse than the
  CRC32 weakness it was meant to close, since claiming another package's digest needs
  no construction at all. The same mapping also exposes the install staging directory
  (`<root>/.staging_*`), so a game could have swapped the staged package between the
  moment the install validated and digested it and the moment it was renamed into
  place, which would have decoupled the digest from the bytes actually served.

  **Fixed first, because everything else depends on it.** `/cache` now maps to
  `GamePaths::sandbox_cache_dir` — a dedicated subdirectory — so every VFS root is a
  directory of its own and the cache root's runtime state (install store and record,
  staging directories, the derived-asset cache) is a sibling of all of them rather than
  a child of one. `buffer_url_dir` moved into the sandbox subtree with the mapping,
  because a `createBufferURL` payload is the game's own bytes handed back to it and its
  path has to stay one the game can read; the derived-asset cache and install store
  stay at the root. A second trust bug closes with it: restore mounts the *prefix* from
  the manifest without validating it, and an empty prefix is a whole-tree overlay, so
  that read was only safe once the record became runtime-owned.

  **Implementation, three parts.** `PackageDigest` is a SHA-256 over the package
  file's bytes, taken from the in-memory buffer at install and by a streaming
  fixed-buffer read otherwise, and truncated to `u64` where it meets the key space.
  `PackSource` stores it as a field, which also removes a per-call cost nobody had
  noticed: `source_identity` was recomputing a SHA-256 over the whole sorted entry
  table on **every call**, and `MountEntry::source_identity` is called inside
  `MountTable::resolve` — so every pack-backed read, `require` and image resolve
  allocated `entries × 32` bytes and hashed them. `install_package` returns an
  `InstalledPackage { identity, digest }` so the caller's manifest write records the
  digest, and restore takes the recorded value, reading only the package index; a
  record without one is digested and written back so the cost is paid once per
  installed package rather than once per session.

  **Correction to the scoping above:** it expected the metadata digest to stay as the
  fallback for a backend with no install record. It is deleted instead, because
  `PackSource::open` can derive the identity from the file whenever no record supplies
  it — which also makes the simple constructor the sound one and the cheap path the
  opt-in, rather than the other way round.

  Deliberate and recorded rather than glossed: content packed twice with different
  chunk sizes no longer shares one decoded copy, which loses sharing and not safety;
  the writer is deterministic for identical input, which
  `streaming_add_matches_buffered_add_bit_for_bit` already pins, so the ordinary case
  still shares. And the 64-bit truncation is the key space's width, not the digest's —
  `ResolvedCode::source_identity` is a `u64` and the on-disk derived-asset key encodes
  eight bytes, so a wider identity would be truncated at the next hop; a second
  preimage costs on the order of 2^64 hashes and yields another game's decoded pixels.

  **A hole this change opened, found by asking what the record is worth when a
  write fails, and closed.** Before it, a stale digest could not mislead anyone,
  because restore always derived the identity from the bytes. Once restore trusts the
  record, a replacement whose own record never lands — the manifest write is only a
  warning for `loadSubpackage`, so a full disk produces it — leaves the *previous*
  package's digest describing the new bytes, and a Session still holding the previous
  package shares a decoded entry with them. Ordinary write ordering fixes it: the
  install drops any digest recorded for the file it is about to replace *before*
  replacing it, and refuses the install if that write fails, because a record it
  cannot make safe must not be allowed to describe new bytes. The intermediate state
  is always safe — a record with no digest is one a restore digests for itself.

  Honest about what the test for it proves. It pins the reachable state (install
  succeeds, record never lands) and it failed before the fix, but it does **not**
  discriminate *when* the invalidation happens: a mutant that invalidates after a
  successful install instead of before the rename passes it, because that install
  succeeded. The ordering earns its place against a process kill between the rename
  and the record write, which the suite cannot observe without a fault-injection seam.
  A power-loss window remains open beyond that, because neither the manifest write nor
  the rename is fsynced and nothing orders them across two files; closing it belongs
  with a durability review of the store rather than here.

  Seven tests, and the important one builds the defect rather than describing it: two
  packages whose entry contents differ while their **paths, sizes and CRC32s agree
  exactly**, which is cheap to construct because appending a message's own CRC32
  little-endian drives the result's CRC32 to a constant. The fixture asserts that
  agreement before asserting the identities differ, so it cannot pass by not being the
  case it claims. Six mutants: five killed by one test each at its own assertion — the
  pre-fix metadata identity kills the collision test while **passing** the sharing test
  (which is why both exist); a value unique per mount — the tempting over-fix — kills
  the sharing test and passes the collision test; digesting half the package kills the
  install-reports test; restoring with `PackSource::open` instead of the recorded
  digest kills only the recorded-digest test; and dropping the write-back kills only the
  persistence test. The sixth killed nothing and is reported above: invalidating after
  a successful install rather than before the rename. The containment test needed no
  mutant: it was watched failing against the pre-fix mapping, and its message named the
  install-store path that mapping exposed.

  Verified at migo-shared 381 passed, up from 374, with migo-io 259, migo-runtime-v8
  504 with 2 ignored, migo-core 49, migo-capi 141 and migo-platform 50 with 1 ignored
  all unchanged; `cargo build --workspace --all-targets` clean, `cargo fmt --all
  --check` and `git diff --check` clean, and no new clippy findings in the five files
  touched.

  Not covered: no fixture drives two live Sessions through the JS `Image` path to the
  same virtual path out of colliding packages; the tests exercise the identity and the
  install/restore records directly. Java's `GamePaths.getCacheDir` javadoc claimed it
  was what `/cache` maps to and now says what it is.

  **Found while doing this, not fixed here, and neither is a regression from it.**
  `op_install_subpackage` takes `zipPath` from JS and hands it to
  `ingest_zip_to_package` as a **real** filesystem path with no VFS resolution, so a
  game can name any zip the app process can read and then read its contents back
  through `/code`. Bounded by having to be a valid zip, and unrelated to the identity,
  but it is app-controlled input reaching the filesystem outside the sandbox. And
  `storage_dir` puts the storage backing under `/user`, which the game can write —
  self-harm only, since the data is its own, unlike the install record. Task 0.30,
  now done.

- [x] 0.30 Stop `op_install_subpackage` taking a path from the game. Found while
  fixing 0.29, and independent of it. `install_subpackage_blocking` built a
  `PathBuf` straight from the JS-supplied `zipPath` and handed it to
  `ingest_zip_to_package`, so the path was never resolved through `VirtualFS` and never
  checked against a sandbox root. A game could therefore name any zip the app process
  can read — including one belonging to the host app rather than to the game — and
  read its entries back through its own `/code` overlay once the ingest succeeded.

  What bounded it was only that the file has to parse as a zip, which is not a
  boundary anyone chose. On Android that leaves the app's own APK in range, which is a
  zip.

  **Resolved by removing the untrusted hop, not by validating the path.** The value is
  produced by the host, which is trusted; what made it unusable is that it reached the
  runtime *through the game's JS*, so the runtime could not tell it from one the game
  invented. Validating it would have kept a game-chosen path and merely constrained
  where the file may sit — and it would have needed the host contract changed to
  dictate a download directory. Instead the path stays where the runtime received it:
  `intercept_download_result` takes it out of the download payload before that payload
  reaches JS, and the install takes it back by request id. `InstallOptions` has no path
  field at all, so the op cannot be handed one.

  The store is keyed by `(session, request)`, so one game's request number cannot name
  another's download; entries are one-shot, so a second attempt on the same request
  finds nothing rather than re-ingesting a file the host may already have cleaned up;
  and a session's remaining entries are dropped at teardown, both from `Host::drop`
  and from the startup guard, because a path outlives the temp file it names. This is
  now stated as Section 6.6, as the general rule for host-produced file references
  rather than a fix for one op — `resolve_path_vfs` and the audio and image resolvers
  already refuse absolute paths from JS, and this op was the one exception.

  Six tests: the payload the game sees carries neither the field nor the path, one
  session's request number does not reach another's download, a recorded path is
  consumed by the install that takes it, a failed download records nothing and keeps
  its reason, teardown drops one session's paths and leaves another's, and the
  pre-fix payload shape — a zip path with no request — is rejected by the op's
  options. The first five were watched failing against a pass-through
  `intercept_download_result`, which is exactly the pre-fix behaviour: the failure
  message printed the host APK path arriving in the game's payload.

  Verified at migo-shared 386 passed, up from 381, migo-runtime-v8 505 with 2 ignored,
  up from 504, with migo-io 259, migo-core 49, migo-capi 141 and migo-platform 50 with
  1 ignored unchanged. **Also verified on the Android target**, which matters here
  because the interception lives in `cfg(android)` code that no host `cargo` run
  compiles: `scripts/build-android-so.sh arm64-v8a` builds `libmigo.so`.

  Not covered: the JS path is unexercised by a fixture, as with the rest of
  `04_subpackage.js`. The zip the host downloaded is still the host's to delete;
  nothing in the runtime removes it after ingest, which is unchanged and out of scope
  here.

- [ ] 0.31 Make a session's own verification cover the targets CI covers. Found by
  running `scripts/build-android-so.sh` for task 0.30's
  `cfg(android)` code: **the branch did not compile for Android**, and had not since
  `c6645bd`. That commit gave three JNI entry points a `jboolean` return — correctly,
  since Java declares `shutdown`, `onTouchEvent` and `updatePermission` as `boolean`
  and `profile_contract.rs` requires `Z` — but left their bodies as `jni_safe!(..);`
  statements, so each computed a verdict and discarded it, returning `()`. Three
  compile errors, one of them on the touch input path, one on session teardown, one on
  the permission gate: the three areas tasks 0.16, 0.18 and the host-bridge work
  touched.

  Fixed here as a prerequisite (the stray semicolons are gone and the target builds),
  but the finding that matters is the gap that hid it. `cargo check`, `cargo test` and
  `cargo clippy` on the host skip `cfg(android)` code entirely, so every session that
  reported those three commands as verification was structurally unable to observe
  this, and the ledger's earlier "verified" lines should be read as host-only.

  **Corrected before any work started: the CI gate already exists, so "add one" is not
  the task.** `pr-ci.yml`'s Android job runs `scripts/build-aar.sh --product-profile
  full debug arm64-v8a x86_64` and the slim equivalent, which builds `libmigo.so` and
  would have failed on all three errors, and it triggers on every pull request. What
  never happened is the *push*: this delivery branch is local by instruction, so no
  gate has run on any of it. The gap is therefore between a session's own verification
  loop and CI's, not a hole in CI.

  So what this task is: make the loop match the gate for the crates the host cannot
  build at all — `core`, `graphics`, `platform`, `capi` — rather than leaving it to
  whoever remembers. Two candidates worth comparing before choosing. A single local
  verification entry point that selects targets from what changed (host tests always;
  the NDK build when `crates/{platform,core,graphics,capi}` or any `cfg(` conditional
  path is touched) is the honest version, and `cargo ndk --target
  aarch64-linux-android --platform 26 -- build -p migo-platform` is the cheap slice of
  it — around a minute warm, versus the full AAR — because it compiles
  core + graphics + platform and links the cdylib, which is where JNI signature
  mistakes surface. The other candidate is leaving the loop alone and pushing early to
  a scratch branch so CI runs; that is not available while the branch is deliberately
  local, and it would also be slower feedback than a local build.

  Also worth settling here: `pr-ci.yml` excludes `core`, `graphics`, `capi` and
  `platform` from its `cargo test` line with a comment saying the AAR build is "their
  real gate". That is true for *compilation* and false for *tests* — those crates' unit
  tests (migo-core 49, migo-capi 141, migo-platform 50) run nowhere in CI, and this
  session ran them only because it runs them locally. Either the comment or the gate
  needs to change.

  **Implementation landed, reviews outstanding.** The first candidate was taken:
  `scripts/verify-change.sh` is one entry point that derives its targets from what
  changed, and `scripts/build-android-so.sh --compile-only` is the cheap slice it
  calls — 1m45s warm here, against the several minutes a full `.so` link costs. It
  builds `-p migo-capi`, which pulls core, graphics and platform, so one package
  selection covers all four crates; the contract asserts that closure with
  `cargo tree` rather than trusting the comment, because narrowing the selection
  would still print SUCCESS while covering less.

  Selecting targets by changed path alone would have been wrong, for two reasons
  found in the tree rather than reasoned about:

  * OpenHarmony is `target_os = "linux"` with `target_env = "ohos"`
    (`platform/src/lib.rs`), so a rule keyed on `target_os` reads all 17 ohos
    conditionals as ordinary host code.
  * A file selected by a conditional need not contain one.
    `capi/src/platform/windows.rs` is plain Rust; the `cfg` admitting it sits on its
    parent's `mod` declaration. So `scripts/lib/verification_targets.py` walks the
    module tree from each crate's `lib.rs` and inherits conditions downward, and it
    ignores polarity: a condition that *mentions* a non-host platform asks for that
    platform's build either way, since removing something a sibling branch referenced
    only fails on the target that compiles it.

  The walk is a regex parser over Rust, so its completeness is enforced instead of
  assumed: `--audit` reports every `crates/*/src/**.rs` no `mod` declaration reaches,
  the entry point refuses to run while any exists, and an unreached file is reported
  rather than assumed portable. That gate paid for itself immediately — it found
  `#[path]` resolved against the module's child directory instead of the declaring
  file's own directory, which is what the Rust reference specifies, and which had
  silently hidden `graphics/src/damage_tracker.rs`. The tree audits clean now.

  A target the loop cannot build is reported `NOT PROVEN` and fails the run. A skip
  there would reproduce the exact defect this item exists to prevent, so `ohos` and
  `windows` — which have no local build on this machine — make the run red rather
  than quiet. Running it over this branch is what produced task 0.32.

  Verified: shared 386, io 259 (5 ignored), runtime-v8 505 (2 ignored), graphics 519,
  core 49, capi 141, platform 50 (1 ignored) — 1909 passed, no failures;
  32 selector unit tests; 36 entry-point contract checks; `cargo fmt --all --check`
  and `git diff --check` clean; `--audit` clean. Android arm64-v8a compiled through
  `scripts/build-android-so.sh`, both the `--compile-only` slice (1m45s) and the full
  `libmigo.so` link (2m34s), since this item edits that script.

  Mutation-tested, ten mutants, all killed at their own assertions: dropping
  `target_env` handling fails the two OpenHarmony cases; not inheriting a condition
  into child modules fails the subtree case; resolving `#[path]` against the module
  directory fails its own case *and* makes the real-tree audit report
  `damage_tracker.rs` again; never reporting unreachable files fails two cases;
  making the link tier not supersede compile fails the absorption case; narrowing
  `--compile-only` to a package that misses platform fails three closure checks;
  removing either the audit gate or the unknown-condition gate fails its own case;
  dropping `-p migo-platform` from CI fails the cross-file check; and counting
  `NOT PROVEN` as a pass fails the ohos case.

  That last mutant is worth recording, because the first version of its test did not
  kill it. The assertion was on the exit code alone, and the fixture had no engine to
  build, so every host step failed and the run exited non-zero with or without the
  rule — provably unable to fail, the same defect the spec review withdrew task 5.1's
  first contention test for. The fixture now carries a real stub workspace whose host
  suites pass, so `NOT PROVEN` is the only thing that can fail that run, and a
  companion case asserts a clean change still passes so an always-red script cannot
  satisfy the rest.

  The CI half is settled by changing the gate, not the comment. A new
  `host-engine-tests` job installs the seven packages the build actually asks for —
  each one traced to a `cargo:rustc-link-lib` line or an include the headers need,
  not guessed — and runs the four suites. It is its own job because host Skia builds
  from source (the feature set has no matching prebuilt) and that does not fit
  quality-gate's 20-minute budget; it runs in parallel with the 90-minute Android
  job, so the wall clock is unchanged. It refuses the Rust cache for the same reason
  the Android job does. The stale comment is rewritten to say what is and is not
  covered, including that clippy for those four crates still ran nowhere — task 0.33,
  since closed, which put their clippy in this same job.
  `scripts/test-local-verification-contract.sh` now also asserts that every crate the
  local loop tests appears in a `pr-ci.yml` test line, so the two lists cannot drift
  apart again, which is how the four crates went missing in the first place.

  **What is not proven: the new job has never run.** The branch is local by
  instruction, so no CI change on it has been observed executing, and this one is
  read as reviewed-but-unexecuted until someone pushes. Its risk is concentrated in
  the apt list and the runner image, not in the commands, which are the same ones
  that pass here.

- [!] 0.32 Compile this branch's OpenHarmony and Windows conditional code for its
  own target. Found by running task 0.31's entry point over the branch: it reports
  `TARGET ohos compile` for nine files and `TARGET windows compile` for five,
  including `capi/src/platform/{ohos,windows,mod,unsupported}.rs` and
  `platform/src/{ohos,windows}/presenter.rs`, and neither target has ever compiled
  on this machine — `engine/target` has no directory for either. Section 7.4 is
  explicit that such a path is unverified until its own target compiles, so every
  earlier "verified" line on this branch covers the portable tree and Android, and
  nothing about the other two platforms.

  Nor does CI cover them. `pr-ci.yml` has no Windows or OpenHarmony job at all.
  `c-abi-candidate.yml` does have a `windows-latest` lane, but it is path-filtered to
  `include/migo/**`, `tests/c_abi/**` and `engine/crates/capi-abi/**`, and
  `capi-abi` is the crate deliberately built to have no dependencies — so it proves
  nothing about `cfg(target_os = "windows")` code in `capi` or `platform`. Nothing
  anywhere builds for OpenHarmony.

  **Blocked, and not on effort.** OpenHarmony needs the SDK's target-prefixed clang
  on `PATH` (`scripts/dev-setup-ohos.sh`) plus a Skia built for
  `aarch64-unknown-linux-ohos`; `cargo check` does not avoid that, because it still
  runs `skia-bindings`' build script. Windows needs a Windows machine: adding
  `x86_64-pc-windows-msvc` to `engine/rust-toolchain.toml` would supply std, but the
  same build script then has to produce an MSVC Skia, which does not cross-compile
  from Linux. Unblocking is therefore: install the OpenHarmony SDK on this machine
  for the ohos half, and add a `windows-latest` job that builds `-p migo-capi` for
  the windows half. Until then the entry point will keep reporting both as
  `NOT PROVEN`, which is the correct reading and not noise to be silenced.

- [x] 0.33 Run clippy on graphics, core, capi and platform somewhere. Found while
  correcting `pr-ci.yml`'s stale comment for task 0.31. The new `host-engine-tests`
  job now has the system packages those crates need, so the missing coverage is one
  step, but it is a separate claim from the tests and pinning 1.95.0 surfaced a
  pre-existing lint backlog the workspace caps to `warn` — so this needs the same
  cap and its own verification rather than a line appended to a green job.

  **Run locally before being added, which is what the task asked for.** `cargo
  clippy -p migo-graphics -p migo-core -p migo-capi -p migo-platform -p migo-audio
  --all-targets -- --cap-lints warn` exits 0 against this tree and reports 610
  warnings, all genuine clippy lints (`collapsible_if`, `too_many_arguments`,
  `manual_is_multiple_of`, …), so the step reports rather than passing empty and
  the cap is doing what it does in quality-gate rather than hiding a failure.

  **audio is in the step even though the task named four crates.** Its clippy ran
  nowhere for exactly the same reason theirs did — it needs ALSA, which only this
  job installs — and it was added to this job's *tests* last round. Leaving it out
  would have reopened the gap in the same commit that closed it.

  **The toolchain step needed a change too, and it is the kind that fails
  confusingly.** `dtolnay/rust-toolchain` installs a minimal profile, so this job
  had no clippy driver at all; without `components: clippy` the step fails to find
  the binary rather than reporting on the code.

  **Closed as a class rather than as an instance.** The reason those four crates
  had no clippy is the reason they had no tests: two lists in one file and nothing
  comparing them. `test-local-verification-contract.sh` now asserts that every
  crate CI *tests* is also a crate CI *lints* — across both jobs, since the split
  is deliberate — so a crate added to a test line without a clippy line fails there
  instead of going unlinted for months. Contract checks 40 → 52.

  Mutation-proved, and the first attempt was wrong in a way worth recording:
  dropping `-p migo-graphics` from the clippy line fails the guard at its own
  `pr-ci.yml lints migo-graphics too` assertion; deleting every clippy invocation
  fails at `cannot find the crates pr-ci.yml runs clippy on`. That second mutant
  initially made the script **exit silently with no failure at all** — under
  `set -euo pipefail` a `grep` matching nothing kills the script before the empty
  check can report it, so the anti-vacuity branch was unreachable. It needed
  `|| true`, which the neighbouring `selection=` line already had for the same
  reason.

- [x] 0.35 Build the shared-executor occupancy gate Section 7.3 now requires, then
  apply it. Found while doing 0.19 step 3's audio item, which the plan had deliberately
  left open for want of exactly this: "no CPU-bound work on the shared worker" is a
  claim about which executor a call runs on, and neither the allocation gate (0.26) nor
  the contention gate (0.27) can see it. `engine/testing/executor-probe` is the third
  mechanism, and 0.19 step 3 holds the design, the two properties its own controls
  found, and the mutation evidence. Applied to the streaming decode, which it moved off
  the shared worker.

  Still uncovered by it, and named rather than implied: the shared IO executor's
  per-host fairness, which Section 6.4 records as enforced by reading.

  **Closed under task 0.37, and not by this probe.** Reading the executor first is
  what settled it: it is a fixed set of OS worker threads behind a condvar, not a
  tokio runtime, and the property is fairness under contention rather than
  occupancy by CPU work — so the probe's shape does not fit and forcing it would
  have produced a gate on the wrong question.

- [x] 0.34 Measure `io::image_cache`'s per-frame paths with the allocation gate.
  **Measured, and the prediction was half right**, which is the half worth recording:
  the pin/unpin pair allocated exactly as expected and the lookup did not.

  Two gates, named separately because they are different events and a combined burst
  could not say which of the three calls reached the heap. The lookup gate —
  `get` on a hit, including the frequency increment, the per-Session counters and the
  per-owner attribution task 0.16 left unmeasured — **passed without any fix**, so
  the `Vec` scan that note was uneasy about costs nothing. The pin/unpin gate failed
  against the unfixed cache at its own assertion with the counts the defect predicts:
  64 fresh allocations and 64 frees over 64 iterations, 704 bytes, which is one
  `String` clone of the key per pin and one free per unpin.

  **The fix is the text cache's, but it is not the text cache's change**, and reading
  it as one would have regressed this cache. `pin` here must accept a key that is not
  resident: an alias is established before its decode finishes, so a pin routinely
  arrives before the bytes (`begin_load` → decode → `insert`), which is what
  `pin_absent_key_is_honoured_on_later_insert` has always pinned down. Moving the
  count wholesale onto the entry would have dropped those. So the count lives on
  `CachedImage` where the entry exists, and a `reservations` table holds exactly the
  counts an entry cannot — one adoption point in `insert`, one hand-back point, and a
  key never in both homes.

  The hand-back exists because `LruCache`'s **entry** cap is inside the crate and
  cannot be taught about pins, so it can displace an entry a live alias is holding.
  The old parallel map survived that for free; the new one has to put the count back
  where a non-resident key's count lives, or a re-decode would come back unpinned and
  the alias's next upload could find no bytes. Every other eviction route — byte
  budget, trim, clear, `clear_for_session` — already refuses pinned entries and now
  reads the field instead of a second hash lookup.

  Mutation-proved, five mutants:
  the pre-fix code itself is the first, and it is the TDD red state rather than a
  synthetic one — it fails the pin/unpin gate with the counts above and nothing else;
  a narrow one that keeps the count on the entry and merely builds an owned key
  beside it fails the same gate alone, so the gate is sensitive to *that call* and
  not to the surrounding shape; dropping the hand-back fails the entry-cap test at
  its "the alias is still live" assertion and nothing else; adopting a reservation
  without consuming it fails both single-home assertions; and making `add_owner` push
  unconditionally fails the lookup gate — with four `realloc` events, which is the
  resize the probe counts precisely so a `with_capacity` becoming a `new` cannot
  slip through — proving that gate is not a permanent pass.

  **One mutant was not killed by the first version of the tests, and that gap was
  real rather than cosmetic.** Handing pins back on a *same-key* replacement as well
  leaves the count in both homes: `insert` has already adopted it onto the
  replacement, so the entry and the table each claim it, no unpin can ever retire the
  table's copy, and the key is pinned forever the next time it is re-inserted. The
  single-home test now re-decodes a key that is resident **and** pinned — two Sessions
  finishing a decode of one shared image, which this cache exists to allow — and the
  mutant fails there.

  **Found while measuring, not fixed here, and named rather than implied:**
  `resolve_cached_image_rgba` builds an owned key **twice** before it reaches this
  cache — once cloning out of the alias map, once in `to_io_cache_key` — on
  `op_tex_image_2d_from_image`'s CPU-bytes path. It is in `runtime-v8`, above the
  cache this task scopes, and it is the slow path: the ordinary draw takes the
  GPU-side copy and never reaches it. Recorded as task 0.36 rather than absorbed
  here, because how often that path is really taken is a question this task did not
  answer.

  Found by applying task 0.26's gate to the text texture cache, which was built by
  copying this cache's shape — its own doc said "identical to the `io::image_cache`
  pattern". In the text cache that shape cost two `String` allocations and two frees
  on **every** `fillText` hit, because a pin lasts one frame and a
  `HashMap<Key, u32>` beside the LRU needs an owned key to record it. The image
  cache still keys pins the same way, and `op_*` image lookups pin per draw
  (`crates/runtime-v8/src/rendering/image/cache.rs` pins on hit,
  `crates/io/src/image_ops.rs` pins before decode), so the same per-event cost is
  likely present on the hotter path.

  Also unmeasured here, and named in task 0.16's notes: the per-owner attribution
  `get` performs against a one- or two-element `Vec`.

  Concretely: add `migo-alloc-probe` to `migo-io`'s `[dev-dependencies]`, declare the
  counting allocator under `#[cfg(test)]` in its `lib.rs`, and write a burst over
  lookup-plus-pin-plus-unpin at steady state. If it allocates, the fix is the one the
  text cache took — the count belongs on the entry — and it must land with its gate,
  not as a note.

- [x] 0.37 Gate the shared IO executor's per-host fairness. Section 6.4 lists it
  among the properties "already enforced" and Section 7.3 recorded it as the only
  one of those with no gate named against it — the last item task 0.35's probe
  left open.

  **The probe does not fit, and reading the executor is what established that
  rather than an attempt to force it.** `engine/testing/executor-probe` spawns onto
  a tokio runtime and asks whether a step occupies a shared worker with CPU-bound
  work. The IO executor is neither: it is a fixed set of OS worker threads behind a
  condvar (`crates/io/src/pools.rs`), with a round-robin lane per host and
  `host_cap_when_contended` bounding how many workers one host may hold while
  another has work queued. The property is *fairness under contention*, not
  occupancy. Applying the probe here would have gated the wrong question.

  What was built instead shares no code with the two probes and every principle:
  manufacture the adversarial condition rather than wait for it. One host fills all
  four workers and keeps two more queued, the neighbour submits one job, and
  exactly one worker is freed — which must go to the neighbour, because the flooder
  then holds three against a contended cap of two. Three load-bearing details, two
  of them the probes' lessons restated: the flooding jobs are released by the test
  alone and share no deadline with the neighbour's wait, or a timeout would free
  the very worker the neighbour was waiting for and an unfair executor would pass;
  saturation is asserted rather than assumed, since a neighbour handed an idle
  worker observes nothing; and one permit is released rather than a broadcast,
  because freeing every worker asks the dispatcher nothing.

  **The duplication question was asked before the test was kept, not after.**
  `QueueState`'s own tests already drive this policy directly, so a second gate had
  to pin something they cannot. It does: giving every submitted job one host token
  — the plumbing between a registration and the queue — fails only the new test,
  because a test that pushes tokens by hand never traverses `submit` → worker →
  dispatch. Removing the cap fails both, which is one policy seen at two levels
  rather than two guards on one case.

  **A third mutant is recorded rather than counted.** Making a completion stop
  releasing its host and class slots deadlocks the executor's own shutdown —
  workers park holding pending work that can never dispatch, and `close` cannot
  drain them — so the suite hangs instead of reporting. No test can report on that
  mutant, this one included.

- [x] 0.36 Remove `texSubImage2D(image)`'s two owned keys per call. Found while
  measuring task 0.34: `resolve_cached_image_rgba` cloned the key out of the alias
  map and then `to_io_cache_key` built a second owned copy from it, so the call
  allocated twice before the decoded-image cache — which now allocates nothing —
  was reached.

  **The hotness question was answered by structure rather than by measurement:
  there is a second caller with no fast path at all.** Task 0.34 recorded this as
  open on the assumption that `op_tex_image_2d_from_image` was the only way in, and
  for that op the reading was right — it takes a GPU-side copy whenever the alias
  has a live shared texture, and `shared_for_image_id` misses only in narrow states
  (a load still in flight, or an alias whose entry is gone), so the CPU-bytes branch
  really is a fallback. But `op_tex_sub_image_2d_from_image` calls
  `resolve_cached_image_rgba` **unconditionally**: there is no
  `TexSubImage2DFromShared` command and no branch above it. Every
  `texSubImage2D(…, image)` paid both owned keys, every call, with no cheaper route
  available to the content, and it does not depend on which game is running.

  **The gate came first and failed with the counts the defect predicts**: 128 heap
  allocation events over 64 measured iterations — 128 fresh, 128 releases, 4352
  bytes — which is exactly two owned keys per call and nothing else. `migo-io`
  already had the counting allocator; `migo-runtime-v8` did not, so it gained the
  `[dev-dependencies]` entry and the `#[cfg(test)]` `#[global_allocator]`. No CI
  list needed editing: `migo-runtime-v8` was already on both the test and the
  clippy line, which is what the two-list contract exists to keep true.

  **The fix deletes the conversion rather than making it cheaper.** The two owned
  keys were one defect wearing two faces: the alias table keyed on
  `(path\0WxH, generation)` and the cache below it on `(path, generation, w, h)`,
  so the boundary had to be crossed by rebuilding. `cache::ImageCacheKey` is now
  *the same type* as `migo_io::image_cache::ImageCacheKey` — the mangling, the
  parse and `to_io_cache_key` are gone from eleven call sites, not just from this
  one — and `cache_key_for_image_id` hands back a borrow, so the lookup copies
  nothing. Making the two sides share one type also puts an encoding drift between
  them beyond writing, which the mangled form could not.

  **Lock order argued, not assumed, and the argument is that it cannot be
  inverted.** The borrow lives only while the alias lock is held, so the lookup now
  nests this Session's alias mutex outside the process-wide decoded-bytes mutex.
  That is the order this code already took everywhere — every `pin`/`unpin` in
  `ImageCache` runs under the alias lock, and `ImageCache::drain` holds an io guard
  inside one — and the reverse is unwritable rather than merely absent: `migo-io`
  does not depend on `migo-runtime-v8`, so no code holding the io lock can reach an
  alias table. Searched rather than recalled: every `global_cache()` acquisition
  outside `migo-io` itself is either a statement temporary or the one inside
  `drain`. No new cross-session lock either — this path already took the shared
  decoded-bytes lock, which Section 6.5 puts in the deliberately-shared tier.

  **Two mutants, each changing exactly one property, each killing exactly one
  test.** A single `key.clone()` at the lookup fails the burst gate alone with 64
  events over 64 iterations — one added allocation per call, so the gate is
  sensitive to *that* call and not to the shape around it. Making `make_cache_key`
  ignore the requested size fails
  `each_requested_size_gets_a_cache_slot_of_its_own` alone, at its own assertion.
  That second guard earns its place: the size used to ride in the path and now
  rides in the key's own fields, so a key built without them is still well formed,
  and every other test in that file uses full-resolution keys — the mutant proves
  nothing else was covering it, including
  `resized_rgba_cache_key_uses_decoded_source_generation`, which stayed green.

  **Not done, and named rather than implied.** `remove_previous_alias` and
  `try_release_and_get_destroy_rid` still clone the key out of `shared_to_key`,
  because they remove from that same map afterwards and the borrow cannot outlive
  it. Those run per `img.src =` and per destroy, not per frame; unifying the key
  type halved them for free, and no gate claims them. The six V8 snapshots were
  already stale on this branch before this change and remain so — the fingerprint
  covers every `runtime-v8` `.rs` file, so they are regenerated once for the
  branch rather than once per task.

- [x] 0.38 Gate the render command path's steady-state allocation. Section 7.3
  listed it as unmeasured and it is the highest-rate event the engine handles —
  one `gl.*` call from content becomes one command in the collector, and Cocos and
  three.js emit hundreds per frame.

  **Two gates, not one, because they are different calls.** `push_gl_fast` is the
  per-command enqueue; `append_gl_batch` is what `op_gl_submit_stream` reaches
  after decoding a stream, which is the path a Pixi frame actually takes. A burst
  covering both could not say which reached the heap — the lesson task 0.34 paid
  for. **Both passed against unmodified code**, so the collector and the command
  vector pool were already doing their job on the per-event path; that is a result
  rather than a non-event, and the mutants below are what make it one.

  **The reservation is established locally rather than by cycling frames through
  the pool, and the reason is recorded in the fixture.** The pool is
  process-global and `cargo test` runs this binary concurrently, so a gate that
  depended on getting *its own* recycled vector back would fail whenever another
  test took it first. A flaky gate is worse than none. The pool's reuse property
  is already covered where it can be deterministic — against a private instance,
  in `command_vec_pool::tests::recycled_vector_reuses_its_allocation`.

  **One measured defect, fixed here.** `build_frame_packet_inner` took the segment
  list with `std::mem::take`, which hands away both the segments *and* the vector
  holding them and leaves it at zero capacity — so the first push of every frame
  allocated it again, forever, on the thread running the game. `drain(..)` moves
  the segments out and keeps the allocation. Measured over ten frames after
  warm-up: **two allocations and two frees per frame became one and one**, at any
  frame size up to the pool's retention ceiling.

  **Mutation-proved, three mutants.** Dropping `append_gl_batch`'s recycle instead
  of returning the vector fails the append gate alone with exactly one allocation
  per event (64 over 64, 90112 bytes) — and **no other test in the binary noticed
  a leaked pool vector**. Reverting `drain` to `mem::take` fails the segment-list
  test alone. Making every command open a segment of its own fails both burst
  gates, at the fixture's own precondition rather than at the burst — and nothing
  else in the binary, so a segment per command is otherwise invisible.

  **That third mutant is also why the fixture has a bound.** Its first version
  looped until the segment held enough reserved slots, which under that mutant can
  never happen: the run allocated until the kernel killed it (exit 137), so the
  mutant was reported as a hung suite rather than as a failure. A fixture that
  cannot establish its precondition has to say so. It now stops and names the
  invariant that broke, which is what turned an unkillable mutant into a killed
  one.

  **Verified.** runtime-v8 514 from a 511 baseline; shared 392, io 264/5, graphics
  523, core 51, capi 142, platform 50/1, audio 48 unchanged; python CI 117; both
  contract scripts; clippy clean on the touched crate — the one warning this
  change introduced was a constant assertion, and clippy's suggestion was taken
  rather than silenced, so the fixture's precondition is now a compile-time check.
  `scripts/verify-change.sh --base HEAD` reports every host step PASS and requires
  no target build: the changed file carries no conditional, which the selector
  confirms rather than assumes.

  **Amended: the append gate was pool-contention-flaky, and fixing it cost a
  mutant.** Its first version called `take_gl_command_vec` inside the measured
  window, reasoning that it recycled a vector every iteration and would get one
  back. That reasoning fails under concurrency: a concurrent test taking the
  pool's last vector leaves `take` with nothing to hand back, so it allocates a
  fresh minimum-capacity one. It passed standalone and failed under
  `verify-change`'s load with a single 1408-byte allocation — exactly
  `GL_COMMAND_VEC_INITIAL_CAPACITY * size_of::<GLCmd>()`. The batch vectors now
  come from a reservoir built before the burst, which is the same rule
  `reserve_gl_segment_headroom` already carried and which this gate should have
  had from the start.

  The cost is recorded rather than glossed: with the pool out of the measured
  window, **the "stops recycling" mutant no longer fails anything** — a dropped
  vector is a deallocation, and the burst counts allocations. Re-checked against
  the final code: 514 passed, nothing failed. What still kills this gate is the
  segment-per-command mutant, via the fixture's precondition. The recycle
  obligation itself is now unguarded, which is task 0.41.

- [x] 0.39 Bound the command-vector pool by the bytes it retains. Task 0.38
  measured the old rule as a cliff rather than a gradient: `MAX_RECYCLABLE_COMMAND_CAPACITY`
  capped a *single* vector at 512 elements, so a frame one command past it had its
  vector dropped and regrew from the 16-element minimum on every subsequent frame.

  | commands per frame | allocations | reallocations | bytes |
  | --- | --- | --- | --- |
  | 512 | 1 | 0 | 224 |
  | 513 | 2 | 6 | 179 040 |

  799x the bytes for one more command, about 10.5 MB/s of copying at 60 Hz, on the
  thread CLAUDE.md Section 7 identifies as the bottleneck.

  **Three defects in one constant, which is why raising it was not the fix.** It
  was in the wrong unit — elements, so the same number meant 44 KiB of `GLCmd` and
  a different amount of `Canvas2DCmd`. It bounded the wrong quantity — one vector,
  not the pool, so it never actually bounded memory: 16 slots times 512 elements
  was already permitted. And it was a cliff, which a retention rule should not be.
  Raising the number moves the cliff; shrinking an oversized vector before
  recycling trades six reallocations for one and loses the capacity anyway.

  So the pool now bounds exactly what the ceiling was protecting: its own retained
  bytes, tracked across `take` and `recycle`. **The permitted worst case is
  unchanged by construction** — the budget is `slots * commands_per_slot *
  size_of::<T>()`, the same arithmetic the old rule already allowed — but the
  allowance is now spent on whatever shape the workload has, one large vector or
  sixteen small ones, and no frame size is special.

  **The unit hazard is designed out rather than documented.** The budget parameter
  stays in *commands* and the pool converts to bytes itself, because a `usize`
  byte budget beside a `usize` command count is a mistake the compiler cannot
  catch — and the first draft of this change made exactly that mistake, handing a
  byte value to the element parameter and getting a green test that proved
  nothing.

  **Mutation-proved, four mutants, and the fourth changed the design.** Removing
  the budget check fails both budget tests. Dropping the reservation rollback on a
  full channel fails the accounting test. Dropping it in `take` fails the same
  test. Removing the separate `bytes > budget` early refusal **killed nothing** —
  so it was a branch that read like a guard while changing no outcome, and it is
  gone; the one budget check turns away the pathological vector too, since it is
  over budget from an empty pool.

  **One mutant survived the first version of the accounting test, and the test was
  wrong rather than the code.** Its "refused by a full pool" case was really being
  refused by the budget, so the `try_send` rollback it claimed to cover was never
  reached. The fixture now separates the two limits — one slot, budget to spare —
  and asserts which limit does the refusing.

  **Verified.** shared 395 from 392; io 264/5, runtime-v8 514, graphics 523, core
  51, capi 142, platform 50/1, audio 48 unchanged; `scripts/verify-change.sh
  --base HEAD` every host step PASS.

  **A tooling failure worth recording, because it nearly cost the work.** The
  mutant-revert helper applied `str.replace(new, old)` with `new` empty when the
  mutant was a deletion, and in Python `"abc".replace("", X)` inserts `X` between
  every character — it wrote 12 505 copies of the block into the file, 2.7 MB. The
  corruption was deterministic and exactly reversible (remove every occurrence of
  the inserted block, which also removed the one real copy, which was the intent),
  and the recovered file was confirmed by compiling and by re-running every mutant
  against it. The helper now refuses an empty anchor or replacement, and reverts by
  restoring a verified copy rather than by inverse substitution.

  **Amended: one existing test was reading the pool's contents and had to be
  re-derived.** `interleaved_single_command_segments_do_not_reserve_256_slots_each`
  summed the capacities of 100 interleaved segments against a fixed threshold. Those
  capacities are not the collector's decision — each segment takes a vector from the
  process-wide pool, so the sum depends on what every other test in the binary
  recycled. The old per-vector length cap had been bounding that measurement by
  accident; once retention went by bytes, a single-command segment could inherit a
  larger recycled vector and the sum crossed the threshold. **The aggregate bound
  did not move** — it is the pool's budget either way — only the distribution did.

  The threshold is now derived from the pool's own rule (its byte budget, plus a
  fresh minimum for every segment beyond what the pool can supply), which is a true
  upper bound whatever the pool holds. It keeps its teeth: reinstating the 256-slot
  policy the test is named for still fails it, at 1 843 200 bytes against a
  1 272 448 ceiling.

- [x] 0.43 Gate the audio graph's real-time path. Section 7.3 listed the audio
  path as unmeasured, and it is the last steady hot path a host test binary can
  reach — the BLE halves need an on-target binary and a JVM mechanism.

  **Different stakes from the frame path, which is why it was worth doing even
  though it passed.** `AudioContext::process` is the output callback's work: once
  per quantum, on a thread `audio_thread.rs` runs under SCHED_FIFO on Android. An
  allocation there is not a throughput cost, it is a deadline miss — the allocator
  can block behind a thread that is not real-time scheduled at all, and the result
  is an audible dropout rather than a slower frame.

  **It passed against unmodified code**: the mix buffer is a reused field, node
  output buffers are created on first use, and the finished-node list is a
  `Vec::new()` that only allocates when a source actually ends. Recorded as a
  result rather than a non-event, because two mutants show the gate can fail.

  **Mutation-proved, two mutants, both killing this gate alone.** Replacing the
  reused mix buffer with a per-quantum `vec!` fails with 192 events and 196 608
  bytes over 64 quanta. The one that matters more is smaller and looks like an
  improvement: giving the finished-node list `Vec::with_capacity(self.nodes.len())`
  instead of `Vec::new()` costs exactly one allocation per quantum — 64 over 64,
  768 bytes — on the real-time thread, for a list that is almost always empty.
  That is the shape of regression this gate exists to catch, and nothing else in
  the crate would have.

  **Scope stated rather than implied.** What is measured is the graph render for a
  source-into-gain-into-destination chain, which is what every buffer-playback
  plus volume does. The rest of the audio subsystem — `audio_thread`'s scheduling,
  `output`'s device handoff, `streaming`'s refill — is still unmeasured, so the
  audio path is not recorded as satisfied, only this part of it.

  **Verified.** migo-audio 49 from a 48 baseline; every other crate unchanged;
  clippy clean; `scripts/verify-change.sh --base HEAD` every host step PASS.
  `migo-audio` was already on both the CI test and clippy lists, so wiring the
  probe into it needed no list change — the two-list contract stayed satisfied
  without editing it.

- [x] 0.44 Gate the texture upload against the per-session registry lock. Section
  7.3 has two requirements per covered path, not one, and task 0.36 changed the
  second: `resolve_cached_image_rgba` now holds this Session's alias lock across
  the shared decoded-bytes lock. Checking whether that path had a contention gate
  at all is what found that it never did.

  **The invariant was a comment.** `SESSION_IMAGE_CACHES` maps every live Session
  to its alias table, and the design says per-event paths never consult it — they
  hold the `Arc` resolved once at isolate bring-up. A comment cannot fail when
  someone adds a lookup, and the lookup would *work*: it returns the right table,
  just after queueing behind every other Session's bring-up and teardown, on a path
  that runs per `texSubImage2D`. That is precisely the failure Section 7.3 asks for
  a test — not an argument — to rule out.

  Wired the way `shared::stats` already does it: a `#[cfg(test)]` accessor hands
  the registry's own lock to the probe, so no shipped build can reach past the
  handle it resolved at bring-up.

  **Mutation-proved.** Replacing the held handle with `image_cache_for_host(id)`
  inside the probed body — the exact regression the comment forbids — fails this
  gate and nothing else, and it fails by *blocking*: the run takes 3.28 s against
  the probe's 2 s patience, which distinguishes a real block from a failure for
  some other reason.

  **Also settled while here: the lock order 0.36 introduced is not a cross-session
  hazard.** The alias mutex is per Session and reached only by that Session's own
  isolates, so holding it across the shared io lock lengthens one Session's own
  critical section, never another's. The shared lock on this path is the
  decoded-bytes cache, which Section 6.5 puts in the deliberately-shared tier and
  which the path already took before 0.36.

  **Verified.** runtime-v8 515 from 514; every other crate unchanged;
  `scripts/verify-change.sh --base HEAD` every host step PASS.

- [x] 0.45 Stop `console.log` taking the process-wide console registry lock.
  Found by sweeping every per-session registry behind a shared lock after task
  0.44, asking of each whether a per-event path consults it. Most came back clean:
  `HOST_SENDERS` was already gated, `SESSION_CACHES` and `STATS` by tasks 0.16 and
  0.27, and `VSYNC_SENDERS` resolves once into the host handle — its
  "intentionally a cold attach-time operation" comment is true and the structure
  enforces it. This one was not.

  `push_console_log(id, ..)` looked the session up in the process-wide
  `CONSOLE_LOGS` map on **every `console.log` the content makes**. The lookup
  *works* — it returns the right buffer — which is exactly why only a gate catches
  it: what it costs is a queue behind an unrelated game's bring-up or teardown,
  which take the same lock for writing, on a path a game can reach every frame.

  **The ordering question that made this a decision rather than an edit was
  checked in the source, not assumed.** The buffer is registered only when debug
  is enabled, and the restart path builds a new isolate — so a bring-up-time
  resolve is only safe if registration is already final both times. It is:
  registration runs in the host's pre-JS services ahead of `HostJsRuntime::new` on
  the first start, and a restart does not unregister, so the answer is settled
  before `bind_thread_console` runs either time. Teardown unregisters, by which
  point the isolate is gone. So the resolve-at-bring-up option is correct, and it
  is the same shape the text texture cache and the image alias table already use.

  The op now holds the buffer it resolved at bring-up in a thread-local, where it
  previously held only the session id. In production, where debug is off, that
  turns a lock acquisition plus a hash lookup per log call into one branch on
  `None`.

  **Mutation-proved, and the mutant is the previous production code.** Restoring
  the per-call `push_console_log` fails this gate and nothing else, by *blocking*:
  3.16 s against the probe's 2 s patience.

  **The fixture was wrong first, and the failure said so.** Its first version
  bound the sink inside the probe body — but binding is what reads the registry,
  so it deadlocked against the held write guard and reported at exactly 2.00 s.
  Production resolves once at bring-up, outside any contention, and writes many
  times after; the fixture now does the same, which is why the resolve and the
  install are separate functions.

  **Containment of the test-only seam checked rather than asserted, and a guard
  deliberately not added.** Both this task and 0.44 add a `pub fn` handing out a
  process-global lock behind `feature = "contention-probe"`, under a comment
  claiming no shipped build enables it. Cargo feature unification can make that
  claim false, so it was tested: a `compile_error!` tripwire on
  `all(feature = "contention-probe", not(test))` **compiles clean** through normal
  builds of `migo-core` and `migo-runtime-v8`, and fires when the feature is forced
  on, so it is not vacuous. Resolver 2 keeps dev-dependency features out of normal
  builds and the claim holds.

  The tripwire was removed rather than kept, because it cannot be made permanent:
  it fires during `cargo test -p migo-runtime-v8` too, since `shared` is built
  there as an ordinary dependency of the test binary rather than under `cfg(test)`,
  so a compile-time check cannot tell a dev-dependency enabling it from a shipping
  build enabling it. The only mechanism left is a manifest-parsing contract script,
  and the harm it would guard against is a `pub fn` returning a lock reference in a
  crate that is not a third-party API surface — real hygiene, no correctness or
  security consequence. Recorded as verified-not-gated so the next reader knows the
  claim was measured and why no gate backs it, rather than finding a comment and
  having to repeat the work.

- [x] 0.41 Guard the batched submit's obligation to return its vector to the
  pool. Task 0.38 proved the gap twice over: dropping `append_gl_batch`'s
  `recycle_gl_command_vec` instead of returning the emptied vector **failed no
  test in the binary** — the burst gate caught it only while it took from the
  pool, and that dependency had to go because it made the gate flaky under load.

  A leaked pool vector is a deallocation, not an allocation, so the burst
  mechanism cannot see it by construction. The two obvious replacements were both
  unusable as written: asserting on the shared pool's contents is the flakiness
  that was just removed, and counting deallocations across the burst is defeated
  by the pool legitimately refusing a recycle once it is full. This task's stated
  job was to find an observation point that does not depend on the global pool's
  state, before writing a guard.

  **There is no such observation point, and the answer is that there does not
  need to be one.** The obligation was removed instead of guarded.
  `shared::command_vec_pool::PooledVec<T>` is a command vector on loan from its
  pool, and its `Drop` returns it. `take_gl_command_vec` and
  `take_canvas_command_vec` hand one out, `GlBatchPayload::commands` and
  `CanvasBatchPayload::commands` hold one, and the free `recycle_*` functions are
  deleted. `append_gl_batch` takes a loan and finishes two of its three paths
  holding an emptied one; both used to call the pool by hand, and neither does
  now.

  Design points, each load-bearing:

  - **The pool is chosen by the element type, not carried in the vector.** A
    `Pooled` trait names the one pool per element type, so a `PooledVec<T>`
    occupies exactly what a `Vec<T>` occupies — asserted, because these vectors
    live inside `FrameOp`, which is itself held by a vector of the same kind, and
    a back-pointer would be paid once per batch and once per packet.
  - **`Drop` empties before offering.** The pool's `recycle` refuses a non-empty
    vector, which is what stops a caller parking live commands in it; a loan a
    consumer stopped reading part-way through still owns an allocation worth
    keeping, and dropping the remainder is what dropping the vector would do
    anyway. The old path threw that allocation away.
  - **Consuming iteration returns the buffer.** `std::vec::IntoIter` owns and
    frees it, which would defeat the pool on every `for op in packet.into_ops()`.
    Reversing once and popping from the back is the same order in O(n), with no
    unsafe and no second allocation, and leaves the emptied vector for `Drop` —
    including when the loop breaks early.
  - **`Deref`/`DerefMut` reach the `Vec`**, so the thirty-odd call sites read as
    they did. That leaves `mem::take` able to steal a loan, deliberately: the
    failure being removed is *forgetting* to return one.

  **Mutation evidence.** The mutant this task was written for — delete the
  recycle call — no longer exists to write. Its nearest expressible form,
  `std::mem::forget(commands)` inside `append_gl_batch`, **survives**, and that is
  recorded rather than papered over: nothing catches a deliberate leak, and the
  type's claim is only about omission. What is killed:

  - A `Drop` that hands the vector over without emptying it: three
    `command_vec_pool` tests, at their own assertions.
  - Consuming iteration without the reversal: the ordering test alone.
  - `FramePacketBuilder` back to `Vec::new()`: the frame-cycle gate from task
    0.40, at 128 allocation events over 64 frames.
  - A `Drop` that frees instead of returning: the same gate, at 192 events over
    64 frames — the three loans a frame holds.

  **Verified.** migo-shared 400 lib from 395 plus a new 1-test integration
  binary, migo-graphics 535 lib from 523, migo-runtime-v8 516, migo-core 51,
  migo-io 264, migo-capi 142, migo-platform 50, migo-audio 49, the input
  transport contract PASS, `cargo fmt --all --check` and `git diff --check`
  clean, and clippy at the two CI invocations exits 0 with no new lint in a
  touched file. `scripts/verify-change.sh --base HEAD` reports **verified for
  every target this change touches**, including
  `PASS android compile bash scripts/build-android-so.sh --compile-only arm64-v8a`
  — required, because the change reaches `crates/graphics` and `crates/core`.

- [x] 0.46 Make `scripts/verify-change.sh` able to pass. Found while verifying
  task 0.40: four of its fourteen host steps failed — `cargo build --workspace`
  and the `migo-graphics`, `migo-core`, `migo-capi` and `migo-platform` suites —
  and re-running it against a stashed, **untouched** tree produced the same four
  failures. They link Skia, which a minimal Linux host cannot build with a bare
  `cargo`: it needs the system clang rather than the NDK's, the Khronos headers,
  and the linux-gnu V8 archive.

  This is the worst state a verifier can be in. It is not a false negative about
  one change; it reports the same red whatever the change did, so the only thing
  it can teach a reader is to stop reading it — and Section 7.4 makes this script
  the thing that produces the sentence "any change touching conditional code
  names the target build that compiled it". A sentence nobody trusts does not get
  written.

  The three prerequisites are already established by `scripts/dev-test-host.sh`,
  so the host steps run through it rather than restating any of them. Whether it
  is usable is asked of that script itself — a new `--probe` that has *run* the
  preparation and reports the outcome, rather than a second copy of its
  conditions in the caller, because two definitions of "usable" drift. Where the
  native toolchain is absent the previous behaviour is unchanged and the verifier
  says so out loud before running, so a reader knows the Skia-linked failures
  below are about the environment.

  Also widened `-p migo-shared --lib` to `-p migo-shared`: the frame-cycle gate
  from task 0.40 is an integration test, and a verifier that ran only the lib
  target would have left it existing but never executed in the session's own
  verification.

  **★It broke `scripts/test-local-verification-contract.sh`, and I did not find
  that — CI did.** The contract recovers the crate list this script tests by
  grepping it for `cargo test -p migo-...`, and the routing above moved the word
  `cargo` out of the step strings, so the grep matched nothing. The claim
  recorded here first — "verified, 15 of 15 PASS" — was made without running the
  one contract that guards the file being changed, which is the same shape as
  every other gap in this ledger: the evidence covered what was easy to reach.
  Every quality-gate contract now runs before that sentence is written; 23 of 24
  pass locally and the Qt kit needs Qt.

  **Two defects, not one, and the second is why the first was hard to read.**
  That grep had no `|| true`, so under `set -euo pipefail` the empty match killed
  the contract *at the assignment* — before reaching the `fail "cannot find the
  crates scripts/verify-change.sh tests"` written directly underneath it for
  exactly this case. CI reported a bare exit 1 after the last passing assertion
  and said nothing else. The block immediately below it carries a comment
  explaining this precise hazard and applies `|| true` to itself; the fix had not
  been applied one block earlier. **The N-th instance of a guard covering only
  the face it was written for.**

  Fixed on both sides. `verify-change.sh --list-host-crates` reports the list
  from the one array that defines it, and the contract asks the script instead of
  guessing at its source — the §9 rule that a gate referencing an external
  identifier must resolve it from the authority or assert it still exists. Both
  extractions in that block are now non-fatal, so an empty one is reported rather
  than fatal-and-silent.

  **Mutation evidence.** Making the listing report nothing now fails with
  `[FAIL] scripts/verify-change.sh --list-host-crates reported nothing` — where
  before the same condition produced a silent exit 1. Adding a crate to the host
  steps that CI does not test fails at `pr-ci.yml runs migo-nonexistent's tests
  too`. Removing a crate from pr-ci's clippy list fails at
  `pr-ci.yml lints migo-graphics too`.

  **Verified** by the run this enabled: 15 of 15 PASS, ending
  `verified for every target this change touches`, plus all 23 runnable
  quality-gate contracts.

- [x] 0.42 Stop the derived-cache prune test depending on the host's wall clock.
  Found by `scripts/verify-change.sh` failing on a crate this branch had not
  touched: `prune_respects_budget_and_preserves_newer_files` asserted that the
  first-written file was evicted, and it survived a prune that removed three newer
  ones.

  **Diagnosed as far as the evidence allowed, then fixed in a way that does not
  depend on the diagnosis.** The test spaced six writes 20 ms apart and let the
  ambient clock supply the ordering; `prune_derived_cache` sorts on mtime with no
  tie-break, so a clock that does not advance monotonically across those writes
  inverts the expected order. It reproduced only under load — it passed 12 times
  in isolation and 8 more across the full suite, and the failing run took 2.10 s
  against 0.25 s idle. Rather than assert a root cause that could not be pinned
  down on this host, the fixture now stamps the six modification times explicitly
  with `File::set_times`, which removes the clock from the test entirely and drops
  120 ms of sleeps.

  The assertion is not weakened by the change: reversing the eviction order in
  `prune_derived_cache` still fails this test and nothing else.

  Recorded rather than folded silently into another commit, because an
  intermittent failure in shared verification is not a nuisance — it is the thing
  that makes every later "verified" line arguable.

- [x] 0.40 Pool the frame packet's op vector, or record why it stays unpooled.
  Pooled. It was the one allocation per frame that survived task 0.38's fix:
  `FramePacketBuilder::new` started from `Vec::new()` and the packet carried it to
  the render thread, so a frame cost an allocation and its doublings no matter how
  little it drew.

  It was its own item because the recycle point is not simply "after execution":
  the packet has several consumption sites, and the main one allocated **two
  more** vectors to reorder the ops into Canvas2D-then-GL phases. All of it is
  gone, in three parts.

  **The op vector is a loan** from a `FrameOp` pool, on the mechanism task 0.41
  built. It needed its own dimensions rather than the command pools': a packet
  holds one op per segment plus `BeginFrame`, the `Materialize` ops at each
  Canvas2D→WebGL boundary and a `Present` — single digits typically, about
  sixty-five in the heaviest scene profiled — while a `FrameOp` is several times
  a `GLCmd`'s width, so matching the command budget would reserve far more memory
  for far fewer elements.

  **The reorder no longer materialises a reordered packet.** It built `phase1`
  and `phase2`, each sized for the whole packet, concatenated them and dropped
  the original: three allocations and a full extra move of every op, to express
  an ordering the loop can simply take. Running the first phase as the packet is
  consumed and holding only the second — in one pooled vector, usually the
  smaller half — leaves zero.

  **The admission check no longer allocates either, and this one was not in the
  original count.** `packet_safe_to_reorder` built two `HashSet<u32>` per packet
  to ask whether a handful of small integers intersect. It gathers into an inline
  32-element list and returns on the first collision instead — no allocation, no
  hashing, and no walk of the GL half at all when the packet has no Canvas2D
  half, which is every WebGL-only frame.

  **Splitting the phase runner out is what made the reorder testable at all.**
  It is the one part of packet execution that can produce wrong pixels rather
  than slow ones — running a Canvas2D read of a WebGL canvas before the WebGL
  work that fills it — and it had **no tests**: the existing ordering test drives
  a separate test-only executor that does not reorder. `run_frame_phases` now
  takes the execution as a closure, so its output is observable without a GL
  context. Seven tests cover the classifier and four the ordering, including
  every op running exactly once on both paths and a present request surviving
  the deferred half.

  **The gate Section 7.3 asked for exists and is deterministic.** A whole frame —
  build the packet, run both phases, hand every loan back — reaches the heap zero
  times in steady state. It lives in `engine/crates/shared/tests/frame_cycle_allocation.rs`
  and **that is not a filing decision**: a frame takes from process-wide pools, so
  measuring it at zero needs the pool to hand back what the previous iteration
  returned. That holds in production and does not hold inside a lib-test binary
  whose dozens of other tests build packets concurrently — a neighbour taking the
  last vector leaves `take` with nothing to give and it allocates. Measured
  exactly that way: green standalone, red under the full suite. An integration
  test is the smallest unit that can hold the pools to itself, and it is also the
  unit that has to install the counting allocator.

  **Mutation evidence.** Each mutant killed the named tests and nothing else:

  - The original two-`HashSet` implementation: both reorder allocation gates,
    529 other tests still passing.
  - Dropping the deduplication: `repeated_canvas_targets_are_gathered_once`
    alone. That test is gated on allocation rather than on the verdict
    deliberately — gathering one canvas sixty-four times returns what gathering
    it once returns, so an assertion on the answer would pin nothing, and what it
    actually costs is the target list spilling to the heap.
  - A collision that no longer refuses: the three ordering-correctness tests.
  - Deleting graphics' counting `#[global_allocator]`: both gates, at the
    installation self-check.
  - `FramePacketBuilder` back to `Vec::new()`: the frame-cycle gate, **128
    allocation events over 64 frames (64 fresh, 64 resize, 43008 bytes)** — the
    measurement of what this item removed.

  **The two sets this item first recorded as "still not zero" are now zero too.**
  Writing the gate exposed that the render path held *three* independently
  written per-frame `HashSet`s of canvas ids, not one: the reorder classifier
  above, `execute_gl_batch`'s `touched_canvases` (allocated per GL batch, which
  Pixi reaches twice a frame and a Cocos scene far more often), and the
  collector's `pending_2d` (per frame, on the thread running the game). Each was
  built and thrown away to answer a question about a handful of small integers.

  They are one type now — `shared::protocol::CanvasIdSet`, an inline 32-entry
  set that deduplicates on insert, keeps its capacity across a `clear`, and
  spills to the heap rather than losing entries on a wider scene. **"A scene
  nobody has produced yet" is how this read when it was written, and it was
  wrong twice over** — the profiled scene was already at thirty of thirty-two,
  and because no call site reused a set, a spill was an allocation *per frame*
  rather than once. Corrected under task 0.52. One implementation means the
  deduplication a set implies is written once
  and gated once; an inline `SmallVec` that a caller forgets to check before
  pushing is a set only by intention. Iteration is insertion-ordered
  deliberately: these ids feed straight into emitted render ops, and a
  `HashSet`'s arbitrary order would make a packet's contents vary run to run for
  no benefit.

  Its own gates are deterministic — no pool, no GL — and four mutants were run:
  dropping the deduplication kills three tests including the burst; a `clear`
  that surrenders the allocation kills the capacity test; and reducing the inline
  array to zero entries kills the burst alone.

  **What is still not gated, named rather than implied:** the two new call sites
  inherit the type's gate but have none of their own, because both need what a
  host test binary cannot give them — `execute_gl_batch` a live GL context, and
  `build_frame_packet` a pool that no neighbouring test is taking from. ~~The type
  is the thing that can regress; the call sites can only regress by swapping it
  back out.~~ **That last sentence was false and is why task 0.52 exists.** A call
  site regresses by *sizing* as well as by swapping: a set constructed per frame
  turns one spill into one allocation and one free on every frame of the scene,
  and the type's gate cannot see which of its callers keeps its buffer. Two of
  the three did not.

- [x] 0.52 Stop the render path's canvas-id sets allocating on every frame of a
  wide scene. Section 7.3 recorded this as a blind spot in two parts: the shared
  `CanvasIdSet` "spills to the heap on a scene with more than 32 distinct Canvas2D
  targets in one packet", and two of its three call sites carry no gate of their
  own. Checking the premise before building on it found the first part **true and
  understated**, the second **true but not the reason it mattered**.

  **The spill was per frame, not once.** The type's own `clear` promised that "a
  set reused across frames stops allocating after the first spill" — and no call
  site reused one. All three constructed a set, filled it and dropped it, so above
  the inline capacity each frame bought a heap block and freed it again: on the
  render thread for the reorder classifier, on the thread running the game for the
  packet builder. Measured over eighty canvases: **128 heap allocation events over
  64 frames (64 fresh, 64 resize, 49152 bytes)** on the classifier alone.

  **And the gate that was supposed to cover it could not.**
  `classifying_a_packet_for_reorder_never_reaches_the_heap` ran against the
  thirty-canvas shop-open scene the reorder was profiled on — which *fits* inside a
  thirty-two-entry inline array — so it read green over a path that allocated on
  every frame of any busier scene. A burst has to be pointed at the workload, not
  at the workload that happens to fit the constant. Its fixture is eighty now.

  **What ruled out both cheap answers, recorded because it is a fact about the
  engine rather than a preference.** Canvas ids come from
  `CanvasManager::new_canvas_id`, a bare `fetch_add` with no free list, so they
  climb for the life of a session and never repack — an id-indexed bitmap would be
  sized by the highest id ever issued, not by the live count. And nothing caps live
  canvases (`MAX_LIVE_CANVAS2D_SNAPSHOTS` caps snapshots, not canvases), so no
  inline capacity is a bound either; widening 32 to 64 moves the cliff and calls it
  fixed. A cocos UI gives each text label its own offscreen canvas, so the profiled
  thirty is four table rows away from thirty-three.

  **The classifier gathers nothing now.** `packet_safe_to_reorder` asks whether the
  Canvas2D targets intersect the canvases the WebGL half binds — a boolean — and
  both sides are already in the op slice it was handed, so materialising either one
  buys nothing and costs a container. It scans the ops once per *distinct* canvas a
  WebGL command binds, which is normally one, carrying a one-entry memo of the last
  canvas **proven absent**. That asymmetry is the load-bearing part: a hit returns
  immediately, so only a proven-absent id is ever remembered, and a stale or
  mismatched entry can therefore cost a repeated scan and can never change the
  verdict. It is also less work than gathering was — that deduplicated on insert,
  a scan per Canvas2D op, and then scanned the gathered list once per WebGL
  command, which on a forty-label frame with three hundred GL commands is twelve
  thousand comparisons the memo collapses to forty-five.

  **The packet builder does need the ids** — it emits one `Materialize` op per
  pending canvas — so its set moved onto `UnifiedFrameCollector`, which spans
  frames, and the spill is paid once however wide the scene gets. It is acquired
  through `CanvasIdSet::begin`, which empties it and hands it out: the reuse is of
  the allocation and never of the contents, and a method rather than field access
  is what keeps that from being a rule someone has to remember.

  **Gated on the retained capacity rather than by a burst, and that is not the
  weaker choice here.** A burst over `build_frame_packet` would also measure the
  packet's pooled op vector, and this crate's five hundred other tests take from
  that pool concurrently — the interference that put the whole-frame burst in an
  integration test of its own over in `migo-shared`, which cannot reach the
  collector. The property at stake is exactly "the second frame does not allocate
  again", which retained capacity states directly and deterministically.

  **The third call site is unchanged, deliberately.** `execute_gl_batch`'s
  `touched_canvases` holds one entry per distinct canvas the batch's commands bind,
  and each of those carries a live WebGL context — an EGL context and its
  framebuffers — so it is not the label count that made the other two reachable.
  Removing its spill needs one of two things, and both were rejected: marking the
  2D contexts stale inside the dispatch loop sets the flag ahead of commands that
  might clear it, and holding the gathered ids in a `RendererGL`-owned scratch
  needs them to survive a `&mut RendererGL` call that could clobber them. Each
  trades a bounded spill for an invariant by convention, on the one path in packet
  execution where a wrong answer is wrong pixels and which no host test can observe
  because it needs a live GL context.

  **Mutation evidence.** Every kill below is at the named test's own assertion.

  | Mutant | Kills | Survives |
  | --- | --- | --- |
  | Classifier gathers a `CanvasIdSet` again | `classifying_a_packet_for_reorder_never_reaches_the_heap`, at 128 events / 49152 bytes | 552 |
  | Memo answers from its entry without comparing the id | `a_canvas_the_gl_half_also_touches_forces_issue_order`, `a_collision_in_the_last_command_of_the_last_batch_still_refuses`, `a_scene_with_eighty_canvases_still_finds_the_one_the_gl_half_collides_with` | 550 |
  | Scan truncated at forty ops | `a_scene_with_eighty_canvases_still_finds_the_one_the_gl_half_collides_with` alone | 552 |
  | Classifier's early-out deleted | **nothing** | 553 |
  | Packet builder gathers into a set of its own | `a_frame_wider_than_the_pending_set_pays_its_spill_once` | 517 |
  | Builder reaches the span-frames set without emptying it | `a_frames_materialize_ops_name_only_that_frames_canvases`, at `[1, 2, 4]` against `[4]` | 517 |
  | `begin` stops clearing | `begin_hands_out_an_empty_set_that_kept_its_capacity` and the same collector test | 6 + 517 |
  | `clear` surrenders the allocation | `clearing_empties_the_set_without_giving_up_its_capacity`, `refilling_a_reused_set_far_above_the_inline_capacity_never_reaches_the_heap`, `begin_hands_out_an_empty_set_that_kept_its_capacity`, `a_frame_wider_than_the_pending_set_pays_its_spill_once` | 4 + 517 |

  **Two of those rows are findings rather than confirmations.** The staleness
  mutant *walked* on the first attempt, and the reason was a defect in the test,
  not in the fix: both of its frames ended with a GL segment, and a GL segment
  consumes the pending run at the boundary — so the set was empty at every frame
  end and there was nothing to inherit. What survives a frame boundary is a
  *trailing* 2D run, which nothing follows and which a non-barrier packet does not
  materialise. Rewritten that way it dies naming the canvases it inherited. And
  the early-out kills nothing at all: deleting it changes no verdict and allocates
  nothing, because the scan then finds no Canvas2D op to match. It is a pure
  performance guard and is recorded as unpinned rather than counted as covered —
  the same status it had before this item, which is why it is worth saying.

  **One test was deleted, which is the one thing a test cannot report about
  itself.** `repeated_canvas_targets_are_gathered_once` was gated on allocation
  because gathering one canvas sixty-four times spilled the target list; with
  nothing gathered there is no deduplication on that path to pin, and the burst it
  shared a fixture shape with now covers eighty distinct canvases. Its sibling
  `more_distinct_canvases_than_the_inline_capacity_stay_correct` was renamed to
  `a_scene_with_eighty_canvases_still_finds_the_one_the_gl_half_collides_with`,
  because the classifier has no inline capacity to be above and the mutant table
  shows what the fixture actually pins: scale, and not the memo, which two
  pre-existing tests already caught.

  **Verified** with `scripts/verify-change.sh --base HEAD`: the whole host
  workspace, every crate's tests, `cargo fmt --all --check`, `git diff --check`,
  and the `arm64-v8a` Android compile the change to `render_thread.rs` requires.
  The classifier decides ordering rather than values, so a running engine is worth
  more than a suite here: the headless Linux player reaches first frame and holds
  60 fps for twelve seconds through the rewritten classifier. **That run exercises
  the early-out and nothing else** — bunnymark is WebGL-only, so it never builds a
  Canvas2D segment and never reaches the scan, the memo or the collision refusal.
  Those are covered by tests alone; a bundle mixing offscreen Canvas2D labels with
  an onscreen WebGL canvas is what would exercise them end to end, and the bench
  repository has none.

- [x] 0.53 Audit Section 7.3's "bounded hot paths" requirement and record what
  observes it. Opened on the premise that nothing observes this behaviourally —
  that `scripts/test-input-transport-contract.sh` greps for the absence of
  `unbounded_channel` and a saturation test is missing. **The premise is false and
  saying so is the finding**, because it is the second time this delivery has
  mistaken that script's greps for the whole of a requirement's coverage: for the
  *allocation* requirement (task 0.26) they really were all there was, and reading
  the same script the same way here understates the tree.

  `host_channel.rs` drives the real queue in eleven tests.
  `critical_bypasses_full_normal_budget_and_keeps_fifo` fills the normal lane and
  requires the refusal to return the command rather than drop it;
  `reliable_reserve_is_bounded_and_returns_original_command` does the same for the
  reserve; and `terminal_supersedes_older_motion_for_its_stream` **is** the
  saturation test this requirement's second sentence asks for — the lane is at
  capacity, and the terminal transition must come back `Enqueued` rather than
  `Reserved`, which holds only if it superseded the replaceable motion instead of
  spending the reserve or being dropped. Nothing needed building for the input
  transport; what was missing was the coverage list every other Section 7.3
  requirement carries, and its absence is what made the bullet read as a claim
  about every queue in the engine. It is written now.

  **What the audit found instead, and it is a defect rather than a gap.** The
  audio command transport is `tokio::sync::mpsc::unbounded_channel::<AudioCmd>()`,
  and its consumer drains at most `MAX_CMD_DRAIN = 256` commands per tick and
  defers the rest — an unbounded queue behind a capped drain, which is the growth
  shape the requirement names. The drain's own comment names the producer that
  reaches it: "prevent mixing starvation when JS fires rapid bursts (automation,
  game SFX)". So the design already anticipates a burst wider than the drain and
  answers it with unbounded memory instead of backpressure. The ceiling is about
  51,200 commands per second in Active mode and a tenth of that in the
  fifty-millisecond low-power window.

  Everything else on a hot path is bounded by construction: the render command
  channel is a `crossbeam_channel::bounded`, and the render thread's deferred
  upload queue refuses above `MAX_DEFERRED_UPLOADS` by handing the image and its
  responder back — the input queue's policy at another layer. `cancelled_uploads`
  only records ids that had a pending upload, with a comment saying why.

- [x] 0.54 Bound the audio command transport. Found by task 0.53.

  **It could not take the input queue's shape.** `AudioCmd` carries JS-allocated
  ids with fire-and-forget creates — `CreateContext`'s own doc says "FIFO channel
  ordering guarantees this command is processed before any node op that references
  `ctx_id`" — so ordering is the protocol, nothing in it is replaceable, and a
  dropped command leaves a later one addressing an id that was never created.
  Coalescing and supersession are both unavailable, and returning a refusal to JS
  is worse than useless: the Web Audio API has no error for it, so every op would
  swallow it and the loss would surface as a node that does not exist.

  **So it takes the render path's shape.** `shared::audio_channel` is a bounded
  crossbeam channel whose send waits — backpressure without loss, the policy this
  engine already uses for its other lossless-FIFO hot path. Deadlock-free for a
  reason worth writing down rather than assuming: the audio thread never waits on
  the producer, because every `AudioCmd` response is a `oneshot::Sender`, so there
  is no cycle between a blocked producer and the consumer that frees it. It is
  crossbeam's rather than a bounded `tokio` channel because `tokio`'s
  `blocking_send` panics inside a runtime context and deno_core ops run in one.

  **Three decisions are load-bearing.**

  - **The notification for a full queue happens before the wait.** The audio
    thread sleeps indefinitely once content has gone silent, and a send's own
    wakeup is what brings it back — so a send that parked first and notified after
    returning would wait for a drain that is waiting for it. Notifying before the
    wait cannot race: the queue is full, so the woken drain must free a slot, and
    `ThreadWakeup` latches its signal. This was found by design analysis, not by a
    failure, and it has a test whose failure mode is a deadline rather than a hang.
  - **The capacity is derived, not chosen.** A full queue must empty within a
    small fixed number of consumer iterations, because that count *is* the bound
    on how long a saturating producer waits;
    `AUDIO_COMMAND_CAPACITY = 4 * AUDIO_COMMANDS_PER_DRAIN`, and the relationship
    is a `const` assertion, so a capacity below one drain is a compile error rather
    than a slow queue.
  - **The pre-start backlog goes to the thread, not back into the channel.**
    `AudioService` buffers commands that arrive before the audio thread exists and
    used to re-inject them with `tx.send`. Against a bounded transport that is a
    deadlock: at the moment of handover nothing is draining the queue, because the
    receiver is still held by the service. The commands are handed over as a
    backlog the thread consumes ahead of the channel, which also removes an
    ordering argument that was never sound — the game thread can enqueue during the
    handover, and a re-injected command would then land behind a newer one.

  **The profile with no audio subsystem needed its own answer**, which is the
  hazard task 0.53 recorded. `AudioService` in `not(feature = "api-media")` held a
  receiver it never read, so a send queued for the life of the session — harmless
  only because the audio ops are compiled out of that profile and nothing could
  reach it. Behind a bounded channel that same shape parks the first producer to
  fill it. It holds `audio_channel::disconnected()` now — a sender with no receiver
  and no queue — so a send fails at once and hands the command back, and the reason
  it is safe stops being a fact about which ops happen to be registered. The
  Worker's placeholder sender had the identical shape and got the identical fix;
  there it is also a small improvement, since a worker that did use audio was
  accumulating commands in a queue nobody read.

  **A second requirement was closed on the way.** `tokio`'s unbounded channel buys
  a block from the heap every thirty-two messages, on the thread running the game,
  on a per-event path Section 7.3's zero-allocation list did not cover. A bounded
  crossbeam send allocates nothing, and the burst gate catches the difference
  independently of the boundedness assertions.

  **Mutation evidence.** Every kill is at the named test's own assertion.

  | Mutant | Kills | Survives |
  | --- | --- | --- |
  | The transport is unbounded again | `the_transport_is_bounded`, `past_capacity_the_queue_hands_the_command_back`, `a_steady_state_audio_command_send_never_reaches_the_heap` | 410 |
  | A full send parks before notifying | `a_send_into_a_full_queue_wakes_the_sleeping_consumer_that_frees_it`, at its deadline | 412 |
  | `disconnected()` keeps a live receiver | `a_disconnected_sender_holds_no_queue_and_refuses_every_send`, on the first command | 412 |
  | The drain consults the channel before the backlog | `the_startup_backlog_runs_before_anything_still_in_the_channel`, at `[30, 40, 10, 20]` against `[10, 20, 30, 40]` | 64 |
  | The service drops what it buffered | all three backlog-assembly tests | 51 |
  | Capacity below one drain | **compile error** at the `const` assertion | — |

  **One mutant walked first, and closing it is why two functions were extracted.**
  Dropping the pre-start backlog killed nothing: `crates/core/src/services/audio.rs`
  had no tests at all, and the only thing naming `check_and_start` is a wiring test
  that greps the host loop's source. The handover itself needs an audio device and a
  host test cannot provide one, so the two parts of it that decide correctness were
  lifted out where they can be observed — `take_startup_backlog`, which assembles
  the buffer and appends `PauseAll` last so a backgrounded app does not start
  playing what it buffered, and `next_command`, which is the drain's choice between
  backlog and channel. The mutant dies at three assertions now.

  **Verified** with `scripts/verify-change.sh --base HEAD` across 22 files: the
  host workspace, every crate's tests, `cargo fmt --all --check`, `git diff
  --check`, and the `arm64-v8a` Android compile. **Both profiles, because the gate
  only covers one:** cargo's defaults are `profile-full`, so the gate never
  compiled the `not(feature = "api-media")` branch this change touches —
  `--no-default-features --features profile-slim` compiles and tests clean
  separately. The headless player, which defaults to `profile-full`, reaches first
  frame and holds 60 fps for ten seconds with the bounded transport wired in and
  the host loop parking on its audio signal. **That run does not exercise a command
  flowing**, because bunnymark plays no audio; what it shows is that construction,
  lazy start and the idle path are intact. A bundle that uses Web Audio is what
  would exercise the transport end to end, and the bench repository has none.

- [x] 0.47 Gate the audio thread's own tick. Section 7.3 recorded the audio path
  as "measured at its graph render only", and the graph render is the half that
  runs inside `AudioContext::process`. The other half is the tick that calls it:
  `run_audio_thread`'s loop, which runs every five milliseconds for as long as
  anything is audible.

  **What the gate had to get past first.** The tick is unreachable from a host
  test binary — it wants an `AudioOutput`, which wants a device — so the part of
  it that is device-free was lifted out: `service_players` drains what the network
  delivered, adopts a finished stream into the cache, and hands out the events the
  players raised, over an event sink rather than `HostTx`. The production call
  site passes a closure over `host_tx` and captures by reference. One iteration of
  the burst is then a whole tick: poll the stream, mix a block, emit.

  **What it found.** `take_events` handed the player's event vector to the caller
  and left the player holding a zero-capacity one, so the next event bought a
  fresh vector — and a playing player raises a throttled `TimeUpdate` about four
  times a second, forever, per player. **Six allocation events over 64 measured
  ticks, 384 bytes**, on the thread whose whole job is to be on time. It is
  `drain_events` now, which keeps the capacity, and the vector is bought at
  construction rather than on the first event.

  **Mutation evidence.** Restoring `take_events` and the zero-capacity
  construction verbatim kills
  `a_steady_state_audio_thread_tick_never_reaches_the_heap` and nothing else, at
  the same six events and 384 bytes. The gate also asserts it saw events at all,
  because a burst over a silent player would pass while proving nothing.

  **What is still not gated, named rather than implied:** the rest of the tick —
  the command drain, the decode-result drain, the power-state transitions and the
  refill loop's `output.write` — stays inside `run_audio_thread` and needs a
  device. What was lifted out is what repeats per player per tick.

- [x] 0.48 Gate the hardware output callback, and stop it owning a heap buffer.
  This one is a correctness item wearing a performance item's clothes. The
  callback runs on a thread the platform schedules as real-time — `SCHED_FIFO` on
  Android — so an allocation in it is not slow, it is a missed deadline heard as
  a dropout, because the allocator can block behind a thread that is not
  real-time scheduled at all.

  **The callback was three closures handed to cpal, which no test can reach.** It
  is a named type now, `OutputCallback`, with the sample conversion behind a trait
  so the three near-identical bodies are two methods. That is what makes it
  measurable, and it deletes about ninety lines of duplication on the way.

  **What it found.** The integer-format callbacks owned a `Vec<f32>` pre-sized to
  4096 samples and grew it with `resize` whenever the device asked for more. A
  device is under no obligation to ask for a number this code guessed — AAudio's
  `numFrames` varies across route changes and stream recovery, and cpal's ALSA
  backend sizes each callback from the period space actually available — so that
  `resize` was a `realloc` inside a real-time callback. The conversion now runs
  through a fixed 512-sample stack scratch, a chunk at a time: there is no
  capacity to outgrow at any device buffer size.

  **Two gates, and the second one is the point.** A steady-state burst cannot see
  this defect and saying why matters: the buffer only ever grew, so the one
  `realloc` a large device buffer caused happened during the warm-up and the
  measured window was clean. That is not an acceptable answer for a path whose
  first call is as deadline-bound as its ten-thousandth. So the second gate runs
  every iteration against a callback that has never run, from a fleet built before
  the measured window. **This needed no new mechanism** — a burst over one-shot
  subjects measures first calls exactly — which is why the three probe crates are
  untouched.

  **Mutation evidence**, and it separates the two gates rather than restating one:

  - The pre-fix heap scratch, restored verbatim: the cold gate alone, at **64
    resize events and 2,097,152 bytes**. The steady gate stayed green, which is
    the claim above demonstrated rather than argued.
  - An allocation in `render_native`, which only the steady gate exercises: the
    steady gate alone, at 64 events and 2048 bytes.

  Two behavioural tests sit beside them, because a chunked scratch that silenced
  or reordered samples would pass every allocation gate: the conversion is
  compared to the ring across a deliberately non-aligned chunk boundary, and an
  underrun is required to pad with silence and ask for a refill.

  **What is still not gated, named rather than implied:** that cpal actually
  calls this, and that the thread it calls it on is the real-time one. Both need
  a device, and the second needs a device on Android.

- [x] 0.49 Gate the streaming refill, and give the decoder its state back.
  The unit is one network chunk, because that is what repeats — a track is
  thousands of them — and everything inside it is per-frame, so an allocation
  here is multiplied by however many frames the chunk carried.

  **What it found: nine allocation events per chunk, 3,923,144 bytes over 64
  chunks**, for two MP3 frames each. Four causes, and the first is not an
  allocation problem at all:

  - **A decoder was constructed per chunk.** That is three allocations — a 6.6 KiB
    `mp3dec_t`, a virtual-memory-backed ring, an 11 KiB refill buffer — and, far
    worse, it **reset the bit reservoir**. MP3 frames are entitled to reach back
    into the previous frame's main data, and a frame that does so decodes to *no
    samples at all* against a decoder that has just been reset. Silently short
    audio, no error anywhere. `Mp3FrameDecoder` now lives as long as the stream.
  - **minimp3 keeps that state only when it can confirm the next frame's header**
    in the same buffer, and wipes the decoder when it cannot. Decoding right up to
    the end of a still-growing buffer therefore threw the reservoir away at every
    chunk boundary regardless. The decoder keeps `STREAM_LOOKAHEAD_BYTES` — two
    maximum frames — behind its cursor.
  - **Each frame was converted through a fresh `Vec<f32>`**, and the safe wrapper
    allocated a `Vec<i16>` per frame to hand it over in the first place. Frames
    decode into a buffer the decoder owns and convert through one reused scratch.
  - **The resampler rebuilt its one-frame history with `to_vec` per call** — one
    allocation per frame for two samples.

  **The chunk that crosses threads is a loan.** It has to be owned, so without a
  way back it is one allocation on the streaming worker and one free on the audio
  thread for as long as anything plays. `PcmChunk` returns its buffer on `Drop`
  rather than through a call someone has to remember — the same instrument the
  render path's command vectors use — and the return is a `try_send` that never
  blocks, which matters because the common caller is the audio thread.

  **Result: zero, and the warm-up is not a hiding place.** The window is 40
  warm-up iterations because tokio hands out message slots 32 to a block and
  recycles them, so the return channel's block list grows once; the property was
  checked over **256** measured iterations before the 128 in the committed gate
  was chosen.

  **Correctness is pinned separately, and as a reftest rather than a golden
  file** — both sides are computed in the same run, so there is no baseline to go
  stale: a stream cut into 137-byte chunks must decode to exactly what one pass
  over the same bytes produces. The fixture is synthesised in
  `mp3_fixture.rs` rather than checked in, which is what lets it *state* the
  property under test: every frame after the first declares that its main data
  begins in its predecessor's reservoir. A control test proves the fixture is a
  real MP3, and a second proves the fixture is strong enough — a decoder rebuilt
  per frame recovers exactly one frame of the four.

  **Mutation evidence.** Each mutant killed the named tests and nothing else:

  - Rebuilding the decoder per decode: the chunk gate (**512 events, 2,886,656
    bytes**) *and* both correctness tests — the chunked-versus-one-pass reftest
    and the trailing-tag one.
  - The resampler's `to_vec` history: the chunk gate alone, at **256 events, 2048
    bytes** — exactly one per frame, eight bytes each.
  - A loan that never returns: the chunk gate alone, at **128 events, 16,777,216
    bytes** — exactly one 128 KiB buffer per chunk.
  - Removing the lookahead: both correctness tests, and neither allocation gate,
    which is right — losing the reservoir costs audio, not memory.
  - Running the flush's in-place pass before isolation instead of after: the
    non-audio-bytes test alone. This is the ordering argument as a mutant.
  - Making the probe measure by decoding rather than through minimp3's
    null-output path: the non-audio-bytes test alone.
  - Offering the whole-file decode the whole remainder again: its own
    tags-around-the-frames test alone.

  Re-run in full after the `Skipped` fix, since that changed what `decode` means
  by a byte it did not use. One result moved and moved for the better: removing
  the lookahead no longer kills
  `the_decode_step_leaves_the_shared_streaming_worker_free`. That test asserts the
  decoder retained a chunk it could not decode, and it used to be sensitive to the
  lookahead constant; now the hold-back retains it for its own reasons, so the
  assertion says what it means rather than what the constant happened to imply.

  The loan's identity is asserted by address as well, because a pool that quietly
  allocated a fresh buffer each time satisfies every other observable property and
  only the allocation gate would notice.

  **Carried into the full-file decode for free.** `decoder::mp3::decode` uses the
  same frame decoder, which removes the per-frame `Vec<i16>` from every audio load
  — roughly seven thousand allocations for a three-minute track — and one full
  copy with it.

  **How much audio the reservoir fix is actually worth, measured against the
  code it replaced.** The per-chunk decoder was not losing an occasional frame;
  it was losing most of them. Running the old algorithm inline against the same
  bytes, with no trailing tag at all:

  | stream | per-chunk decoder (before) | persistent decoder (after) |
  | --- | --- | --- |
  | 40 frames | **14 of 40** | **40 of 40** |
  | 6 frames | 2 of 6 | 6 of 6 |
  | 40 frames + ID3v1 tag | 13 of 40 | 39 of 40 |

  That is the defect stated as audio rather than as allocations: about two thirds
  of a streamed track was being dropped, silently, with no error on any path.

  **Bytes that are not audio used to cost frames, and now cost none.** minimp3
  will not accept a frame it cannot chain to a successor, so a tag at either end
  of a stream defeated it: an ID3v1 tag (128 bytes, at the end of a large
  fraction of real files) stranded the final frame, and a stream shorter than the
  lookahead — which decodes nothing until the flush and therefore arrives with a
  decoder that has never run — lost *every* frame it had, decoding to silence
  outright. That last one is the case short remote sound effects fall in.

  Three things were needed, and the order of two of them is the fix rather than a
  preference:

  - **minimp3 accepts a buffer that is exactly one frame** without asking for a
    successor to confirm it. So the flush isolates frames, handing the decoder one
    at a time, and the real decoder then takes its state-preserving fast path.
  - **The length comes from a throwaway decoder**, because every rejected probe
    resets minimp3 and the bit reservoir is exactly what must survive.
  - **The probe measures without decoding**, via minimp3's null-output-pointer
    path. Measuring by decoding does not work and the reason is the same one
    underneath everything here: a frame whose main data lives in a reservoir the
    *probe* does not have decodes to zero samples and is indistinguishable from
    garbage — so the probe reported "no frame" for precisely the frames worth
    rescuing. This was found by instrumenting the flush phases after the first two
    parts landed and still lost the last frame of every long stream.

  Isolation runs **before** the in-place pass, not after. An in-place decode that
  fails resets minimp3, and what it resets is the reservoir belonging to the very
  frame it just failed on — trying in place first destroys the state needed to
  recover it. The in-place pass is kept as a fallback for the one thing isolation
  cannot do, getting past leading non-audio larger than a frame, and runs only
  when isolation could not move at all.

  A fourth piece closed the last gap: **the probe searches from the next sync
  candidate when nothing is isolable at the front.** A leading tag larger than a
  frame — an ID3v2 tag routinely is — hides the audio from a front-anchored probe
  as well as from the in-place path, and ID3v2 at the front paired with ID3v1 at
  the end is the classic layout. Eleven set bits is the sync word, so candidates
  are cheap to find; minimp3 still decides, and the search is capped at 32 false
  starts so a pathological file cannot cost a rescan per byte.

  Measured across 24 combinations of stream length (1 to 96 frames, spanning both
  sides of the lookahead) and tag placement (none, trailing, leading, both):
  **all 24 exact.**

  **The same defect was in the whole-file decode, it was worse there, and it was
  not mine.** `decoder::mp3::decode` is the path every locally loaded or fully
  downloaded MP3 takes — far more travelled than streaming — and it offered
  minimp3 the whole remainder, so the same chain-check failure applied. Measured
  against the previous implementation, cell for cell **identical**: the last frame
  of a long tagged file lost, and *every* frame of a short one, which made
  `decode` report that it had produced no samples. A tagged two-second sound
  effect did not play slightly short — it failed to load.

  It is fixed by handing that decoder one frame at a time as well, uniformly
  rather than only near the end, which is both simpler and strictly better: a lone
  frame is what minimp3 accepts without a successor, and it is also its
  state-preserving fast path, so the reservoir survives every step instead of only
  the ones a lookahead happened to cover. **All 20 combinations exact**, against 15
  of 20 before.

  The extra call per frame measures the frame and returns before any decoding
  work, and it costs nothing measurable: 2000 frames — about 52 seconds of audio —
  decode in **18.77 ms isolated against 18.80 ms whole-remainder**, release build,
  twenty runs each.

  Three earlier attempts are worth recording because each was plausible and each
  was wrong. Decoding eagerly until the first frame lands recovered the short case
  only partly (1 frame of 6) while costing a frame on streams that had been exact
  (23 of 24) — reverted. Isolating *after* the in-place pass fixed one- and
  two-frame streams and nothing else, for the ordering reason above. Keeping the
  whole-file decode's bulk phase and isolating only its tail left a short file
  with tags at both ends at zero, because the bulk phase's "skip everything but
  the last frame's worth" rule — which is right for a stream that may still grow —
  discards real audio in a file that is already complete.

  The committed test asserts every combination **exact**, deliberately not "at
  most one frame lost": a rule that tolerates one lost frame tolerates the
  mechanism ceasing to work and losing it every time.

  **That experiment did find a real latent defect, though, and it is fixed.**
  `Mp3Step::Skipped` was conflating two different answers. When minimp3 consumes
  *less* than it was given it has got past a real frame it could not decode; when
  it claims everything it looked at it means "nothing usable here" — and a frame
  that has merely not arrived in full is indistinguishable from garbage at that
  point. The old rule held back three bytes, enough for a split sync word and not
  enough for a partial frame, so a `decode` called on a buffer without a complete
  frame would discard the front of one. Nothing reached it, because the lookahead
  guarantees a complete frame is present — safe by accident of a constant rather
  than by construction. It now holds back `MAX_FRAME_BYTES + 3`: any frame that
  began later than that could still be incomplete, and anything older cannot be,
  because its frame would have fit. The bound is what keeps pure garbage from
  growing the buffer without limit.

  **What is still not gated, named rather than implied:** the network side. The
  reqwest body stream, the response handling and the `OffWorker` hop each allocate
  and none of them is covered; the gate starts where the bytes are in hand. And
  `PCM_CHUNK_SAMPLES` is a starting capacity, not a bound — a chunk larger than a
  third of a second of 48 kHz stereo grows its buffer once, and the recycled
  buffer then keeps the larger capacity.

  **A trap worth recording, because it cost a false result.** The mutation
  harness restored files with `shutil.copy2`, which preserves mtime — so cargo saw
  nothing newer than the artifact it had already built and reran the *mutated*
  binary against the restored tree. That is CLAUDE.md §9's WSL2 mtime trap arriving
  through a new door. Restores touch the file now.

- [x] 0.51 Build the steady-state growth gate Section 7.3 requires, then apply it.
  The fifth of Section 7.3's structural requirements to get a mechanism. Unlike
  task 0.50, auditing this one found **no defect**: 180 s of continuous rendering
  at 60 fps moves resident memory 270.4 MB → 257.7 MB, net **−12.7 MB**. So this
  task is a missing gate rather than a missing fix, and it is worth saying that
  plainly rather than manufacturing a finding.

  **Why the allocation burst could not be reused.** A burst asks whether a path
  touches the heap at all, which is only answerable where the answer must be
  never. Every path this requirement is really about — admit and evict a cache
  entry, take and release an alias, open and close a connection — allocates for a
  living, so no burst can be pointed at it. `Cycle` /
  `assert_no_steady_state_growth` asks the other question: did the window give
  back what it took. Net live bytes, from the same counting allocator, which
  gained `bytes_freed` and a signed `live_bytes()` for the purpose.

  Design points, each pinned by a control or a mutant:

  - **A resize is both ends at once.** `realloc` takes the new block and returns
    the old, so `record_reallocation` records both. Recording only the new size
    makes every growing container look like it leaked the difference — the mutant
    that drops the old-size half fails `growing_a_block_nets_only_the_difference`
    at 4096 against 4032.
  - **The installation check is stricter than the burst's, and it has to be.** A
    growth gate has one extra way to be silently green: if *frees* stopped being
    counted every cycle would look like it grew, which is loud and harmless, but if
    *allocations* stopped being counted every cycle would look like it shrank and
    the gate would pass forever. So the check is not "did we see an allocation" but
    "did a known allocate-and-release pair net to exactly zero", which no single
    broken counter satisfies. The mutant that stops counting frees is killed by the
    check itself, naming the direction.
  - **The judgement is a pure function of the observed counts.** A broken allocator
    cannot be installed beside the real one — `#[global_allocator]` is unique per
    binary — so `untrustworthy_growth_observation` takes counts and returns the
    reason, and the four disqualifying shapes are unit-tested against fabricated
    ones. This is the same "separate the observation from the judgement" move that
    makes the other probes' policies testable.
  - **Passing means net.** A cycle that leaks a hundred bytes while releasing two
    hundred elsewhere passes, because net is what "does not grow" means and a
    stricter reading fails every legitimately shrinking path. Stated in the doc
    comment rather than left for a reader to discover.

  **A claim written before it was tested turned out to be false, and the test is
  what caught it.** The mechanism was designed to close the pooled-vector hole
  task 0.41 recorded as open — "nothing catches a deliberate leak". The reasoning
  was that a lost loan is a missing deallocation, so a net-bytes measure would see
  it where an allocation burst cannot. **It does not.** A delta measure only moves
  when the *window* allocates or frees, and a loan was allocated before the window;
  taking it from the pool and forgetting it moves neither counter. The test written
  to demonstrate the closure failed, and it is kept — inverted, as
  `a_block_taken_from_an_earlier_population_and_leaked_is_not_visible` — so the
  boundary is pinned rather than re-assumed. Section 7.3's pooled-vector paragraph
  is corrected in place. The lost loan still surfaces only when the drained pool
  forces an allocation, which is the burst's second-order signal, not a new one.

  **Applied to the image cache, and the mutant proves it is not redundant.** Two
  gates: the reservation round trip (`pin` then `unpin` of a *non*-resident key,
  which must allocate an owned key and give it back) and admission at the byte
  budget. The reservation table is the one structure in that cache `current_size`
  does not account for, which makes it the one place growth can hide from every
  budget test **and** from the public API: a reservation left behind at a count of
  zero is indistinguishable from an absent one through `pin_count`
  (`unwrap_or(0)`), and invisible to `size_bytes`. Deleting the
  `reservations.remove(key)` that runs when a count reaches zero fails
  `a_reservation_round_trip_gives_back_the_key_it_took` at 6968 retained bytes over
  64 iterations — **and nothing else in the crate**, across 266 tests. That is the
  general lesson: measure the allocator, not the structure's own accounting, because
  a guard that reads `current_size` cannot see growth in what `current_size` does
  not count.

  The gate needs distinct keys per iteration, which is not cosmetic: repeating one
  key would let a table that never released it still look balanced, because the
  second pin of a live reservation allocates nothing. They are the same length so
  the balanced net is exactly zero rather than approximately so.

  **The process-level instrument is committed too**, because the requirement is
  about resident memory and the heap gates cannot see GPU allocations, the V8 heap,
  mmap or fragmentation. `scripts/measure-steady-state-growth.sh` samples `VmRSS`
  across a long workload and **fails when the content stopped producing frames**,
  for the same reason the idle-wakeup script asserts painted frames: a stalled
  engine does not grow either. Two instruments, one two-sided rule.

  **Verified.** migo-alloc-probe 11 unit from 3 and 17 harness from 8, migo-io 266
  from 264, migo-shared 405, migo-graphics 554, `cargo fmt --all --check` clean, and
  `scripts/verify-change.sh --base HEAD` PASS on every host step and on
  `android compile`.

  **What this does not close.** The threshold that turns the process measurement
  into a gate belongs in the versioned baseline file (Phase 5), and the run exists
  on the Linux host only. Session create/destroy cycles, the V8 heap across a soft
  restart, and GPU-side growth have no gate of their own — named rather than
  implied, since "no steady-state growth" would otherwise read as covering them.

- [x] 0.50 Make frame delivery demand-driven on the engine-paced platforms.
  Section 7.3's idle-quiescence requirement was the last of its structural
  requirements with no mechanism at all, and auditing it found a live defect
  rather than a missing test: **Linux, Windows and HarmonyOS woke the render
  thread sixty times a second with nothing to draw.**

  **What was wrong, and why it survived.** `RenderThread::spawn` created
  `crossbeam_channel::tick(1/fps)` whenever the platform reported no external
  vsync source, and the `recv(ticker)` arm drained commands, presented and touched
  stats on every tick regardless of demand. Android escapes this because
  `FrameClock::uses_external_vsync` is true for it, so it arms one Choreographer
  callback at a time through `should_arm_one_shot`. The revealing detail is that
  `should_arm_one_shot`'s own passing test asserted
  `should_arm_one_shot(has_vsync=false, ..) == false` — "no vsync source (desktop
  ticker) → never arm". The demand-driven policy was written, tested, and then
  deliberately excluded from the path whose unconditional ticker *was* the clock.
  So this was never a partially-implemented requirement; it was a requirement met
  on the one platform that had an arm route. `LinuxPlatform` and `WindowsPlatform`
  answer false, and HarmonyOS reaches the engine through `CapiHostKit`, which
  answers false unless the C host installed `on_request_frame` — which the ArkTS
  bridge does not. Three of the four delivered platforms, one battery-powered.

  **The fix is one policy over two arms, not a second policy.**
  `SoftwareFrameClock` answers when the render thread should next wake, `None`
  meaning never, and `RenderWait` is the loop's single wait point taking that
  answer. Design decisions and why each is load-bearing:

  - **The deadline is an argument to the wait, not a timer channel.**
    `crossbeam_channel::at(deadline)` was the obvious mechanism and is the wrong
    one: it allocates a channel per armed frame, sixty allocations a second on the
    very render thread tasks 0.38 through 0.41 spent four commits de-allocating.
    `Select::ready_deadline` re-arms for free. The `Select` itself is built once
    outside the loop for the same reason.
  - **The pacing grid outlives the idle period.** Two `Option<Instant>`s, not one:
    `armed_at` is the scheduled frame and `earliest_next` is the grid. Demand
    republished two milliseconds after a frame therefore waits for that frame's
    slot, so a rAF loop cannot spin the clock — with a single field, re-arming
    would set the deadline to *now* and free-run. The grid advance is computed, not
    iterated: dropping the partial interval `ran_at` sits in and adding a whole one
    lands on the next slot, so a frame that overran by ten slots owes one frame
    rather than ten, an on-time frame keeps its phase, and there is no
    multiplication to overflow.
  - **An overdue frame is served before the channels are polled.** `Select` returns
    any ready operation without consulting the deadline, so a continuously-ready
    command queue would starve the frame indefinitely. The converse cannot happen:
    the frame branch drains the command queue itself and running a frame advances
    the grid past now.
  - **Stopping the clock is only safe because demand can reach a sleeping thread.**
    On engine-paced platforms `request_vsync` becomes a single-slot nudge the wait
    selects on, so `op_await_next_frame` wakes the render thread and the thread
    then arms its own clock. The nudge carries no payload — demand is read from the
    latch — which is what makes a full slot the right thing to drop rather than a
    lost wakeup. The render thread is deliberately *not* handed the nudge closure:
    a thread nudging itself is a wakeup that arms nothing, and `host.rs` passes
    `None` there so that is a compile-time fact rather than a rule.

  **One asymmetry is deliberate and recorded where it is written.**
  `should_arm_engine_paced` omits `can_present`, which its vsync sibling requires.
  Asking a compositor for a frame callback with no live surface is meaningless,
  whereas an engine-paced frame with no surface still opens the per-frame upload
  budget and drains completed uploads — the exact stall the vsync branch's own
  comment describes, and a real risk for a C host that loads content before
  handing over its window. Uniformity here would have been a regression dressed
  as symmetry.

  **Also cleaned up, because the ticker was scattered state.** `ticker` was a
  `Receiver<Instant>` reassigned from four places (init, `FrameRate`, `Pause`,
  `Resume`). Those become `set_fps`, `stop` and demand, and the four `select!`
  arms become a `match` on a named `Wake`. `Select` readiness is advisory, so each
  arm now distinguishes an empty receiver (spurious wakeup, wait again) from a
  disconnected one — which the `select!` version could not express.

  **Measured, and the measurement needed two sides.**
  `scripts/measure-idle-wakeups.sh` reads the render thread's and the whole
  process's `voluntary_ctxt_switches` while committed probe content
  (`scripts/fixtures/idle-probe`) paints twice and then stops asking for frames:

  | | render thread | whole process | frames painted |
  | --- | --- | --- | --- |
  | before | 59 wakeups/s | — | 2 |
  | after | **0** | **0** (53 threads) | 2 |
  | engine-paced arm deleted | 0 | 0 | **0** |

  That third row is why the script asserts painted frames before believing the
  silence: **an engine that never renders is also perfectly quiet.** It is the
  always-red-gate failure mode this ledger has hit twice in tests, arriving
  through a measurement instead — and it is also the mutation evidence for the
  clock-to-loop wiring, which no unit test covers.

  **Mutation evidence.** Seven mutants, each killed by exactly one named test at
  that test's own assertion: the clock starting armed
  (`an_idle_clock_schedules_no_wakeup`), a frame leaving it armed
  (`a_frame_that_ran_leaves_the_clock_idle_until_demand_re_arms_it`), arming
  ignoring the pacing slot
  (`re_arming_inside_the_current_slot_cannot_raise_the_frame_rate`), a frame
  re-phasing instead of advancing the grid
  (`pacing_does_not_drift_when_every_frame_runs_late`), `stop()` not retiring the
  wakeup (`stopping_cancels_the_armed_wakeup`), the arm ignoring pause
  (`engine_paced_arm_needs_demand_and_a_running_clock`), and the wait polling
  channels before an overdue frame
  (`an_overdue_frame_is_served_before_a_ready_command_queue`).

  **One mutant survived first, and fixing it changed the design rather than the
  test** — which is the reading rule this ledger already records, applied to its
  own case. With state `slot: Option<Instant>` plus `armed: bool`, mutating
  `new()` to start armed killed nothing: `deadline()` returned `self.slot`, which
  is `None` on a fresh clock, so the idle test passed for the wrong reason.
  `armed && slot.is_none()` was an unreachable state that was nevertheless
  *representable*, and a representable invalid state is what let the mutant hide.
  Replacing the pair with `armed_at: Option<Instant>` makes "armed for no
  particular time" unspellable, and the same mutant then dies at the assertion it
  was aimed at. The lesson generalises: when a mutant walks, ask whether the state
  space is too large before assuming the test is too weak.

  **Verified.** `scripts/verify-change.sh --base HEAD` PASS on every host step and
  on `android compile` for `arm64-v8a`, which it required because the change
  touches `render_thread.rs`. Whole workspace green (migo-graphics 554 from a
  measured 535 baseline — eleven clock tests, one arm-policy test, seven wait
  tests; migo-shared 405, migo-core 51, migo-capi 142, migo-io 264,
  migo-platform 50), `cargo fmt --all
  --check` and `git diff --check` clean, and no new clippy finding in any changed
  file (the two `warm_frames -= 1` saturating-subtraction warnings are present at
  baseline; `frame_scheduler.rs` and `render_wait.rs` are clean).

  **End-to-end, not only in units.** The headless Linux player runs the bunnymark
  bundle at a steady **fps=60** and captures a 720x1280 presented frame, so the
  demand-driven clock paces at the target rate rather than merely stopping.

  **What this does not close.** Section 7.3's ceiling belongs in the versioned
  threshold file, which is Phase 5's; the measurement exists on the Linux host
  only, so Windows and HarmonyOS need the same instrument run against their own
  hosts before their rows are real, and HarmonyOS's target build stays blocked
  behind item 0.32. A disconnected external vsync channel would leave the wait
  spinning — pre-existing on the Android path, unchanged here, and named because a
  quiescence claim should not be read as covering it.

- [x] 0.28 Give pack-backed image cache keys a globally meaningful identity.
  Found by spec-checking the sharing precondition in Section 6.5 while finishing
  0.19's second half, and confirmed in the source rather than inferred. A `/code`
  path that resolves to a real file gets a token hashing the resolved real path with
  the file's size, mtime and mount origin, which is what makes sharing the decoded
  bytes safe. A path that resolves *inside a package* has no real path, and
  `worker_image_source` falls back to `resolved.source_mounted_at` alone. That
  counter lives on a single `MountTable`, each Session has its own, and a base mount
  is `1` in every one — so two games shipping different packages collide on
  `("/code/<same virtual path>", 1, 0, 0)` and the second is handed the first's
  decoded pixels.

  Reachable in the ordinary case, not an edge: a shipped game is normally
  pack-backed, and any filename two games share is enough. It is also not recoverable
  from content any more, now that `ImageCache.clear()` is correctly session-scoped —
  the colliding entry retains the other Session as an owner and survives the clear.

  **The identity already exists, so this is plumbing rather than design.**
  `PackSource::identity()` returns a `PackageIdentity { name, version, checksum }`
  whose checksum is a deterministic CRC32 over the package's entry metadata, which
  means the same thing in every Session — exactly what `source_mounted_at` does not.
  Hot update is covered by construction: `install_package` and the subpackage mount
  path both build a fresh `PackSource`, so a changed package yields a different
  identity and therefore a different key.

  **Done, and it needed a prerequisite fix nobody had noticed.** `PackageIdentity`
  documents its checksum as a deterministic CRC32, and it was not: the reader
  computes it by iterating a `HashMap<String, EntryMeta>`, every `HashMap` instance
  hashes with its own seed, and CRC32 is order-dependent — so two opens of the *same
  package file* could produce different identities. As a cache discriminator that
  would have been worse than the bug being fixed: it does not collide, but it defeats
  sharing between two Sessions running the same content, and it changes across a
  restart. Both the writer and the reader now hash in sorted path order, so the two
  sides also agree with each other.

  **Correct by construction rather than contingent on the accessor.** Each mount now
  carries an id unique within the process, and `ResolvedCode::source_identity` is the
  backend's own identity where it has one and that id otherwise. So a backend that
  never implements `source_identity` cannot reintroduce the collision — it loses
  sharing instead, which is the safe direction.

  **A second collision of the same shape, found while fixing this one.** The
  `ImageSource::Pack` branch that re-resolves to a *real* file also keyed on
  `source_mounted_at`, so two directory-backed Sessions collided on the same virtual
  path. It now uses the same real-path variant token the directory-backed branch
  uses, which is globally meaningful.

  Two behavioural tests, and they pull in opposite directions on purpose: different
  packages must not agree, identical packages must still agree. Three mutants, each
  landing where it should — the pre-fix key (the mount position alone) kills the
  collision test; making every mount unique and ignoring the package's identity, the
  tempting over-fix, kills the sharing test; and restoring the `HashMap`-order
  identity also kills the sharing test.

  A precision note on that third one, because it took a second attempt: with
  single-entry packages it killed **nothing**, since a one-entry `HashMap` has only
  one iteration order. The fixture now builds thirteen-entry packages. That kill is
  still probabilistic in principle — it depends on two `HashMap` instances ordering
  those entries differently — but only in the direction of missing the mutant:
  correct code always agrees, so the test cannot flake on green.

  Not covered: no fixture drives two live Sessions through the JS `Image` path to the
  same virtual path; the tests exercise the key-construction choke point
  (`current_image_cache_key`, which both the LRU fast path and the decode worker go
  through) directly.

  **Independent review then found two more, both real, both verified in the source
  before accepting.**

  The first was a regression this work introduced. The loader pre-pins the
  decoded-bytes slot before the decode inserts into it, and it keyed that pin off
  `resolve_local_src`'s own resolution — which used `source_mounted_at`, matching the
  io side only because the io side used it too. Changing one and not the other split
  them, so the pin landed on a slot nothing was inserted into, leaving the admission
  filter free to reject the real entry and a later `texImage2D(image)` to find no
  bytes and render black. Both sides now read the *same field*.

  The second was that folding `source_mounted_at` into the token defeats the sharing
  it was meant to preserve. Installed packages are restored by iterating a
  `HashMap<String, ManifestEntry>`, so two Sessions mount the same subpackage in
  different orders and it lands at a different position in each table. The token is
  now the identity alone, which is also simpler: a remount of a *changed* package
  changes the content identity, and a replacement with no backend identity changes
  the fallback mount id, so nothing needed the per-table counter.

  Both fixes are mutation-proved. Reinstating the counter now fails two tests — the
  key-agreement test and a new mount-order test — and the pre-fix key fails the
  agreement test and the collision test. The mount-order test was added *because*
  reinstating the counter initially failed only the agreement test: the existing
  fixture gave both Sessions the same mount position by luck, so the half of the
  finding about sharing was not covered at all. It mounts two subpackages in opposite
  orders and asserts the positions really do differ before asserting the keys agree.

  **A further round found the identity itself too weak, and it was right.** It was
  `name + version + PackageIdentity::checksum`, and the first two are labels the
  installing app picks — a package claiming another's name and version contributes
  nothing to telling them apart — which left a **32-bit aggregate CRC32** as the only
  discriminator between two games' packages. CRC32 collisions are straightforward to
  construct, so a crafted package could still be served another game's pixels. The
  identity is now a SHA-256 over every entry's path, size and CRC32 in sorted order.
  The collision test was changed to give both packages the *same* name and version, so
  it now pins that the identity comes from content rather than from labels: an
  identity built from labels alone fails it.

  **Residual, and recorded rather than glossed:** this is not a content hash. The
  package format's per-entry integrity primitive is itself a CRC32, so a package
  crafted to match another's per-entry paths, sizes and CRC32s still produces the same
  identity. **Closed under task 0.29**, which needed neither the startup cost nor the
  format change assumed here: the install already holds the whole package in memory, so
  the digest is taken there and recorded.

  Not re-reviewed: the fixes from the second and third rounds landed after the round
  that found them.

  What has to be threaded: `resolve_code_path` returns a `ResolvedPath` that carries
  no backend identity today, so the pack branch of `worker_image_source` has nothing
  to fold in. Carrying a `u64` digest of the identity on `ResolvedPath` is enough and
  keeps `MountBackend` free of a package-specific return type; the trait would need a
  default-returning-`None` accessor for it. Verified while scoping: production really
  does construct `PackSource` — `install_package` and the installed-subpackage mount —
  so this is not a latent path waiting on a future backend.

- [x] 0.19 Give shared budgets per-session accounting. **Design corrected before
  implementation** — the obvious reading of this task was wrong and would have made
  things worse. Governed by new specification Section 6.5.

  **All four shared budgets are done**, and no two took the same treatment, which is
  Section 6.5's tiers falling out of what each one holds: the image cache kept its
  sharing and gained per-entry ownership, the Skia budget's denominator moved to the
  scope its numerator already had and its trim ceiling became transient, the code cache
  kept its sharing and its budget moved to the directory that owns the bytes, and the
  audio worker gave its CPU work away. Reading each of the four as "split it per
  Session" would have been wrong for three of them.

  What is left is recorded rather than open: two processes sharing one code-cache
  directory still get a budget each; whether `trim_resource_cache` installs the low cap
  before restoring the ordinary one is unobserved for want of a GPU; and whether sharing
  Skia `Typeface` objects pays is a measurement nobody has taken. Tasks 0.34 and 0.20
  carry the two follow-ups that are work rather than residue.

  The plan implied by task 0.16 was to split shared caches per session the way the
  text texture cache was split. For the image cache that is the **wrong** fix: its
  entries are decoded RGBA bytes, which are context-independent, and its key is
  disambiguated by a generation token hashing the resolved real path together with
  the file's size, mtime, and mount origin. So two games loading the *same* asset
  already share one decoded copy today, and two games whose virtual `/code/logo.png`
  are different files already do not collide. Splitting per session would have
  destroyed that sharing and duplicated memory for nothing. The text cache had to be
  split for a reason that does not generalise: its entries hold GL texture names,
  which mean nothing outside the EGL context that minted them.

  **The acute defect is fixed, and it turned out the two caches want opposite
  treatment** -- which is Section 6.5's tiers falling out of what the entries hold.

  `migo_io::global_cache()` holds decoded RGBA and its key carries real identity, so
  the `clear()` calls in `Host::drop` and in `on_restart` were **removed**. They were
  not merely harmful but unnecessary: the invalidation they appeared to provide is
  already in the key, which the existing token tests cover
  (`extensionless_primary_size_change_invalidates_token` and its siblings) -- a
  changed file yields a different key. So one game exiting or restarting no longer
  discards every other running game's decoded images, a later session reusing the
  same unchanged file gets them for free, and total memory stays bounded by the
  cache's own LRU byte budget exactly as before.

  `runtime_v8::IMAGE_CACHE` is the opposite case and its clear was **kept**. It maps
  src to a shared image id, and those ids name GPU textures in an EGL context that is
  gone once the Session ends, so it is tier one: leaving a dead Session's texture ids
  reachable is worse than over-clearing. It is still process-wide, so that clear does
  still drop other live Sessions' aliases -- the remaining half of this task, and the
  reason the two caches must not be confused for each other again.

  `crates/core/src/runtime/tests/session_teardown_caches.rs` pins all three
  statements, and is honest about being source-pinned: proving the behaviour needs two
  live `Host`s, which need surfaces and a GPU, while "this call must not be here" is
  structural and is what a source assertion can express. Each of the three tests was
  mutation-checked to fire for **its own** defect only.

  Two test-precision defects were found and fixed while doing that, both the kind
  that make a test pass for the wrong reason. Searching for `on_restart` matched a doc
  comment 1300 lines above the function, so that test's window swallowed the whole
  `Drop` body and it failed for `Drop`'s mutant rather than its own. And bounding the
  window with `expect("followed by another method")` panicked instead of asserting,
  because `on_restart` is the last method in its impl.

  **Two of the remaining defects are reachable from content**, which makes them
  isolation problems rather than only accounting ones. `ImageCache.clear()` in
  `01_image.js` reaches `op_clear_image_cache`, which clears the process-wide decoded
  cache — so game A's script evicts game B's images. `ImageCache.getStats()` reaches
  `get_image_cache_stats`, which returns process-wide entries, bytes, hits and misses
  — so game A can observe game B's asset-loading behaviour. Both must become
  session-scoped, and neither can be until entries carry Session attribution.

  **Execution order, and why.** The two halves interlock, and doing them in the wrong
  order means threading a Session id through call sites twice.

  1. **Partition `runtime_v8::IMAGE_CACHE` per Session first.** **Done.** It is tier
     one regardless — its entries map src to a shared image id naming a GPU texture in
     one Session's EGL context — so this was required work, not a means to an end.
     Doing it first also gave every migo_io `pin`/`unpin` site in `runtime-v8` a
     Session context for free, because those sites sit alongside the alias bookkeeping
     being partitioned.

     Built as designed: a `RwLock<HashMap<i32, Arc<Mutex<ImageCache>>>>` registry with
     `image_cache_for_host` / `unregister_image_cache`, resolved **once** at isolate
     bring-up into op state so no per-event path reads the registry. Teardown drops
     the Session's entry with no texture-destroy dispatch, because `render.shutdown()`
     has already joined the thread that owned the context those ids named. Restart
     drains this Session's ids and keeps the registration, since the isolate it is
     about to build resolves the same handle.

     **Four premises of this plan were wrong, and each changed the work.**

     The **inventory was incomplete in the way that mattered most**: besides the sites
     named here there were two more in `rendering/webgl/webgl.rs`, and both sit on the
     WebGL texture-upload path that runs per frame. So the process-wide `Mutex` was
     not only an isolation defect but already a Section 7.3 violation — one game's
     `texImage2D` could wait on another game's image load. It is now each Session's
     own lock. A third site there, `load_cached_image_rgba`, had no caller anywhere in
     the tree and was deleted rather than threaded.

     The op state did **not** need a new extension. `host_v8_image`'s own `state`
     closure carries the handle, and `extension!`'s generated `args()` propagates the
     state function to the snapshot-restore path by itself. A separate extension had
     to be added to two snapshot lists instead, which
     `snapshot_extensions_match_main_runtime_order` and its worker twin exist to
     police — churn in a delicate ordering for no gain. Those two tests caught the
     first attempt, which is what they are for.

     Teardown must **still release the io-side pins** even though it must not clear
     the io cache. Those read as contradictory but sit at different layers: the
     decoded bytes are shared and survive the Session, while the pin asserting "a
     live alias needs these bytes" dies with it. Since the io `clear()` was removed
     one commit earlier, dropping the alias table without unpinning would have turned
     every torn-down Session's entries into permanent residents of a bounded LRU — a
     worse leak than the one that clear was papering over.

     `HostStartupGuard` leaks this the same way it leaked the text cache, which is
     task 0.16's review finding recurring: a `Host::new` that fails after the isolate
     is built leaves a registration with no `Host` to remove it. The guard now
     unregisters both, and it drops after the runtime that would have registered them,
     so nothing can re-register behind it.

     `session_teardown_caches.rs`'s third statement was rewritten rather than kept: it
     required the process-wide clear to stay, and now requires the per-session drop to
     be present. Deliberately not strengthened with an "and not process-wide" half —
     the function takes a host id, so a process-wide drop is no longer expressible and
     such an assertion could not fail.

     **Mutation evidence.** Six behavioural tests sit beside the registry; three
     mutants, each reproduced faithfully only after a first attempt that was not:

     - *One process-wide table*, all four entry points keyed on one slot, which is the
       pre-change behaviour. Kills exactly the four isolation tests, each at its own
       named assertion. `both_isolates_of_one_session_reach_the_same_alias_table` and
       the pin test survive, correctly: neither is about partitioning. A first attempt
       mutated only the lookup, which left teardown removing a key that was never
       registered — a *different* defect — and killed five tests, one of them for the
       wrong reason.
     - *Teardown drops the table without draining it.* Kills exactly one, the pin
       test.
     - *A private table per resolve*, the way `io_state.rs` builds a private
       `IoScheduler` per isolate, which is the plausible mistake here. Kills three:
       `both_isolates`, the restart drain (the registry is never populated, so it
       returns nothing), and the pin test (teardown finds no table to unpin). A first
       attempt left the read fast path in place, so the second resolve still found the
       first's entry and the mutant killed **nothing**.

     A test-precision defect found while gathering that: the fixture asserted
     `begin_load` answered `StartLoading`, so under a shared table three tests died
     inside the helper and never evaluated the claim in their own name. The helper now
     asserts nothing, and each test asserts the state its own property needs.

     **Not covered, and stated rather than implied.** `ImageCache.clear()` still
     clears the process-wide decoded-bytes cache and `ImageCache.getStats()` still
     reports process-wide numbers: only the GPU-alias half of those two
     content-reachable defects is fixed. The pre-pin taken before decode is still
     unattributed, so a Session that dies mid-load still leaves it behind and `drain`
     cannot release what no entry records. Both are step 2. And the frame path's
     freedom from the registry lock is once again **structural**, which Section 7.3
     explicitly does not accept as satisfying its contention gate. Task 0.27 has since
     gated the *text* cache's frame path that way; this image-cache frame path still
     has no such test. No test builds two live `Host`s either; these exercise the
     registry and alias table directly.

     Verified at migo-runtime-v8 503 lib tests from a 497 baseline, migo-core 49,
     migo-io 245, migo-shared 373, migo-capi 141, migo-platform 50, Android 103 per
     flavour on Full and Slim with no failures, errors or skips, `cargo fmt --all
     --check` and `git diff --check` clean.

     **Independent code-quality review done, no findings**: the wiring was checked
     across the eager, snapshot, worker, WebGL, restart and teardown paths — the six
     that could each have been missed independently — and the suites re-run rather
     than taken from this record. Spec review still outstanding.

  2. **Then give migo_io's `ImageCache` per-entry Session ownership.** Store the owner
     ids in the entry rather than in a side index: entries hold megabytes of RGBA, so a
     small `Vec<i32>` per entry is free, while a `HashMap<i32, HashSet<Key>>` would
     clone every key's `String` per Session. `insert` and `get` record the asking
     Session; `release_session(id)` removes it from every entry's owners;
     `clear_for_session(id)` evicts only entries left with no owner, which is what
     makes `ImageCache.clear()` correct. Keep the existing aggregate
     `pins: HashMap<Key, u32>` as-is so the eviction, trim and admission checks stay a
     single lookup, and add attribution beside it rather than nesting it.

     This also closes the pin leak: a Session that dies mid-load leaves its pre-pinned
     entry immune to eviction **and** to `clear` for the life of the process, because
     pins are not attributed. `release_session` on teardown is the fix.

     **The pin leak is fixed, and attribution turned out to be the wrong instrument
     for it.** Step 1 changed the premise: each Session's alias table now records
     exactly which pins it holds, as one per `refs` unit on each entry, and its drain
     returns them all at teardown. So the *paired* pins were never the leak. What
     leaks is specifically the **pre-pin** taken before the decode, which no entry
     records, so no bookkeeping can return it — and what loses it is cancellation
     rather than accounting: two `.await` points sit between that pin and the load
     settling, and a Session torn down or restarted with a load in flight has the op
     future **dropped**, so the manual unpin on each explicit exit path never runs.

     The fix is therefore a `PrePin` guard that releases in `Drop`, which covers every
     exit including the one no explicit path can: cancellation. It is also smaller
     than attribution would have been — it removed four manual unpin sites and the
     `Option<ImageCacheKey>` that had to be threaded past two awaits to reach them.

     Worth stating because it explains which residue matters: cancellation leaves
     bookkeeping behind in two places, and only one is a reachable defect. The
     `loading_map` and `pending_alias_to_key` entries an abandoned load leaves live in
     the **Session's own** alias table, which the same teardown wipes, so they are
     unreachable. The pre-pin lives in the **process-wide** decoded-bytes cache, which
     outlives the Session and is deliberately not cleared, so nothing ever reclaims
     it. Residue only survives when it is held outside the structures the dying
     Session owns.

     Covered by `a_load_abandoned_mid_flight_releases_its_pre_pin`, which abandons a
     pending load through a timeout and requires the pin count back at zero; a `Drop`
     that releases nothing — the shape the code had before — kills exactly that test
     and nothing else. Not covered: the test pins the guard's cancellation semantics,
     while the claim that runtime teardown actually drops a pending op future is
     deno_core behaviour and is not observed here.

     Inventory of production sites: 8 in `crates/io/src/image_ops.rs`, 8 in
     runtime-v8's `image/mod.rs`, 7 in its `cache.rs`, 1 in `host.rs`. The
     `image_ops.rs` functions already take their dependencies as explicit parameters,
     so a Session id joins `scheduler`, `gpu_caps` and `mount_table` rather than
     needing new plumbing.

     **A third correction, and this one constrains the representation:** "entries hold
     megabytes of RGBA, so a small `Vec<i32>` per entry is free" is true of storage and
     false of access. `get` returns a **clone** of `CachedImage`, and it is called once
     per `texImage2D` on the frame path, where today that clone is an `Arc` bump plus
     two `u32`. Adding a `Vec<i32>` to the entry would put a heap allocation on that
     path, which is what Section 7.3's zero-steady-state-allocation rule forbids. So
     the owner list must not be part of what `get` hands back: `get` should return the
     `NormalizedImage` and `CachedImage` should stop being public.

     **Per-entry ownership landed, and both content-reachable defects are closed.**
     Entries carry a `Vec<i32>` of owners; `insert` records the Session that decoded
     the bytes and `get` records a Session that reads them, because reading an entry
     another game decoded is how sharing pays off and it makes this Session depend on
     those bytes too. `clear_for_session` drops the caller's claim and evicts only
     what no other Session still holds, so a game's script can discard what it holds
     and nothing else. `release_session` drops claims at teardown and restart and
     evicts nothing at all: the bytes are context-independent, so a later Session
     loading the same unchanged file should still get them free.

     Where the Session id comes from was the one thing that made this small rather
     than sprawling: `IoScheduler` is built per Session and already exposes
     `host_id()`, so every `image_ops.rs` decode path that already takes a scheduler
     needed **no new argument**. On the runtime-v8 side the id rides in
     `ImageCacheState` beside the alias handle, so the frame path resolves both in one
     op-state lookup.

     **`getStats()` semantics, decided with the user rather than assumed.** `hits` and
     `misses` are the Session's own lookups; `entries` and `size_bytes` cover the
     entries it owns, counting a shared entry's bytes **in full for each owner**;
     `max_bytes` is the one shared budget, reported as such. So two games holding one
     4 MB atlas are each told 4 MB and per-Session totals can exceed the resident
     total. Splitting the bytes would need an arbitrary rule, and under-reporting what
     a game depends on is the more misleading of the two errors.
     `shared_bytes_are_reported_in_full_to_each_owner` asserts that overlap
     deliberately, including that the resident total stays one copy.

     **Mutation evidence**, four mutants with four distinct kill sets, each test
     failing at its own claim:

     - *`clear_for_session` ignores ownership*, reproduced as the process-wide `clear`
       it replaced: kills the two clear tests. `a_pinned_entry_survives_its_owners_clear`
       correctly survives, because the old clear spared pins too.
     - *A read does not record the reader*: kills three — the shared-entry clear test,
       the shared-bytes test, and the teardown test — because B's claim on bytes A
       decoded is exactly what goes missing.
     - *`stats_for_session` returns the process figures*: kills two.
       `shared_bytes_are_reported_in_full_to_each_owner` does **not** discriminate
       here and is not counted as if it did: with one copy resident, the process total
       and the per-owner total coincide.
     - *Teardown evicts as well as releasing*, the plausible slip of reusing
       `clear_for_session` for `release_session`: kills exactly the teardown test.

     Not covered: no fixture drives this through two live `Host`s, so the tests
     exercise the cache directly rather than two games' scripts. And the frame-path
     cost of attribution is reasoned rather than measured — `get` compares against a
     one- or two-element `Vec` and allocates only the first time a Session touches an
     entry. The mechanism that would settle it now exists (task 0.26) but has not been
     applied here: `migo-io` installs no counting allocator yet. Worth doing sooner
     than later, because applying the same gate to the text cache's structurally
     identical pin map found a real per-frame key clone.

     **Independent review found a real hole in the guarantee this work adds, and it
     is fixed.** The owners were carried across a replacement *after* the eviction
     loop, and that loop can pop the very key being replaced when it is the coldest
     unpinned entry — two Sessions finishing a decode of the same image into a cache
     with no spare room is enough. The read then found nothing, the replacement kept
     only the later Session, and the earlier Session's bytes became evictable by the
     later one's `clear_for_session`: precisely the defect this commit exists to
     prevent, reintroduced in its own implementation. The owners are now carried
     across before any eviction, folded into the residency check that was already
     peeking at the same entry, and
     `a_replaced_entry_keeps_the_first_games_claim_even_under_pressure` fails against
     the previous shape.

     **A second finding, accepted in part rather than in full.** On restart a decode
     already running in a worker can insert after the claims are released, so the new
     isolate inherits a claim it never made. The recommendation was incarnation-aware
     owner identity; the assessment behind not building it: a host id names the
     **Session**, not the isolate, so such an entry is attributed to the same game
     that decoded it and cannot cross to another game. Neither the clear guarantee nor
     the statistics guarantee is broken — what is affected is the tidiness of one
     game's own figures across its own restart boundary, which is a diagnostic. The
     release is now ordered after `close_io_scheduler()`, which rejects the queued
     work and leaves only an already-running closure able to land late; that residue
     is recorded here rather than designed against.

     **A confirmation review then found something bigger, and it is not this task's:
     the decoded-image key collides across Sessions for pack-backed assets.** Verified
     rather than accepted — `MountTable::generation` counts mounts within one table,
     every Session owns its own, and a base mount is `mounted_at: 1` in all of them,
     so two games shipping different packages produce the identical key
     `("/code/logo.png", 1, 0, 0)` and the second is served the first's pixels. This
     predates the whole of 0.19; what changed here is only that a game can no longer
     recover by calling `ImageCache.clear()`, because the colliding entry keeps
     another Session as an owner and so survives. That recovery was itself the defect
     being removed — one game wiping another's bytes — and it never addressed the
     wrong pixels. Section 6.5 asserted this key was safe and has been corrected;
     the fix needs a globally meaningful package identity and is task 0.28.

     **Two corrections to that inventory, made by counting rather than trusting it**,
     since step 1's equivalent list was the premise that most changed the work. It
     omits `rendering/webgl/webgl.rs`, which reads the decoded bytes once per
     `texImage2D` in `resolve_cached_image_rgba` — if `get` is to record the asking
     Session then this frame-path caller has to carry one, and it is exactly the file
     step 1's inventory left out too. And the counts hold only for production code:
     `image_ops.rs` mentions `global_cache()` 32 times, 24 of them inside its own test
     module below the `#[cfg(test)]` at line 1332.

  3. Then the remaining shared budgets: the Skia resource budget, the on-disk code
     cache, and the single worker serving all audio streaming.

     **Memory-pressure trim no longer compounds per Session. Done.** A trim level now
     names the bytes the cache may *keep*, as a ceiling on its budget, rather than a
     fraction of whatever is resident when the call arrives. That makes the operation
     idempotent, so the second and third Session to relay one Android `onTrimMemory`
     find the cache already under the ceiling and free nothing — the level means the
     same thing however many Sessions relay it.

     Two things fall out that are worth stating, because neither was the motivation.
     A cache sitting well inside its budget is now asked for nothing at moderate
     pressure, where the old rule paid for a re-decode of a quarter of it to release
     bytes the OS was not short of. And the two branches of `trim` collapse into one
     loop, because "release everything" is just a ceiling of zero.

     `shared::text_texture_cache` was changed the same way even though it never
     compounded — it is per Session, so N relays trim N separate caches once each.
     The old reading was wrong there for the second reason rather than the first, and
     two caches responding to one signal should not interpret its levels differently.

     Mutation-proved on both sides: restoring "a fraction of what is resident" fails
     the repeated-relay test and the inside-budget test in `io::image_cache`, and the
     ceiling test in `text_texture_cache`. `background_pressure_still_empties_an_underfull_cache`
     correctly survives, since at full pressure both readings mean the same thing.

     Original note, kept for the record: `host.rs`'s `OnMemoryWarning` handler calls
     `global_cache().trim(level)`, and the signal reaching it is per Session —
     `GameSession.notifyMemoryWarning` passes its own `sessionId`, and a host app
     with two Sessions calls it once for each from a single Android `onTrimMemory`.
     So one pressure signal releases `1 - (1 - f)^N` of the shared cache instead of
     `f`: at `RunningModerate` and three games, about 58% rather than 25%. This one
     is **not** fixed by attribution, because trimming a cache that is shared on
     purpose is inherently a process-level action; the fix is to coalesce the signal,
     so it must not be folded into step 2's per-entry ownership work.

     **The Skia resource budget is fixed, and the defect was worse than this task
     recorded.** The note said only "remaining shared budget". Reading it showed the
     numerator and the denominator had different scopes: the budget is a process-wide
     atomic, while the divisor was `self.contexts_2d.len()` — one `CanvasManager`'s own
     count, and there is one manager per Session. So two Sessions each holding one
     canvas each divided the *whole* aggregate budget by one, and the process handed
     Skia twice the ceiling this module exists to enforce. N Sessions meant N times,
     against a stated 200 MB native-heap target.

     The count is now process-wide and carried by a `LiveContextCount` guard held as a
     required field of `Canvas2DContext`, so the compiler refuses to build a context
     that is not counted and no exit path can forget the decrement — including a
     construction that fails after the counter was raised. `rebalance_resource_cache`
     and `per_ctx_resource_cache_bytes` lost their `live_ctxs` argument, because a
     caller that can pass the wrong denominator is how this happened.

     **Convergence is lazy, and that is forced rather than chosen.** A Skia
     `DirectContext` may only be touched from the render thread that owns it, so a
     Session cannot rebalance another Session's contexts. A new context takes the
     smaller share immediately; already-live contexts elsewhere keep their larger cap
     until their own next canvas create or destroy. The overshoot is bounded by what
     those contexts had already been granted, and it is stated rather than implied.

     Mutation-proved: fixing the divisor at 1 reports 64 MiB where 32 MiB is due —
     the original defect's exact signature — and a guard whose `Drop` releases nothing
     reports the floor where the full budget is due. Each killed only the budget test;
     `the_budget_never_drops_below_one_context_s_floor` correctly survived both, since
     it does not depend on the count. Not covered: the guard is exercised directly
     rather than through `Canvas2DContext::new`, which needs an EGL context and a GPU.
     What that leaves out is only whether a context is enrolled at all, which is a
     compile error rather than a convention.

     **Still open here, verified rather than assumed:** one Session's `onTrimMemory`
     calls `set_skia_resource_cache_budget(low_memory_budget())`, which lowers the
     *process* budget to 16 MiB and is never restored — only engine init raises it
     again. So one game's memory warning permanently caps every other game's Skia
     budget for the life of the process. The fix is a transient ceiling rather than a
     stored budget, which is a design change and not a line edit.

     **The Skia trim ceiling is transient now, and the second half of that defect was
     the opposite mistake.** A warning is answered by squeezing each live context to
     `low_memory_per_ctx_bytes()` and restoring the ordinary share in the same call:
     installing a lower cap is what makes Skia purge, so the release lands inside the
     call and the cap does not have to stay behind to have had its effect.
     `low_memory_budget()` is gone rather than left unused — a function that yields a
     low figure to *store* is the instrument that caused this, so removing it is what
     stops the next caller. Both figures now come from one private `per_ctx_share`,
     which takes no count, so the divisor cannot be passed wrongly here either.

     The opposite mistake, found while reading the call sites rather than looked for:
     `CanvasManager::new` also writes that global, with `tier_budget(tier)`, and it
     runs per Session — so a second game starting *raised* a still-relevant low ceiling
     back to the tier's. With nothing lowering the global any more, that write is
     harmless: every Session computes the same device tier, so the store is idempotent.
     A repeated relay is idempotent for the same reason the image cache's is: a squeeze
     to a fixed ceiling finds nothing left to free the second time.

     Mutation-proved: restoring the stored-budget shape — `low_memory_per_ctx_bytes`
     setting the process budget and reading the share back — fails
     `a_low_memory_squeeze_leaves_no_ceiling_behind` at its own named assertion and
     kills nothing else. `the_per_context_cap_divides_the_budget...` and
     `the_budget_never_drops_below_one_context_s_floor` both survive, correctly:
     neither is about what a warning leaves behind. Not covered, and stated rather than
     implied: whether `trim_resource_cache` really installs the low cap before
     restoring the ordinary one is unobserved, because a `DirectContext` needs an EGL
     context and a GPU. What is covered is that no path can store the low figure. The
     one assertion lost with `low_memory_budget` was `low_memory_budget() >=
     MIN_PER_CTX_BYTES`, which only restated the `.max` in its own body.

     **The on-disk code cache had the Skia defect's exact shape, and reading it found a
     third consequence the note did not name.** `MAX_CACHE_SIZE` is 32 MB and
     `DiskCodeCache` tracks the directory's size incrementally — but one instance was
     built per Host while the directory comes from `MigoEngineConfig.code_cache_dir`,
     which is per Engine. Verified rather than assumed: `capi/src/surface.rs` passes
     `session.engine.code_cache_dir` into every Session's `InitOptions`. So each
     Session scanned the directory once and then counted only its own writes: the
     ceiling admitted N x 32 MB, one Session's eviction deleted files another's counter
     still claimed so that counter over-counted and over-evicted, and — the one the
     note missed — two Sessions could write and read one path at once, so a Session
     could be handed half of another's write. That last one is not exotic: both
     Sessions load the same engine extension JS, so they compile the same sources and
     collide on the same hashes by design.

     **The directory owns the cache.** `create_code_cache` hands back the instance that
     directory already has, keyed on the resolved path so two Engines configured with
     one directory spelled two ways cannot get a budget each. `DiskCodeCache::new` is
     private, which makes an uncounted second cache over one directory unexpressible.
     The registry holds `Weak`, so the cache dies with the last Session holding it and
     the next one rebuilds it, opening scan included.

     Splitting it per Session was **not** an option and the reason is Section 6.5's
     tiers rather than convenience: compiled bytecode for a source is the same bytes
     whichever Session compiled it, so this is the shared tier, and the key is the
     source's own hash. What was wrong was the accounting.

     Where the lock landed is the part worth stating. `get` takes the directory guard
     **shared** and `set` takes it exclusively, so two Sessions starting at once read
     their modules concurrently while nobody reads half of a write — the torn read
     needed no fix before this, because before this no two Sessions shared one counter's
     worth of state. The incremental counter stays: a scan per write is O(N) in a
     directory a game start walks tens of times.

     Mutation-proved, three mutants with three distinct kill sets:

     - *A cache per Session over one directory*, the pre-change shape: kills
       `two_sessions_on_one_directory_share_one_budget` and nothing else, reporting the
       defect's own signature — 40 MiB resident against a 32 MiB ceiling.
     - *One cache for the process whatever directory was asked for*, the plausible
       over-correction: kills `two_engines_with_different_directories_do_not_share_one`,
       and also the reopened-directory test, which is a real second consequence rather
       than a misattribution — a Session handed another directory's cache cannot count
       what is in its own.
     - *A new cache starting its counter at zero instead of scanning*: kills
       `a_directory_reopened_after_its_last_session_counts_what_is_there` alone.

     **Not addressed, and stated rather than implied.** Two *processes* pointed at one
     directory still get a budget each, because nothing takes an OS-level lock on the
     directory; what keeps that bounded is the drift correction already in
     `evict_if_needed`, which replaces the tracked figure with the measured one whenever
     a scan finds the directory under the ceiling. The 64-bit `DefaultHasher` key is
     left as it is, on the same terms Section 6.5 accepted for the decoded-image key: a
     second preimage costs on the order of 2^64.

  Also here: `render_diagnostics::set_text_cache_gauges` was recorded as a
  process-global accumulator that interleaves two Sessions' gauges into one
  meaningless number. **That premise is now stale**: the module publishes through a
  per-render-thread accumulator into a per-Session sink, and there is one render
  thread per Session. What was missing was the evidence, and specifically for gauges:
  `sinks_are_isolated_by_thread` covers *counters*, which publish with `fetch_add`, so
  a defect that merged the gauges alone would leave it green — gauges publish with
  `store`, where merging means whoever flushed last speaks for both.

  `a_gauge_set_before_another_session_sets_its_own_still_reports_its_own` closes that,
  with the four phases ordered by a barrier rather than raced: *set A, set B, flush A,
  flush B*. Two threads setting at once would catch a merged gauge only when the
  interleaving happened to cross, which is a coin flip. Mutation-proved by moving the
  gauge value into a process-global read at `drain` — A's flush then publishes B's
  4096 where 8192 is due. It killed only this test; the counter isolation test and the
  single-threaded gauge test both survived, which is the evidence that it sees
  something neither of them can. A first attempt at that mutant stored to the global
  and read it straight back in the same call, so the value never actually escaped its
  thread and the mutant walked.

  **The audio streaming worker is fixed, and reading it had already changed what needed
  fixing.** `streaming::STREAM_RUNTIME` is a process-wide tokio runtime built with
  `worker_threads(1)` and shared by every Session, and the MP3 decode
  (`decoder.push_data` / `decode_available`) ran *inline* in the async download task.
  So the defect was not merely that the worker is shared: CPU-bound decode occupied the
  single worker, and while it ran no other Session's download task could be polled at
  all. Neither gate built for tasks 0.26 and 0.27 observes this — it is not a lock and
  not an allocation.

  **The evidence needed a third mechanism, and designing it was the whole of the work.**
  `engine/testing/executor-probe` spawns the step under test onto the shared runtime,
  waits for it to announce that its CPU-bound body has begun, and only then spawns a
  co-tenant task that must run while the step is still in flight. Timing is not the
  observation: the step *blocks* until the co-tenant releases it, so the co-tenant's
  progress is a precondition of the step finishing rather than something measured
  against a clock. Four properties make it a gate rather than a decoration, and two of
  them were found by the mechanism's own tests failing:

  - **The bound lives inside the step.** The defect's signature is a deadlock, not a
    wrong value, so a naive test hangs the suite instead of reporting. When no release
    arrives the step gives up, the future completes, and the failure is named.
  - **The step records the release it received**, rather than the gate reading a flag
    afterwards. Mutation-proved, and this is the decision the whole mechanism rests on:
    with the co-tenant recording instead, *both* inline-step controls pass, because
    under the defect the co-tenant runs the instant the step stops occupying the
    worker. The gate would have been unable to fail on the very thing it exists for.
  - **Every worker but one is occupied first.** A gate that assumed a single worker
    would pass an inline step on a two-worker runtime — the same defect with more room.
    Mutation-proved: dropping the fillers kills only the multi-worker control.
  - **The fillers wait unbounded**, released by the gate dropping their senders. The
    first version gave them the same deadline as the step, and the multi-worker control
    failed: a filler timing out freed a worker at the very moment the step was still
    waiting, so the co-tenant ran and an occupying step passed.

  The fix keeps memory flat, as planned: the decode step moves off the shared worker
  with `spawn_blocking`, the decoder moving in and out, so nothing is queued and
  nothing is buffered — the chunk order and the backpressure are exactly what they
  were, and the shared worker becomes pure I/O. tokio's blocking pool is elastic, which
  is why this is a fix rather than a relocation of the bottleneck: two Sessions decoding
  at once get two threads, where they were sharing one worker.

  **What carries the guarantee is a type, not a convention.** `OffWorker<T>` owns the
  decoder, its field is private and it has no accessor, so the download task cannot
  reach the decoder except inside `with`, whose step runs on a blocking thread. Inline
  decode is a compile error — the instrument `LiveContextCount` uses for the Skia
  budget. The value moves in and comes back only on success, so a step that panicked
  leaves no wrapper a caller could ask again, and the poisoned state is unrepresentable
  rather than handled.

  Mutation-proved, two mutants with one kill each, and the first attempt was too wide.
  *The step runs inline on the worker* — the pre-change shape — killed the gate **and**
  the panic-reporting test, because `spawn_blocking` supplies both properties at once;
  narrowed with `catch_unwind` so only the occupancy changes, it kills
  `the_decode_step_leaves_the_shared_streaming_worker_free` alone, at its own
  assertion. *A panicking step is re-raised on the caller's thread* kills the panic
  test alone.

  Honestly reported: `what_a_step_changed_is_what_the_next_step_sees` cannot be killed
  by any mutant of this implementation, because by-value move makes the round trip
  structural — there is no way to hand back a value the step did not mutate without a
  `Clone` or `Default` bound the type does not have. It is kept for the `&mut self` plus
  `Option` variant it would catch, which is the refactor someone reaches for to avoid
  the `decoder = rest;` line. Also not covered: the loop's own control flow, since
  `streaming_download_task` needs an HTTP response; what closes that gap is the privacy
  above rather than a test.

  The gate had to be given somewhere to run. `migo-audio`'s suite ran in neither
  `verify-change.sh` nor CI — the gap task 0.32's entry point was built to close, still
  open for one crate — so both now run it, and the contract script's CI-parity check
  covers the new probe as well. It is in the ALSA-installing CI job, because cpal links
  ALSA.

  One thing checked and found *not* to be a defect: `GlobalAudioCache` is constructed
  inside the audio thread body, so despite the name it is per Session, not shared.

  Unmeasured, deliberately not assumed: Skia `Typeface` objects are CPU-side and
  parsed per render thread, so N games parsing one font parse it N times. Whether
  sharing them pays is a measurement; the GPU glyph atlas built from them stays
  per-session regardless.
- [x] 0.20 Resolve Engine-scoped storage roots. `MigoEngineConfig` takes the
  file, cache, and code-cache roots per Engine, so a single-Engine host cannot
  give two games different roots. Either document the shared root as intended or
  move the roots to Session scope.

  **Documented as intended — and the three roots reach that answer by three
  different arguments, which is why the task insisted they be argued separately.**

  `code_cache_dir` decides itself: Section 6.5 *requires* the on-disk V8 code
  cache to be shared, its key is `hash(source_bytes, v8_version)` so an entry means
  the same thing whichever Session compiled it, and task 0.19 moved its budget onto
  the directory precisely because the directory is one. Session scope would not
  relocate that cache, it would retract it — each Session paying for its own copy
  of every compile, and the ceiling that now bounds the directory bounding nothing.

  `files_dir` and `cache_dir` decide themselves too, from what they are: the host
  application's own directories, granted to it once by the platform. Android gives
  one `Context.getFilesDir()` per app, iOS one `NSDocumentDirectory`; there is no
  second one to hand a second game. Session scope would add a *way to get it
  wrong* rather than a capability — a host can already give two Sessions one root,
  and would then also have to be trusted not to give them one content id. The host
  that genuinely needs two volumes creates a second Engine, which the ABI has
  always allowed and `two_engines_account_for_their_own_sessions` already executes.

  **The obligation this leaves on the host was unstated, and that was the real
  gap.** Isolation below the root is `<root>/migo/games/<content_id>/…`, the
  content id comes from `MigoContentDescriptor`, and nothing checks it for
  uniqueness — so two concurrently live Sessions given one id share one game
  directory: storage, cache and temp alike. The header now says so where the roots
  are declared. Refusing a duplicate is deliberately not done: two Sessions of one
  title is a legitimate thing for a host to want and the engine cannot tell that
  from a mistake, so the honest move is to name the contract, not to guess.

  **Executed rather than only written**, because Section 6.4 says a property never
  executed may not be claimed. The single construction site in
  `capi/src/surface.rs` became `EngineInner::session_init_options`, and
  `concurrent_sessions.rs` now runs two Sessions of one Engine and asserts all
  three roots come back exactly as the host named them. One place by construction:
  there is no second site for a per-Session override to be added to and missed at.
  Two mutants, each killed at its own assertion — deriving a subdirectory
  (`files_dir.join("migo")`), which is the "never invents a location" clause, and
  the copy-paste the extraction invites, `code_cache_dir` reading `cache_dir`.

  **A third test was written and then deleted, and the reason is worth keeping.**
  It asserted that two Engines' Sessions never carry each other's roots. Every
  mutant that killed it killed the roots-are-verbatim test first, because "each
  Engine's Sessions get that Engine's configured roots" already forces two
  differently configured Engines apart — no mutation makes one pass while the other
  fails. Two guards on one case pin it no better than one.

  **Still not executed, and not claimed:** the identity half of the split. A
  Session from `migo_session_create` has no surface, no Host and no bound game
  identity, so what these tests reach is the roots, and `GamePaths`' own
  storage-isolation tests in `runtime-v8` reach the `<content_id>` partition below
  them. Nothing yet drives two live Sessions through content to two directories on
  disk; that stays task 0.21's open half.
- [ ] 0.21 Add the first behavioural two-session tests. **First increment landed;
  two of the four property groups still uncovered.** The opening premise is now
  false in the good way: `engine/crates/capi/src/concurrent_sessions.rs` creates two
  concurrent Sessions through the real C API, where before this no test anywhere
  created a second one, so a reintroduced process-global would not have failed
  anything.

  Covered, and mutation-proved: two Sessions coexist on one Engine as distinct
  handles; Engine destruction is refused while *either* is live and succeeds only
  after the last one goes; destroying one Session leaves the other usable and a
  replacement creatable; two Sessions are created **and** destroyed concurrently
  from two host threads, with a `Barrier` so the operations genuinely overlap rather
  than merely running on different threads; and two Engines account for their own
  Sessions, so one Engine's live Session does not block the other's destruction.
  Removing the `live_sessions > 0` refusal in `migo_engine_destroy` fails exactly
  three of the five and leaves the two that do not assert it — so they test the
  guard rather than coexist with it.

  The public header now states the guarantee affirmatively where
  `migo_session_create` is declared, rather than leaving it to the absence of a
  prohibition: any number of live Sessions per Engine, more than one Engine per
  process, per-Session isolate and registries and permission monitor and storage
  root, and that two Sessions may be driven from two host threads while calls
  through a *single* Session stay the host's to serialise.

  **Two property groups were recorded here as needing a surface. That was the wrong
  reason, and re-reading it is what found the gap that was real.** The claim was
  that isolate separation and storage/quota separation need "a Session driven far
  enough to start content, which needs a surface". True of *this* layer —
  `migo_session_create` yields a Session with no Host, and `migo_session_load_content`
  refuses without a render target, so asserting either from the C API would be the
  inspection-as-test mistake. But it does not follow that a surface is what a test
  needs; what it needs is the layer that can see the property, and that layer
  already exists. `tests/published_namespace_isolation.rs` boots a real `JsRuntime`
  — a real V8 isolate — in a host test with no surface, no Host and no C API at all.

  Reading each group at that layer:

  - **Storage separation is covered**, in `tests/storage_isolation.rs`:
    two game ids resolve to non-overlapping roots, neither contains the other,
    `storage_dir` asks `game_paths` rather than the host app's directory, and a
    missing game fails rather than falling back. The Session-level wiring above it
    is covered by task 0.62's two live Sessions rather than by construction as this
    entry once said.

    ~~Quota is not a shared pool either: `MAX_TOTAL_BYTES` is passed to each
    storage op *alongside the directory* and enforced inside that file's SQLite
    transaction, so it is per-root by the same fact that makes the roots separate.~~
    **That sentence was reasoning, not coverage, and task 0.62 was right to name quota
    as untouched.** The reasoning is correct and it was never executed: no test filled
    a quota, so nothing distinguished "each game gets 10 MB" from "the games share
    10 MB", and distinct directories do not settle it — the store handles live in a
    process-wide `HashMap` in `storage_ops`, and a shared running total or a cache key
    that lost the directory would leave two directories in place while making one
    game's writes count against the other's budget.

    **Now executed.** `one_game_exhausting_its_quota_leaves_the_other_game_its_own`
    extends task 0.62's fixture: two live Sessions, a real isolate and a real
    `evaluate_module` each, the *same* app directories so the game id is the only
    thing that can separate them, and `game-a` filled through the production
    `storage_set` under the shipped `MAX_TOTAL_BYTES` until it is refused. The
    refusal is asserted, and asserted to be the quota's rather than any other error,
    because a fixture that never reached the limit would say nothing about sharing
    it. Then `game-b` writes and must be admitted — and both stores' byte totals are
    asserted, because "b's write succeeded" is also satisfied by a shared store that
    happened to have room.

    | Mutant | Kills | Also kills |
    | --- | --- | --- |
    | The store-handle cache key loses the directory | this test, at the neighbour's write (`:403`) | 4 `migo-io` tests |
    | The quota admits twice its limit | this test, at the exhaustion control (`:390`) | 1 `migo-io` test |
    | The shipped `LIMIT_SIZE_KB` drops from 10240 to 1024 | this test, at the exhaustion control | **nothing** |

    The first two are the same policy seen at two levels rather than two guards on
    one case, and saying so matters: on this project's own rule, a test killed only
    by mutants that also kill another is redundancy. The third is the case only this
    test can see — `migo-io`'s quota tests pass their own 1 KiB limit in, so the
    number each *game* actually gets is invisible to them, and the whole point of the
    claim is that it is 10 MB per game rather than per process. Its sibling
    `two_live_sessions_resolve_storage_under_their_own_game_id` survives all three,
    which is what says the namespace half and the accounting half are different
    claims.
  - **Isolate separation is a property of `deno_core`**: a `JsRuntime` owns its
    isolate and two of them cannot share one. What is worth testing is not that, but
    what two Sessions reach *around* their isolates — and there the audit found one
    genuine hole, which is now closed (task 0.55).

  **Permission independence was the third group, and its recorded reason was wrong
  the same way.** "The Java gate's own tests cover a single gate with several session
  ids, which is not the same as two Sessions" — but `PermissionOperationGate` is a
  process-wide `static` in `NativeExports`, keyed internally by session id, so a
  single gate holding several ids *is* the production topology and there is no other
  gate for two Sessions to have. Its tests already drive two sessions in several
  ways: `closingOneSessionLeavesAnotherLiveSessionUntouched`,
  `twoSessionsAdmitCallbacksConcurrently`,
  `perEventSessionLookupTakesNoLockSharedAcrossSessions`, and the id-ordering pair
  from task 0.23.

  What was genuinely missing was one direction: every one of those grants a scope and
  then checks the *granted* session still works, and nothing checked that the session
  which was **not** granted is refused. Closed under task 0.58.

  A footgun worth remembering: closure capture is per-field since edition 2021, so
  reading `wrapper.0` inside a `thread::spawn` closure captures the raw pointer and
  the `unsafe impl Send` on the wrapper never applies. The threads take the pointer
  through an accessor method, which captures the wrapper.
- [x] 0.55 Pin the decoded-image cache's game scoping on the branch that had
  none. Found by auditing task 0.21's remaining property groups. The decoded-image
  cache is the one process-global structure two Sessions both reach that holds
  their *content*, so what keeps one game's pixels out of another's is entirely the
  cache key.

  **And for a directory-mounted `/code` asset the key's path component is the
  virtual string, not the real one.** `resolve_local_src` returns
  `path: effective_src` there — `/code/logo.png`, byte-identical for both games —
  so separation rests entirely on the source-version token, which happens to hash
  the real path behind the mount. The other branches do not have this shape: the
  VFS fallback and the `/user`, `/cache`, `/tmp` roots all carry the resolved real
  path in the key itself.

  **The codebase already knew the hazard and had fixed the other half of the same
  branch.** `ResolvedCode::source_identity` carries it in its own doc: that
  `source_mounted_at` is *not* usable across Sessions because "it counts mounts
  within one `MountTable`, every Session owns its own, and a base mount is `1` in
  all of them", and that "any cache shared between Sessions must key on this
  instead". Task 0.28 applied that to the pack-backed case. The real-path case
  satisfies the same rule by a different route — the file's own location, which is
  strictly stronger for its case — and **nothing said so and nothing tested it.**

  `two_games_do_not_share_a_cache_entry_for_an_identical_asset` does now: two
  games' own code directories, the same virtual path, **identical bytes and
  identical mtimes**. That fixture detail is the test. Left to the filesystem the
  two files would differ in mtime, the keys would differ for a reason that has
  nothing to do with isolation, and a token that had stopped hashing the real path
  would walk straight through — the non-discriminating-fixture failure this plan
  has recorded twice before. Two titles from one publisher shipping the same logo,
  unpacked by the same reproducible extraction, is that fixture in production. The
  paired second assertion is the control the first needs: a token that varied per
  call would satisfy "the keys differ" while destroying the cache it exists to key.

  **Mutation evidence.** Both kills are at the test's own assertion, and both leave
  518 other tests passing — so this is a single load-bearing guard rather than one
  of two, which also answers whether `source_mounted_at` helps: it does not.

  | Mutant | Kills |
  | --- | --- |
  | The version token stops hashing the real path | `two_games_do_not_share_a_cache_entry_for_an_identical_asset` alone |
  | The real-path branch keys on `source_mounted_at`, the substitution its own doc warns against | the same test alone |

- [x] 0.56 Say what the presentation path actually does, and give each bypass
  condition a test of its own. Section 7.3's "no redundant presentation copy" was
  recorded as device-blocked. Half of that is true — `platform/src/{windows,ohos}/
  presenter.rs` have never compiled on this machine (ledger 0.32) — but the Linux
  path is reachable, and reading it found the requirement **unmet for ordinary
  content**, which no amount of device access would have told us.

  The onscreen canvas renders into a Chromium-style `DrawingBuffer` FBO, blitted to
  the window before every `eglSwapBuffers`. A bypass exists that redirects the
  default framebuffer to real FBO 0 and skips the blit, gated on four conditions.

  **Measured, because the engine already logs the transition and the log was one
  field short of being an instrument.** The transition now carries its four inputs
  alongside the verdict, and with that the bunnymark bundle answers immediately: it
  reaches `bypass = true` at startup, drops to `false` when a second canvas appears,
  and **presents its whole 60 fps steady state through the blit** — `canvas_count=2`,
  everything else favourable (`needs_default_fbo_readback=false`,
  `onscreen_has_2d_context=false`, `onscreen_db_matches_surface=true`). At 720×1280
  that is ~3.7 MB read and ~3.7 MB written per frame, about 440 MB/s, for a copy
  whose only purpose is disambiguation. A Cocos UI, which gives each text label its
  own offscreen canvas, is in the same state permanently.

  **The condition is sound, which is why this task did not delete it.** Bypass makes
  the onscreen default framebuffer *real* FBO 0, and real FBO 0 is whichever EGL
  draw surface is current; offscreen canvases are `SurfaceKind::Pbuffer` with
  surfaces of their own and the render path switches between them batch by batch, so
  with a second canvas "FBO 0" stops naming the window. The `DrawingBuffer` removes
  the ambiguity because its FBO is a name in the shared context that does not move
  when the surface does. **None of that was written down**: the other three
  conditions carried a paragraph each, and this one carried "bypass is safe when
  there is exactly one canvas" with no argument — which is why it read as arbitrary
  and why the cost went unnoticed.

  **Each condition now has a test of its own.** Two of them shared one, so deleting
  either failed a test that covered both and neither was individually pinned — the
  aggregate-assertion shape this plan warns about. Mutation, each leaving 553 other
  tests passing:

  | Mutant | Kills |
  | --- | --- |
  | Drop `canvas_count == 1` | `an_offscreen_canvas_disables_bypass_because_fbo_zero_stops_naming_the_window` |
  | Drop `!needs_default_fbo_readback` | `a_latched_default_fbo_readback_disables_bypass` |
  | Drop `!onscreen_has_2d_context` | `onscreen_canvas2d_context_disables_bypass` |
  | Drop `onscreen_db_matches_surface` | `onscreen_db_smaller_than_surface_disables_bypass` |

  **Not covered, named rather than implied.** The blit itself cannot be observed by
  a host test — it needs a live GL context — so what observes it is the engine's own
  log, and what asserts anything is the pure condition function. The existing
  guards over `swap_buffers_no_restore` in `present_damage.rs` are **source-text
  inspection**: they read the function's body as a string and look for
  `.swap_buffers(` and `blit_succeeded`. They catch a reordering and they cannot
  fail on a behavioural change that leaves the text intact — the same
  inspection-wearing-a-test's-clothing shape Section 7.3 names, sitting on the
  presentation path. ~~Replacing them needs a GL context and is not attempted
  here.~~ **That last sentence was wrong, and task 0.61 replaced them.** The blit
  needs a GL context; the *bookkeeping either side of it* does not, and taking the
  swap outcome as an argument instead of an early `?` return makes both branches
  reachable with no surface at all. Reading it as "the effect needs GL, so the
  ordering cannot be tested" is the same conflation that let 0.57's dead bypass path
  stay dead. ANGLE and `OHNativeWindow` are untouched for the reason 0.32 gives; the
  Android Surface path compiles but is unmeasured.

- [x] 0.57 Make the DrawingBuffer bypass path present at all. Planned as "sharpen
  the `canvas_count == 1` condition, or record why it cannot be"; the enumeration
  the plan asked for found the path the condition gates was **dead**, so the
  sharpening is deferred and this task is the repair.

  **The recorded plan named the wrong thing to check, for the fifth time in a
  row.** It proposed hoisting `make_current_needed(cmd.touches_canvas())` to the
  top of `handle_command` so the invariant held by construction. `touches_canvas()`
  cannot carry that: its own doc says "exhaustive over every variant that carries a
  `canvas_id` field", and 29 of 118 such variants are missing — `TexImage2D`,
  `TexSubImage2D`, `TexStorage2D`, the whole `Uniform*` family,
  `FramebufferTexture2D`, the compressed uploads, and every
  `TexImage2DFrom{Canvas2D,Shared,Snapshot,TextCache}`. The hoist would have
  *deleted* make-current from 29 command families, which is exactly the
  draw-lands-in-the-wrong-surface fault the bypass condition exists to prevent.
  Split out as task 0.59; it is a live defect on two other consumers.

  **What the enumeration actually found.** Ask "which framebuffer is this canvas's
  WebGL default?" and three sites answer independently. The post-swap restore
  applied bypass in a bespoke `if !bypass` and was the only one that did;
  `make_current_needed` and the surface-recreate DrawingBuffer reuse both derived
  `drawing_buffer.map(|db| db.fbo)` with no bypass term. And a fourth site was
  missing entirely: a bypass mode change moves *what the default framebuffer means*
  with no `bindFramebuffer` from the content, and nothing re-pointed the binding.
  So the onscreen canvas kept the DrawingBuffer bound from its own creation — which
  deliberately leaves it bound — drew into it, and skipped the only blit that would
  have carried it to the window.

  **Measured, because the property is a driver binding and no host test can see
  one.** `scripts/fixtures/bypass-probe` holds all four bypass conditions for a
  whole run; every shipping bundle breaks `canvas_count == 1` within a second of
  startup, which is why bypass had only ever run for warmup frames nobody looked
  at. It painted 240 frames of `rgba(51,204,102,255)` and the captured frame was
  `rgba(0,0,0,0)` on every sampled pixel. `blit-probe` — the same fixture plus one
  more `createCanvas()`, so `canvas_count == 2` and bypass never latches —
  presented the colour. After the fix, both do.

  `scripts/verify-bypass-present.sh` is the gate, and its shape is the point. The
  frame count is asserted beside the pixel because **240 frames in the wrong buffer
  is the same count as 240 frames on the window**: the pixel alone is satisfied by
  a run that never painted, the count alone by a run that painted into a buffer
  nobody reads. Which path ran is read from the engine's transition log, not from
  the fixture's intent, so a fixture that failed to create its second canvas is not
  scored against the path it meant to take — that happened on the first run, when
  `migo.createOffscreenCanvas` turned out not to exist. `blit-probe` doubles as the
  control on the instrument: if the player, the capture or the PNG decode were at
  fault both probes would fail, and the fault would not be bypass. The gate builds
  the player unconditionally rather than if-missing, because a mutation run leaves a
  binary compiled from the mutant beside a restored tree and WSL2 preserves mtime.

  **Mutation evidence. Six mutants, each attributed, file byte-identical
  (sha256, not `git diff`) after every restore.**

  | Mutant | Kills |
  | --- | --- |
  | `default_framebuffer_of` ignores bypass | `bypass_resolves_the_default_framebuffer_to_the_window` |
  | `plan_bypass_rebind` ignores `mode_changed` | `an_unchanged_mode_issues_no_bind` |
  | ... ignores `onscreen_context_is_current` | `a_mode_change_off_the_onscreen_context_defers_to_the_next_make_current` |
  | ... ignores `draws_to_default_fbo` | `a_mode_change_leaves_a_framebuffer_the_content_bound_alone` |
  | ... never re-binds | `entering_bypass_repoints_...at_the_window` **and** `leaving_bypass_repoints_...at_the_drawing_buffer` |
  | ... always re-binds the window | `leaving_bypass_repoints_...at_the_drawing_buffer` |

  The last two exist because the first four are all *negative* assertions, and a
  planner that never re-binds satisfies every one of them — the always-red gate this
  plan keeps catching. `never re-binds` proves the gate can fire; `always re-binds
  the window` proves something pins the target and not merely the decision. Killing
  two is correct for the first of those: their job is the positive control, and they
  differ in the value carried.

  **And a seventh mutant that no host test can reach.** Deleting the rebind from
  `evaluate_bypass` while keeping everything else: 240 frames painted — *the same
  count* — and `rgba(0,0,0,0)` captured. That is the site attribution the unit tests
  cannot give, since they pin the planner and not the call.

  **Design notes worth keeping.** `onscreen_context_is_current` is a precondition,
  not an optimisation: a bind lands in whichever context is current, so off the
  onscreen context it would corrupt an offscreen canvas instead. Skipping loses
  nothing because the `make_current_needed` that brings the context back resolves
  the binding from the same function — between them the two sites cover every path,
  which is why the planner returns `Nothing` rather than deferring work.
  `draws_to_default_fbo` is consulted because content mid-render-to-texture has its
  own FBO bound and the driver must keep it; re-pointing regardless would aim an RTT
  pass at the screen. The post-swap site's bespoke `if !bypass` is gone: it now asks
  the resolver, and "under bypass there was no blit, so nothing was clobbered" falls
  out of the answer instead of being asserted separately.

  **Not covered, named rather than implied.** The *cost* half of Section 7.3's "no
  redundant presentation copy" is still unmet for ordinary content — this repairs
  the fast path, it does not widen the condition that keeps ordinary content off it.
  ~~and that widening still needs the FBO-0 invariant enumerated over every toucher
  plus a device run per platform.~~ **The first half of that is now known to have
  been satisfied all along**: the enumeration, split out as task 0.65, found the
  recorded reason for `canvas_count == 1` to be false, because every canvas owns an
  EGL context that is only ever made current with its own surface. What the widening
  still needs is the device run. Of the four re-pointing sites only the mode change
  is covered end to end; a single-canvas fixture never switches contexts, so
  `make_current_needed` and the surface-recreate reuse are covered only at the shared
  resolver. ANGLE and `OHNativeWindow` are untouched for the reason 0.32 gives.

- [x] 0.59 Make `GLCmd::touches_canvas()` exhaustive by construction. Found by
  0.57, which needed it as a hoist target and could not use it.

  The function's own doc claimed exhaustiveness over every variant carrying a
  `canvas_id` and instructed future variants to be added; a `_ => None` catch-all
  meant the compiler never checked, and 29 of 118 had drifted off — `TexImage2D`,
  `TexSubImage2D`, `TexStorage2D`, the compressed uploads, all four
  `TexImage2DFrom*`, `FramebufferTexture2D`, `DebugLoseContext`, and the whole
  `Uniform*` family. Two live consumers read it, and neither degrades safely:

  - **Scoped stale marking** (`execute_gl_batch`) flips `skia_state_stale` only for
    canvases the batch touched. A canvas whose only commands in a batch are
    unlisted ones is not marked, and the next Canvas2D draw on it trusts Skia state
    a WebGL batch has since disturbed.
  - **Phase-reorder admission** (`can_reorder_phases`) defers the WebGL half of a
    packet past the Canvas2D half when the two share no canvas. A missed id makes
    two halves look disjoint when they are not, and the reorder inverts issue order
    on one canvas.

  **Two design choices, and the second is the one that matters.** Dropping the
  catch-all makes *exhaustiveness* the compiler's job. It does not make
  *classification* the compiler's job — a variant carrying a `canvas_id` can still
  be written into the `None` bucket behind a `{ .. }`. So the `None` arms name every
  field explicitly and use no `..`: giving any of them a `canvas_id` then fails to
  compile too. Together those two make the syntactic rule hold by construction
  rather than by the comment that had been asserting it.

  **The rule stays purely syntactic — carries `canvas_id` ⇒ `Some`.** That moved
  `DebugLoseContext` onto the list even though its handler arm ignores the field:
  any semantic exception is where the next drift starts, and both consumers are
  safe in the conservative direction (an extra stale mark, a reorder refused).

  **Mutation evidence. Two of the three mutants must fail to *compile*, because a
  claim about the compiler is only demonstrated by making the compiler refuse.**
  File byte-identical (sha256) after each restore.

  | Mutant | Result |
  | --- | --- |
  | Add a new `canvas_id`-carrying variant | `E0004 non-exhaustive patterns: MutantProbe { .. } not covered` |
  | Give `LinkProgram` a `canvas_id` | `E0027 pattern does not mention field canvas_id` |
  | Misclassify `Uniform3f` into the `None` bucket | `touches_canvas_covers_the_families_that_drifted_off_the_list` alone, at its own assertion |

  Removal is no longer an available mutant — with no catch-all, deleting a variant
  from the list does not compile — so the third is the realistic drift that remains,
  and it is what the new test exists for. The test pins one representative of each
  family that had drifted (a uniform write, an immutable texture allocation, a
  framebuffer attachment, and an upload sourced from another canvas) rather than
  enumerating 118 variants, because enumeration is now the compiler's job.

  Verified at 414 `migo-shared` and 561 `migo-graphics` lib tests, no failures; the
  four phase-reorder tests still pass, which is where a newly-refused reorder would
  have shown.

  **Not covered, named rather than implied.** `TexImage2DFromCanvas2D` and
  `TexSubImage2DFrom*` carry a *second* canvas — the 2D source they read. This
  returns the destination, which is right for both consumers as they are written,
  but reorder admission arguably wants the source too: deferring a WebGL half past a
  Canvas2D batch that draws into the very canvas the upload reads would sample the
  wrong frame. Nothing here establishes whether that packet shape can occur.

- [x] 0.60 Stop the engine's own framebuffer re-points from lying to the dedup
  shadow. Found by 0.57's enumeration; kept separate because it is not a bypass
  property, and it turned out to be a live defect rather than a coverage gap.

  `gl_state`'s framebuffer shadow is keyed on the **user-facing** framebuffer name,
  where `None` means "the default framebuffer". Two engine-internal paths re-pointed
  the driver at the DrawingBuffer without telling that shadow — the EGL switch in
  `make_current_needed` and the post-swap restore after the blit. Content holding its
  own FBO therefore had its next `bindFramebuffer(sameName)` deduped against a claim
  the engine had already invalidated, the call never reached the driver, and the
  render-to-texture pass drew wherever the engine last pointed: **the screen**.

  **Measured before it was fixed.** `scripts/fixtures/rtt-probe` binds a complete
  render target, lets a canvas switch happen, re-binds the same target and clears
  red. It presented `rgba(217,26,38,255)` full-screen on every one of 180 frames with
  `fbo_status = GL_FRAMEBUFFER_COMPLETE`. Any multi-canvas WebGL game doing
  render-to-texture was drawing its offscreen passes onto the window.

  **The `make_current_needed` fix is a deletion, and the argument for it is not
  performance.** The framebuffer binding is per-GL-context state and each canvas owns
  its context, so EGL hands a canvas back exactly the binding it had — which is also
  what the shadow already claims. Re-pointing it gave one function *two* behaviours:
  the `bound == Canvas(id)` short-circuit left the content's binding alone while a
  real switch clobbered it, and the shadow described only the first. Removing the
  re-point makes the two paths agree, removes a driver call per canvas switch, and
  needs no shadow write because nothing changed. A fresh context needs no help
  either: `DrawingBuffer::new` leaves its FBO bound and `evaluate_bypass` re-points
  it when bypass latches.

  **The sites that genuinely destroyed the binding get a paired operation.** The blit
  binds `READ=DrawingBuffer, DRAW=0`, so the post-swap restore must re-point — and
  `record_default_framebuffer_bind` is half of that operation, not bookkeeping after
  it. It records `None` on all three framebuffer targets and sets
  `draws_to_default_fbo`. Recording `None` rather than `clear()`ing the map is the
  difference between free and one redundant driver call per frame: `None` *is* the
  name of the default framebuffer, so the Cocos-style `bindFramebuffer(FRAMEBUFFER,
  0)` every frame — the exact redundancy this dedup exists for — stays deduped.

  **Mutation evidence, host half.** Files byte-identical (sha256) after each restore.

  | Mutant | Kills |
  | --- | --- |
  | The record does nothing (the defect as it shipped) | all three property tests, each at its own assertion |
  | The record `clear()`s the map instead of naming the default | `a_content_bind_of_the_default_is_still_deduped_after_the_engine_repoints` |
  | The record covers only `FRAMEBUFFER` | `the_engine_repoint_covers_the_separate_draw_and_read_targets_too` |
  | The record forgets `draws_to_default_fbo` | `the_engine_repoint_makes_the_canvas_draw_to_the_default_framebuffer_again` |

  **Mutation evidence, site half — and this is where the work was.** No host test can
  reach either site, so each needed a running engine, and each needed its *own*
  fixture:

  | Site mutant | Killed by | Presents |
  | --- | --- | --- |
  | Restore the `make_current_needed` re-point | `rtt-probe` alone | `rgba(217,26,38,255)` — the render target on the screen |
  | Delete the post-swap shadow record | `rtt-boundary-probe` alone | `rgba(26,76,230,255)` — a frozen first frame |

  `rtt-boundary-probe` exists **because the post-swap mutant walked past
  `rtt-probe`**, and the reason generalises: `rtt-probe`'s first framebuffer call each
  frame binds `null`, which differs from any stale shadow and is issued however wrong
  that shadow is. Reaching the post-swap site requires the frame's *first* call to be
  the content's own FBO, which means drawing the baseline with no bind at all —
  legitimate, because a frame beginning with the default framebuffer bound is exactly
  what the post-swap restore guarantees.

  **The gate had to be strengthened twice, and both holes were the same defect
  wearing different clothes.** First, `dominant_pixel.py` reported only the most
  common sampled colour, so a frame presented through a partial damage region could
  carry wrong pixels in part of the surface while the stale majority still read as
  expected; it now reports the distinct sampled colour count and the gate requires
  exactly one, which is the honest claim for fixtures that clear flat. Second — and
  this one is worth remembering — the post-swap mutant's real consequence is not red
  pixels but **no presentation at all**: with `draws_to_default_fbo` left false every
  clear looks invisible to damage tracking, the engine presented exactly one frame per
  run (confirmed by instrumenting the post-swap site: one event in three seconds), and
  the frozen frame's colour *was* the pass condition. Every probe now paints its first
  frame a different colour, so a stale capture reads blue. **"The screen is the
  expected colour" is an absence claim and needs its liveness paired in the same
  instrument — the frame count is not enough, because the JS loop kept running at
  60 fps throughout.**

  Verified at 569 `migo-graphics` lib tests and all four probes green.

  **Not covered, named rather than implied.** `frame_capture::capture_default_fbo`
  binds `READ_FRAMEBUFFER = None` and does not restore it; that is now subsumed,
  because the post-swap record names all three targets and runs after the capture —
  but nothing *pins* it, since a probe would have to request a capture mid-run and the
  player only captures once at the end. The equivalent shadow question for buffers,
  textures and programs is untouched: only the framebuffer binding was audited, and
  `make_current_needed` was the only engine-internal re-point of it. ANGLE and
  `OHNativeWindow` are untouched for the reason 0.32 gives.

- [x] 0.61 Replace the presentation path's source-text inspection tests. Named as a
  known hole by 0.56, which then said replacing them "needs a GL context and is not
  attempted here". That was wrong, and correcting it in place is half the point of
  this task.

  `swap_failure_preserves_accumulated_damage_for_retry` and
  `blit_failure_poison_is_propagated_to_present_history` read the *source text* of
  `swap_buffers_no_restore` and asserted that the byte offsets of `.swap_buffers(`,
  `self.damage.reset()` and `blit_succeeded` came in the right order. Two real
  properties, both orderings, neither able to fail on a behavioural change that
  leaves the text intact — on the path where being wrong means a frame of stale
  pixels.

  **What made them look untestable was a conflation.** The blit needs a live GL
  context; the *bookkeeping either side of it* does not. What actually blocked a real
  test was that swap failure was expressed as an early `?` return, so the failure
  branch could not be entered without a window surface. Passing the swap outcome in
  as data — the move `run_frame_phases` already used for the frame phases — makes all
  three branches reachable, and `commit_present_outcome` now owns them over a real
  `FrameDamageAccumulator` and `PresentDamageHistory`, both of which construct on the
  host.

  **Two design choices came out of writing the tests rather than out of planning
  them.** The commit takes the whole `PresentDamagePlan`, not just `current`, so
  "history records the frame's own damage and never the age-expanded repair" is a
  decision *inside* the tested function instead of a choice at the call site that
  only a text search could see — which retired a third inspection guard,
  `manager_blit_consumes_repair_and_history_records_current`'s history half. And the
  fixture plan gives `current` and `repair` deliberately *different* values, because
  they were interchangeable in the old guard and a test cannot tell apart two regions
  that are equal.

  **Mutation evidence. Six mutants, each a realistic tidying-up defect, each killing
  exactly one test at its own assertion; file byte-identical (sha256) after every
  restore.**

  | Mutant | Kills |
  | --- | --- |
  | Reset the accumulator even when the swap failed | `a_failed_swap_keeps_the_frames_damage_for_the_retry`, at its damage assertion |
  | Record a frame the swap never presented | *the same test*, at its history assertion |
  | Never reset the accumulator | `a_successful_swap_resets_the_frames_damage` |
  | Record `repair` instead of `current` | `history_records_the_frames_own_damage_and_never_the_age_expanded_repair` |
  | Leave a partial frame in history as a repair source | `a_failed_blit_makes_the_history_unusable_as_a_repair_source` |
  | Forget the whole-surface debt after a partial blit | `a_failed_blit_leaves_the_next_present_owing_the_whole_surface` |

  The first two matter as a pair: they kill one test at two different assertion
  lines, which is what shows both of its claims are load-bearing rather than one
  riding along. The last two are why the failed-blit case is **two** tests: poisoning
  history fixes what a *later* present repairs from, the accumulator debt fixes what
  the *next* one does, a commit could do either alone, and one bundled assertion
  would have left whichever half broke unnamed — the shape this plan has now caught
  three times.

  `a_successful_swap_resets_the_frames_damage` is the control: without it, "damage
  survives a failed swap" is satisfied by an accumulator that never resets, which
  would make every frame repair the whole surface and the buffer-age machinery
  decoration.

  Verified at 564 `migo-graphics` lib tests, no failures.

  **Not covered, named rather than implied.** One text guard is kept and narrowed:
  `blit_to_surface` must return `bool`, because a blit that cannot report failure
  makes the poison branch unreachable and no host test can see that. Whether that
  return value is *truthful* still needs a GL context. Two writing rules found along
  the way and worth reusing: a reset `FrameDamageAccumulator` resolves to
  `FullSurface`, not to an empty rect list, so `resolve_rects` is the wrong
  observable for "owes nothing" and `has_damage()` is the right one; and
  `PresentDamageHistory` exposes no accessor, so what an entry *is* must be read
  through `resolve_with_age(current, 2)` rather than by counting with `len()`, which
  cannot tell a poisoned entry from a recorded one.

- [x] 0.63 Audit the rest of the dedup shadow against the engine's own GL writes.
  The open question 0.60 left: it fixed the framebuffer binding, and buffers,
  textures, programs and vertex arrays are shadowed the same way.

  **The enumeration.** Seven sites in the manager declare `TEXTURE_BINDING` stale to
  *Skia's* tracker (`mark_all_2d_contexts_stale_bits`), which is the honest signal
  that they disturbed the texture binding — and none told the WebGL dedup shadow, the
  other consumer of the same driver state. The discriminator is not whether they
  declare but whether they **restore**: five save `prev_tex` / `prev_active_texture`
  and put it back, so the shadow stays true and the Skia declaration is needed only
  because Skia's tracker cannot see the restore. Two bind *zero* instead —
  `pbo_upload.rs` (behind `load_shared_image`) and `texture_import.rs` (behind
  `load_ahb_image`) — and those two are the defect. `compressed_upload.rs` already
  restores, under the comment "Restore the app-visible bindings on both success and
  failure paths", which makes the other two an inconsistency rather than a decision.

  **So the fix is a restore, not an invalidation.** An `invalidate_after_texture_upload`
  helper was written first and then deleted: telling the shadow to forget is correct
  but costs the next draw a rebind for state that never needed to move, and it leaves
  three upload paths following two patterns. Saving `TEXTURE_BINDING_2D` and
  `UNPACK_ALIGNMENT` and putting them back keeps the shadow true by construction,
  costs two `glGet` per *image* rather than per frame, and makes all three identical.

  **Two hypotheses were wrong, and recording them is the point.** First: this was
  predicted to be a live defect on every image load. It is not — ordinary loads go to
  the upload thread, which owns a GL context of its own and so cannot disturb a
  canvas's bindings. Measured: `scripts/fixtures/upload-shadow-probe` binds a texture,
  loads images continuously, re-binds, and asks `getParameter(TEXTURE_BINDING_2D)` —
  which the handler answers from a real driver query, not from the shadow — and
  reports the binding intact for a whole run. Second: the sync render-thread path was
  assumed reachable from JS by making an image too large for the async byte budget. It
  is not; a 1200×1200 image (5.49 MB decoded, 8 KB on disk) still went async, and
  instrumenting `load_shared_image` counted **zero** calls in a run. What actually
  reaches a canvas context is the AHB path (unconditional, and the Android default),
  compressed textures, `ImagePriority::Critical` — which nothing produces today, every
  JS load asks for `Normal` — and the async-degradation fallbacks.

  So the defect is **latent on the platform that ships** and unreachable here. The
  probe is kept, inverted, as a boundary control on the architectural property that
  makes it latent: uploads must not run on a canvas context. An "optimisation" that
  moved them onto the render thread to save a context switch would silently
  reintroduce exactly what 0.60 fixed on the framebuffer binding, and this probe is
  what would say so.

  **A fixture defect the gate caught, worth keeping in mind.** The probe first
  reported `path=bypass` when it wanted the blit path: its second canvas was held in
  an unused `const` and was collected — the engine logged `canvas_count` going
  1 → 2 → 1 as the finaliser ran, driven by the per-frame image allocations. A canvas
  has to be *used* to stay alive. This surfaced only because the gate reads which path
  ran from the engine's own transition log instead of trusting the fixture's intent.

  **Not covered, named rather than implied.** The two restores are unpinned: no host
  test can see a driver binding, and neither reachable caller can be driven on Linux.
  They are argued from the code and from the third path that already does it, not
  measured. Programs, vertex arrays and buffers were checked only through the seven
  `mark_all_2d_contexts_stale_bits` sites; a path that disturbs one of those *without*
  declaring anything to Skia would not appear in that enumeration, and nothing here
  rules one out.

- [x] 0.64 Delete a GL object from a context in which its name means that object.
  Found by the FBO-0 enumeration ledger 0.57 deferred to 0.65, which is the third
  time that enumeration has produced a defect adjacent to the one it went looking
  for.

  ES 3.0 Appendix C.1 shares buffer, program, shader, renderbuffer, sampler, sync and
  texture objects across an EGL share group. It does **not** share the container
  objects — framebuffers, vertex arrays, queries and transform feedbacks — so the
  same small integer names a different object in every context of the group. Every
  canvas here owns a context, so "which context is current" decides which object a
  `glDelete*` frees.

  **Eleven delete sites decided that four different ways.** Six bound *any* canvas
  through `ensure_any_canvas_current` (`DeleteProgram`, `DeleteShader`,
  `DeleteTexture`, `DeleteRenderbuffer`, `DeleteBuffer`, and — wrongly —
  `DeleteFramebuffer`); three bound nothing and used whatever the previous command
  left current (`DeleteSampler`, `DeleteSync`, and — wrongly — `DeleteVertexArray`);
  two consulted the owner behind a fallback that deleted from the current context
  when it was absent (`DeleteQuery`, `DeleteTransformFeedback`).

  **The knowledge was already written down next to the code that ignored it**, which
  is the part worth remembering. `VaoMeta`'s doc comment says "VAOs are not shared in
  the EGL share-group model WebGL uses"; `DeleteVertexArray` deleted from whatever was
  current. `FramebufferMeta.owner_canvas` and `VaoMeta.owner_canvas` were both
  recorded and both carried `#[allow(dead_code)]` — the attribute *is* the defect,
  written down. And `SyncMeta`'s comment had the opposite error, claiming sync objects
  are not shared when Appendix C.1 says they are, so its rebind was unnecessary
  rather than missing.

  **How it was found, and it was not by reading the delete sites.** Cross-referencing
  `GLCmd::touches_canvas()`'s 118 `Some(canvas)` variants against which
  `handle_command` arms call `make_current_needed` produced two lists. Seven `Some`
  variants have no call in the arm — `DebugLoseContext`, which touches no GL, and the
  six `TexImage2DFrom*`/`TexSubImage2DFrom*`, which make current one level down in
  their `CanvasManager` helper. Five arms *do* call it while `touches_canvas()`
  answers `None`: `ClientWaitSync`, `DeleteQuery`, `GetQueryParameter`,
  `DeleteTransformFeedback`, `GetTransformFeedbackVarying`. Those five resolve a
  canvas from an object's owner rather than from their own fields, and asking *why
  those five and not the other delete arms* is what surfaced the sharing rule.

  **What the five mean for `touches_canvas`, reported because it survived.** Its two
  consumers — scoped Skia stale marking and phase-reorder admission — want "which
  canvas will this command bind", and the function answers "which canvas do this
  command's fields name". For those five the answers differ. Neither consumer is
  harmed today, because all five issue only `glDeleteQueries`,
  `glDeleteTransformFeedbacks`, `glGetQueryObjectuiv`,
  `glGetTransformFeedbackVarying` and `glClientWaitSync`, none of which touch any GL
  state Skia caches. So this is a latent contract mismatch and not a live defect, and
  it is recorded rather than fixed: the next command added in this shape inherits it.
  Task 0.59 made the classification exhaustive over *fields*, which is a different
  claim from exhaustive over *canvases touched*.

  **What the defect costs, and which half a driver decides.** The leak is certain
  everywhere: the name does not exist in the context the call went to, `glDelete*`
  silently ignores unknown names, and the metadata has already been discarded, so
  nothing can ever free the object. The other half is the driver's numbering, and an
  instrumented build settled it on this host rather than leaving it to argument — the
  onscreen DrawingBuffer is framebuffer 1, the onscreen canvas's own render target is
  2, an offscreen pool comes back 3..10, and every one of the eight offscreen deletes
  was dispatched with `bound=Some(1)`, the onscreen canvas. Mesa numbers container
  objects from one share-group counter, so no collision is reachable here. A driver
  that numbers per context — mobile GPUs — gives an offscreen canvas's first
  framebuffer the name 1, and that delete then destroys the DrawingBuffer, after
  which the onscreen canvas renders into a deleted object and presents nothing. That
  is the same outcome 0.57 spent a whole task on, reachable from a game freeing a
  render-target pool.

  **The fix makes the decision unspellable rather than repeated.** `GlObject` carries
  a kind and a name that cannot disagree, and a container variant cannot be
  constructed without its owner. `CanvasManager::delete_gl_object` is the only place
  that issues a `glDelete*` and it makes the owning context current first — one
  operation, not a bind followed by a call that could be written without it. The four
  container metadata types lost their `Option` around the owner, because
  `Option<CanvasId>` where only `Some(canvas_id)` is reachable is the
  representable-but-unreachable state that let two sites "handle" `None` by ignoring
  the owner entirely; a `take_for_delete` hands the handle out only inside a
  `GlObject`, so a caller cannot get a bare name. A missing owner is no longer a
  reason to delete from somewhere else: a container object cannot outlive its
  context, so if the canvas is gone the object already is.

  **Mutation evidence. Four mutants, each attributed, every file byte-identical by
  sha256 after restore.**

  | Mutant | Kills | Survivors |
  | --- | --- | --- |
  | `Framebuffer` classified shared | `a_container_object_must_be_deleted_from_the_context_that_minted_it` at `gl_object.rs:144` | 570 |
  | `VertexArray` classified shared | the same test at `gl_object.rs:153` | 570 |
  | `Texture` classified per-context | `a_shared_object_may_be_deleted_from_any_context_in_the_group` | 570 |
  | A new `GlObject` variant | `E0004 non-exhaustive patterns`, in **both** matches | — |

  The two container mutants dying at *different* assertion lines is the point: both
  claims in that test are load-bearing and neither rides along. The third is the
  positive control — every assertion in the first test is satisfied by a
  classification that answers `Some(owner)` for everything, which would also refuse
  to delete a shared object whose nominal owner canvas has gone. The fourth is the
  only kind of evidence available for a by-construction claim.

  **Not covered, named rather than implied.** The wrong-context deletion is
  unobservable on this host — neither the cross-canvas destruction nor the leak moves
  a pixel or a counter under Mesa's numbering — so no probe here gates the defect
  itself. `scripts/fixtures/fbo-owner-probe` gates the eleven rewritten call sites
  instead: an offscreen canvas frees a pool of framebuffers, in a frame where nothing
  has named it so the onscreen context is current, while the onscreen canvas keeps a
  render target of its own and clears the window *before* its render-to-texture pass
  so a destroyed target would send red to the screen rather than be painted over. It
  is honest about what it is: an end-to-end regression guard on a rewrite of eleven
  arms, not the gate for the defect. Nothing prevents a future site calling
  `gl.delete_*` directly, because the container metadata's handles stay readable for
  binding. Verified by `scripts/verify-change.sh --base HEAD`: every host target plus
  the arm64-v8a Android compile, migo-graphics 571 tests, and all seven probes in
  `scripts/verify-bypass-present.sh` green.

- [ ] 0.65 Widen the DrawingBuffer bypass condition. Split out of 0.57, whose
  deferred half this is. **The invariant work is done and the recorded reason for the
  condition turned out to be false; the flip is deliberately not taken here.**

  0.57 recorded that widening "needs the FBO-0 invariant enumerated over every
  toucher plus a device run per platform". The enumeration says the first half is
  already satisfied and always was, for a reason the spec had backwards.

  **The recorded reason cannot arise.** It held that bypass makes the onscreen default
  framebuffer real FBO 0, that real FBO 0 is whichever EGL draw surface is current,
  and that offscreen pbuffers therefore stop "FBO 0" naming the window. FBO 0 follows
  the current surface *of the current context*, and every canvas owns a context:
  `create_onscreen` calls `eglCreateContext`, `register_offscreen` calls
  `create_pbuffer_context`, each sharing only the resource context's objects, and
  `make_current_needed` takes context and surface from one `EglContextHandle`. A
  pbuffer is only ever current with the canvas that owns it, in which the onscreen
  canvas cannot be drawn at all. The spec's supporting claim that the DrawingBuffer's
  FBO is "a name in the shared context" is wrong for the same reason 0.64 is about:
  framebuffers are container objects and share groups do not share them.

  **What the condition is really doing.** Both modes require the same thing — a
  command that draws to a canvas runs with that canvas current, established per
  command in `handle_command` — and `canvas_count == 1` makes that precondition
  vacuous by leaving no other context to be current. Bypass does not weaken the
  precondition; it changes what violating it looks like, from a framebuffer name that
  means nothing in the current context to a silently wrong surface.

  **Enumeration of what bypass changes**, which is the smallest complete answer:
  `get_drawing_buffer_fbo(onscreen)` returns `None` instead of the DrawingBuffer's
  FBO, and the swap-time blit is skipped. Four consumers read the first
  (`is_drawing_buffer_bound`, `bind_default_framebuffer`, `evaluate_bypass`'s rebind,
  `BindFramebuffer`'s default-framebuffer redirect) and two read the second
  (`swap_buffers_no_restore`, `prepare_present_plan`, which already forces a full
  repair region under bypass). None has a canvas-count term. The paths that could
  read the onscreen canvas's content as a texture do not exist: `DrawingBuffer`'s
  `color_tex` never leaves `drawing_buffer.rs`, `TexImage2DFrom{Canvas2D,Snapshot}`
  source a *2D* canvas — and an onscreen 2D context already disables bypass — and
  `ReadPixels` on the onscreen default framebuffer latches
  `needs_default_fbo_readback`, which also disables it. The cross-canvas upload path
  uses a per-canvas `image_copy_fbo` as `READ_FRAMEBUFFER` and restores it, never
  FBO 0.

  **Measured with two live canvases, which no probe had done.** `bypass-probe` and
  `blit-probe` differ by whether a second canvas *exists* and neither draws to one, so
  nothing here had ever switched EGL contexts inside a frame — the exact condition the
  recorded reason was about. `scripts/fixtures/bypass-multi-probe` draws to both twice
  per frame and ends the frame on the offscreen pbuffer, so presentation must bring the
  window back unaided; its offscreen clear is red, so an empty capture means the
  onscreen clear went somewhere that is not the window and a red one means an offscreen
  draw arrived there. On the blit path: 240 frames, `rgba(51,204,102,255)`, one
  distinct colour. With `canvas_count == 1` temporarily removed: the same, on the
  bypass path, and `bypass-probe` and `blit-probe` also both green.

  **Why the flip is still not taken.** Because bypass has never run in steady state
  on any device — this condition is *why* — so widening it enables, for every bundle
  on four platforms at once, a path whose only end-to-end evidence is a Linux
  software rasteriser. Android's Surface path compiles here and is unmeasured;
  `platform/src/{windows,ohos}/presenter.rs` have never compiled here at all (0.32).
  The remaining step is one line in `can_bypass_drawing_buffer` plus its test, and
  what it needs is `scripts/verify-bypass-present.sh` run against each platform's own
  host, `bypass-multi-probe` included. The condition's own test was renamed to
  `a_second_canvas_keeps_the_onscreen_canvas_on_the_drawing_buffer`, because the old
  name asserted the reason that turned out false.


  task 0.21's storage/quota group and 0.58's permission group both stop at: each is
  proven "given distinct inputs" and nothing showed two Sessions *producing* distinct
  inputs.

  **The recorded obstacle was false, which makes six in a row.** It was written down
  as "a Session with no surface never reaches `evaluate_module` or the permission
  gate". `HostJsRuntime::new(host_id, host_state, cache_dir, ..)`
  (`runtime-v8/src/host_runtime.rs:101`) takes **no surface**, and
  `HostJsRuntime::evaluate_module(game_id, entry)` (`:831`) is precisely where the
  identity binds: it builds `GamePaths::new(files_dir, cache_dir, game_id)` and
  installs it with `set_game_paths` (`:942`), which is the value
  `crate::storage::storage_dir` resolves against. A surface is required by
  `spawn_host_thread` → `Host::new` (`core/src/runtime/host.rs:312`), i.e. by the
  *orchestration* layer — one level above where the property lives. Asking "which
  layer can see this property?" arrives at `HostJsRuntime`; asking "how do I start a
  Session?" arrives at the surface and stops. Nor was a surface an obstacle even
  there: `tools/player` runs headless on an `OffscreenSurface` pbuffer with no window
  server.

  **What V8 actually constrains, which is not what the plan guessed.** Two
  `HostJsRuntime`s on one thread abort the process:
  `Fatal error in v8::HandleScope::CreateHandle(): Cannot create a handle without a
  HandleScope`. The current isolate is thread-local and each runtime expects to own
  it. So the fixture is a thread per Session, driven step by step over a channel with
  every wait bounded — which is *closer* to the thing under test, because a real
  concurrent Session is a host thread. The real constraint was the isolate, not the
  surface.

  **Two fixture decisions carry the test.** Both Sessions get the **same**
  `app_files_dir` and `app_cache_dir`; giving each its own would make the two paths
  differ for a reason unrelated to per-game namespacing, and the test would pass over
  an engine that ignored the game id entirely. And both loads complete before either
  path is read: every way an identity can be shared — a process-wide slot written at
  bind time, a resolver memoising its first answer, one op state behind two runtimes —
  produces two *equal* paths only once both Sessions have bound, so reading `a` before
  `b` loads would let all of them through.

  The assertion is not merely `assert_ne!`. Two paths can differ while both are wrong
  — a counter, a host id, a temp name — so each is also required to be under its own
  namespace *and under no other*. That negative half is what caught the first mutant.

  **Mutation evidence, and the point is what *survived*.** Files byte-identical
  (sha256) after each restore.

  | Mutant | Kills the new test at | Pre-existing suite |
  | --- | --- | --- |
  | The stored identity is the Session, not the game (`host-N` for the game id) | its namespace assertion | **all 519 survive** |
  | The bound identity escapes to a process-wide slot the resolver prefers | its `assert_ne!` | **all 519 survive** |

  Two different assertion lines, so both halves of the one test are load-bearing. That
  the whole pre-existing suite survives both is the proof the fixture closes a real gap
  rather than restating one:
  `the_resolver_gives_two_games_different_storage_files` builds two `OpState`s by hand
  and never calls `set_game_paths`, so it cannot see an identity that escapes at *bind*
  time — the escape is invisible to a test that never binds.

  **The first mutant had to be narrowed, and the reason generalises.** Changing the
  `game_id` fed to `GamePaths::new` moved the *code* directory too, so the entry module
  went missing and the test died inside a helper on a module-load error rather than at
  the assertion it is named for. The usable mutant changes only the identity that gets
  *stored*, leaving the local `code_dir` the loader uses intact. When one value feeds
  two things, mutate at the point only the property under test can see.

  **One production line was added:** `pub(crate) fn op_state()` on `HostJsRuntime`, so
  the question goes to the *production* resolver over the state a real
  `evaluate_module` left behind, rather than to a hand-built `OpState` that agrees with
  it by construction.

  **A test was written, passed, and was then deleted — worth recording.**
  `loading_a_second_game_does_not_move_the_first_ones_storage` was killed by the second
  mutant, but by the *same* mutant as the other test and by nothing alone. Every
  shared-identity defect is already caught by reading both paths after both loads, so
  it was redundancy, and redundancy hides which guard is load-bearing. Its content
  moved into the surviving test's doc comment, where the read ordering it was
  protecting is now stated as the load-bearing thing it is.

  **Not covered, named rather than implied.** This covers the storage half only. The
  permission half is a *different arbiter*: `platform/src/android_permission_gate.rs:33`
  keys `live: HashMap<i32, HostState>` by **host id**, not by game id, so the game
  identity never enters it — and 0.58 pinned the *Java* `PermissionOperationGate`,
  which is keyed by session id. Two separate objects, and the spec should stop implying
  one property spans both. Quota is untouched: distinct storage roots are necessary for
  a per-game quota but not sufficient, and nothing yet shows one game exhausting its
  10 MB leaving another's intact.

- [x] 0.58 Assert that a permission grant does not cross Sessions. The last of task
  0.21's uncovered property groups. `PermissionOperationGate` is a process-wide
  static keyed by session id, so two concurrent Sessions meet inside one object and a
  grant that leaked between them would let one game use a capability the user
  approved for another — the permission half of Section 6.4's isolation.

  **The gate had two-session tests and none of them looked this way.** Every
  cross-session test grants a scope and then asserts the *granted* session still
  works; that direction passes over a gate that grants everyone.
  `aGrantOnOneSessionLeavesTheSameScopeDeniedOnAnother` asserts the other direction —
  an ungranted session is refused both the callback admission (`runIfGranted`) and
  the cancellation registration (`register`) — with the positive case in the middle as
  the control, since a gate that granted nobody would satisfy both denials while
  breaking every permission in the product.

  **Mutation evidence, and the mutant had to be chosen rather than reached for.**
  Moving `Session.scopes` to the gate would have been the obvious "share the grants"
  mutant, and it is not usable: `close` iterates its own session's scopes, so a
  shared map makes closing one session cancel another's and the *existing*
  cross-session close test fails — the mutant would have killed two tests and pinned
  neither. The mutant used instead is the realistic defect: a fast path in
  `runIfGranted` that, finding no entry for this session, reuses another session's
  grant for the same scope — the shape an "optimisation" takes. It fails
  `aGrantOnOneSessionLeavesTheSameScopeDeniedOnAnother` **alone**, with 103 other
  Java tests passing, because no other test ever queries a scope on a session that
  was not granted it.

  Verified at **104 tests per flavour, Full and Slim, no failures, errors or
  skips**; the permission coverage contract still reports 30 gated, 8 cleanup, 38
  permission-sensitive operations.

  **Not covered, named rather than implied.** This is the gate's own arbitration.
  That two *real* Sessions reach it under distinct ids is a Rust-side property of id
  allocation, which `capi/src/concurrent_sessions.rs` covers as distinct handles but
  does not follow through to a registration at this gate — the same layer seam the
  storage group has, and for the same reason: a Session with no surface never gets
  that far.

- [x] 0.22 Retain the concrete handle when cleanup of a rejected late GATT
  candidate fails. **Implementation complete, reviews outstanding.**
  `publishGattConnection` closed a candidate whose attempt had been superseded, and
  when that `close()` threw the failure was reported and the `BluetoothGatt` dropped:
  the OS handle stayed open for process life and nothing ever tried again.

  Why the owned path did not have this bug, and why the fix could not simply copy it:
  `closeAndRemoveGatt` gets retain-on-failure for free, because `closeGatt` throwing
  means it never reaches its `gattConnections.remove`, so the attempt stays mapped and
  the next close retries it. A late candidate is **never** in that map — the map holds
  the attempt that won — so there was no entry to leave behind. The retention has to
  be explicit.

  `unclosedCandidates` is a concurrent set that entries leave **only on a successful
  close**, which is the same rule stated directly instead of emerging from control
  flow. `retryUnclosedCandidates` drains it, snapshotting first because a concurrent
  `publishGattConnection` may add while it runs and the next close will take those.
  It is called from `closeGattConnection` before that device's own close, so a retry
  failure is reported rather than masking the caller's close, and from `closeAdapter`,
  so a session that never closes a device individually still retries at teardown.

  Written test-first: both tests failed to compile against the missing
  `unclosedCandidateCountForTests`, then failed, then passed. Mutation-proved —
  removing the retention fails 2 tests, and retiring a candidate whose retry also
  failed fails 1. The second test needed a new `failCloseAlways` mode on the fake,
  since the existing `failCloseOnce` cannot express a handle that stays unclosable.

  Verified at 96 tests per flavour, Full and Slim, up from 94, with no failures,
  errors, or skips; the permission coverage contract still reports 30 gated, 8
  cleanup, 38 sensitive operations.
- [x] 0.23 Apply the retired-id tombstone to the Java permission gate.
  **Implementation landed, reviews outstanding** (`6825fad`). The gate refused any
  session id at or below the highest ever opened rather than the ids actually
  retired, and because `registerSession` throws on refusal, two Sessions created
  concurrently whose ids reached the gate out of allocation order left the loser's
  `GameSession` partly constructed. The tombstone is now an explicit retired-id
  set, retired atomically with the removal under the admission guard so no `open`
  can observe an id as neither live nor retired, and only on a successful close so
  a retained session is never retired. A plain guarded set rather than a concurrent
  one, precisely so retirement and removal are one step. Verified at 94 tests per
  profile with no failures, errors, or skips; permission coverage contract passes;
  `git diff --check` clean. Mutation-tested: reintroducing the high-water mark
  re-fails both ordering tests.
  **Checked, and the answer is that no Java path re-registers a live session id.**
  `registerSession` has exactly one caller — the `GameSession` constructor — and
  `restart()` calls `NativeMethods.onRestart(sessionId)` without rebuilding the wrapper,
  so the tolerance the Rust gate needs (because `HostCommand::Restart` *does* rebuild
  `AndroidDeviceServices` for the same live id) has no counterpart here. Treating every
  refusal as fatal is therefore correct, and no distinction of that kind was drawn.

  **A different distinction was missing, and it was the diagnostic.** `open` returned a
  `boolean` for two refusals that mean different things to a host — an id still live is
  two sessions sharing an id, an id retired is one whose permissions can never be
  granted again — and the single message `registerSession` threw named the *closing* case
  for both, telling a host the opposite of what happened in the other half. `open` is now
  `admit`, returning `ADMITTED | ALREADY_LIVE | RETIRED` answered in the **same**
  acquisition of the admission guard, so no caller can observe a state between the two and
  none can recompute the distinction differently. A `boolean` plus an `isRetired`
  accessor was written first and rejected for exactly that: it is two acquisitions and two
  chances to disagree about one id.

  `admissionSaysWhetherARefusedIdIsLiveOrRetired` pins it, with the admitted case
  asserted in the same test because a gate that refused everything satisfies both refusal
  assertions. Mutation: collapsing a live id's answer into `RETIRED` — the shape that
  produced the wrong message — fails that test and nothing else. Converting the boolean
  call sites also made every existing refusal assertion name a cause instead of a
  polarity. 107 tests per flavour, Full and Slim, from 106, no failures, errors or skips;
  permission coverage contract unchanged at 30 gated, 8 cleanup, 38 sensitive.

  **Not covered, named rather than implied.** `NativeExports.registerSession` itself has
  no test. Reaching it needs a `GameSession`, whose constructor starts a
  Choreographer-driven scheduler and touches framework state, and this module's test
  classpath has no mocking framework; a production accessor added to reach it was written
  and deleted rather than shipped. So what is gated is the gate's answer, and what renders
  it into a message is read rather than executed.
- [ ] 0.24 Order connection-state reports per device. `reportRetiredAttemptDisconnected`
  reads the current owner and then reports outside any lock, so a retired attempt
  that observes no owner can still deliver a stale `connected=false` after a fresh
  attempt has published and reported `connected=true`. The current code is a net
  improvement over the previous unconditional report, which overwrote the
  replacement's state every time, but the residual race remains. A correct fix
  serialises ownership transfer and state reporting per device — for example by
  replacing the attempt map with a per-device state record whose monitor orders
  both — which is a substantive change to the GATT ownership model and needs its
  own plan and review rather than a patch.

  **Re-read on 2026-08-08 and the description holds**, which is worth saying because
  most recorded obstacles on this ledger have not. `gattConnections` is a
  `ConcurrentHashMap<String, GattAttempt>`; `publishGattConnection` only checks
  `get(deviceId) == attempt` before attaching, and `reportRetiredAttemptDisconnected`
  reads the same map and then calls `connectionStateReporter.report` outside it. The
  window is after `closeAndRemoveGatt` removed the entry and before a replacement is put
  in: the retired attempt's `false` is *correct at the moment it is decided* and the
  decision is not atomic with its delivery, so a replacement that publishes and reports
  `true` in between is overwritten.

  **One constraint the recorded "for example" runs into, found while confirming the
  race.** Ordering both under a per-device monitor means holding that monitor across
  `report`, which crosses into native — and this codebase's own rule elsewhere is that a
  Migo lock is not held across a JNI call: the permission gate runs its external
  operation under a counted lease precisely so revocation can wait without retaining the
  host mutex. Reconciling those two is the substance of the plan this item asks for
  rather than an afterthought. The alternatives are a monitor held across a report that
  is a post rather than a wait, or a per-device delivery sequence that drops a report
  older than the last delivered one — and the second only orders if the compare and the
  call are themselves one step, which puts the monitor back. Recorded so the plan starts
  from the real constraint instead of rediscovering it.

  **Fixed on 2026-08-08. Implementation, tests and mutation evidence are below; neither
  independent review has run, so the item stays open.** The "substantive change to the
  GATT ownership model" this asked for turned out not to be needed, and saying why is
  the point of this entry.

  **The narrowest correct structure is not the one recorded here.** This item specified
  a per-device state record whose monitor orders ownership transfer and reporting.
  Per-device is indeed the narrowest ordering the property requires — and it is the more
  expensive answer, because the monitor has to outlive the attempts it orders (ordering
  an outgoing attempt against its replacement is the entire point), so it becomes a map
  of monitors with a lifetime rule, an eviction rule, and a bound, since content chooses
  the device ids. **One monitor per session** is strictly stronger, has no lifecycle at
  all, and costs two devices' connect events the duration of one queue push, on a path
  that fires when a peripheral connects or drops. Narrowest and cheapest were different
  answers here.

  **The recorded constraint — a Migo lock must not be held across a JNI call — is real
  and does not apply to the whole transition, only to part of it.** What the monitor
  covers is the ownership re-check and the report. What stays outside it is every
  framework call: `close()`, `disconnect()` and `discoverServices()`, all of which can
  block. That split is sufficient rather than merely convenient: a publisher's map write
  precedes its own report in program order, and the monitor orders the reports, so a
  thread that acquires the monitor after that write observes it. Holding it across the
  report itself is safe for the reason the constraint is really about — the report is a
  **post, not a wait**: it enqueues on a bounded channel and returns, never re-enters
  Java, and never waits on a Migo lock. That is the same distinction the permission
  gate's counted lease is built on.

  **A second staleness the description did not name, found while fixing the first.**
  `discoverGattServicesAndReport` reported its result unconditionally, so a superseded
  attempt whose service discovery finished after its teardown reported the device
  **connected** — resurrecting a device whose close had already completed. The two
  directions are therefore not symmetric, and that asymmetry is now the semantics: a
  *disconnect* from an attempt the map no longer holds is precisely the report that must
  arrive, because retirement is what removed the entry; a *connect* from one must not,
  because no owner means nothing is entitled to claim the device is connected.

  **Mutation evidence, and the third mutant is the one worth reading.**

  | Mutant | Kills |
  | --- | --- |
  | Decide, leave the region, then deliver — the shape this item describes | two tests, the first at `the replacement never blocked … it is in TERMINATED` |
  | The monitor kept, `connected` reported unconditionally as before | the resurrection test, `expected:<[]> but was:<[true]>` |
  | The monitor kept, the re-check moved back outside it | **nothing, at first** |

  **The third mutant passed every test, and it is a genuine defect.** Moving only the
  *report* inside the monitor still orders the two reports against each other, so the
  interleaving test was satisfied — while the stale decision it exists to catch survived
  untouched. The test could not see it because the fixed implementation has no window
  between reading the map and delivering, so nothing could be parked there. What
  discriminates is holding the monitor **from the test**, which stops the retired
  attempt between its two steps in the mutant and before both of them in the fix: it
  wakes to a world where a replacement exists and must notice. That needed
  `connectionStateOrderForTests()`, exposed for the reason the Rust contention probe is
  handed a registry's lock — a property about two steps being one cannot be demonstrated
  without stopping a thread between them. **This is the same lesson as the JVM probe's
  missing self-check control two items ago, in the same session: the first version of a
  guard covers the side it was designed for.**

  **Verified.** The Android Java suite at 119 tests, no failures or errors, across both
  product variants; four new tests, three mutants each killing the gate named and no
  others. No device evidence: this is a race between a peripheral dropping and
  reconnecting, and nothing here has run against one.
- [x] 0.25 Snapshot the pending cancellation action safely. Landed with `6825fad`:
  `runCancellations` captures the action into a local while the snapshot is taken
  under the session monitor, so the executed action is exactly the one the snapshot
  admitted and correctness no longer rests on the unstated argument that the only
  mutator is reachable solely from a monitor-holding path. Mutation-tested:
  restoring the late field read re-fails the replacement test.

