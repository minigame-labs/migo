## Four-Platform Delivery Ledger

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take Migo to a locally verified `0.10.0-rc.1` release candidate that is delivery-ready on Android, Linux, Windows, and HarmonyOS, without pushing, tagging, or publishing.

**Architecture:** Correctness first, then hermetic four-platform builds, then native qualification, then the integration contract that makes host integration comparable across platforms, then consumers, then performance evidence and public material. Each phase is fail-closed: a later phase never compensates for an unverified earlier invariant.

**Tech Stack:** Rust 2024, V8/Deno Core, Skia/EGL/ANGLE, JNI/Java/Gradle, Android NDK, C/C++ ABI, X11/Wayland, Win32/MSVC, HarmonyOS NAPI/ArkTS/hvigor, CMake/Ninja, Qt 6, Bash/PowerShell, GitHub Actions, SBOM and provenance tooling.

---

## Authority

The design authority is
`docs/superpowers/specs/2026-08-03-four-platform-delivery-design.md`, which
supersedes Sections 3 and 15 of
`docs/superpowers/specs/2026-07-29-three-platform-delivery-design.md` and extends
everything else in it. Inherited Phase A items keep their original identifiers
(A1 through A13) so their detailed plans remain valid.

## Status Convention

- `- [ ]` open
- `- [x]` complete, with fresh verification and both independent reviews passed
- `- [!]` **blocked** — the blocking reason and the exact unblocking command or
  purchase is recorded inline. A blocked item is never silently reclassified as
  complete or skipped.

Completion requires all of: implementation, behavioural tests, fresh
verification output, independent spec review, and independent code-quality
review. A commit alone is not completion evidence.

---

## Contents

The ledger body is split into per-phase files in the
[`2026-08-03-four-platform-delivery/`](2026-08-03-four-platform-delivery/)
directory. Each file is self-contained and can be read, searched, or edited
independently.

| Part | Items | File |
|------|-------|------|
| Tooling And Verification | T.1 – T.5 | [part-tooling.md](2026-08-03-four-platform-delivery/part-tooling.md) |
| Phase 0 — Correctness Foundation | 0.1 – 0.65 (66 items) | [part-phase-0.md](2026-08-03-four-platform-delivery/part-phase-0.md) |
| Phase 1 — Hermetic Four-Platform Builds | 1.1 – 1.14 (27 items) | [part-phase-1.md](2026-08-03-four-platform-delivery/part-phase-1.md) |
| Phase 2 — Native Platform Qualification | 2.1 – 2.8 | [part-phases-2-5-blocked.md](2026-08-03-four-platform-delivery/part-phases-2-5-blocked.md) |
| Phase 3 — Integration Contract | 3.1 – 3.7 | [part-phases-2-5-blocked.md](2026-08-03-four-platform-delivery/part-phases-2-5-blocked.md) |
| Phase 4 — Consumers And Helper Repositories | 4.1 – 4.10 | [part-phases-2-5-blocked.md](2026-08-03-four-platform-delivery/part-phases-2-5-blocked.md) |
| Phase 5 — Performance And Release Candidate | 5.1 – 5.7 | [part-phases-2-5-blocked.md](2026-08-03-four-platform-delivery/part-phases-2-5-blocked.md) |
| Blocked Item Details | — | [part-phases-2-5-blocked.md](2026-08-03-four-platform-delivery/part-phases-2-5-blocked.md) |
