# Contributing to Migo

Thanks for your interest in contributing to **Migo**! This document explains how to report issues, propose changes, and submit pull requests.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Ways to Contribute](#ways-to-contribute)
- [Development Setup](#development-setup)
- [Coding Guidelines](#coding-guidelines)
- [Pull Request Process](#pull-request-process)
- [Contributor License Agreement (CLA)](#contributor-license-agreement-cla)
- [Questions](#questions)

## Code of Conduct

Be respectful and constructive. Harassment, discrimination, or abusive behavior will not be tolerated.

## Ways to Contribute

### Reporting Bugs

Before opening a bug report:

1. Search existing issues to avoid duplicates
2. Try the latest `main` to confirm the bug still exists
3. Collect key information (OS/device, versions, logs, minimal repro)

When reporting, include:

- Clear title
- Steps to reproduce
- Expected vs actual behavior
- Logs/screenshots (if applicable)
- Environment details (OS, device, Rust/NDK/JDK versions)

### Suggesting Features

Feature requests are welcome. Please include:

- The problem/use case
- Why it matters (impact, who benefits)
- Proposed approach (if you have one)
- Alternatives considered

### Submitting Code

1. Fork the repo
2. Create a branch: `git checkout -b feat/your-change` (or `fix/...`)
3. Make changes + add/update tests
4. Ensure formatting/lints/tests pass
5. Open a PR

## Development Setup

> For the full, platform-specific build guide (Linux / macOS / Windows),
> including Skia-from-source requirements and troubleshooting, see
> [`docs/BUILD.md`](docs/BUILD.md). This section is the short version.

### Prerequisites

- Rust 1.80+ (edition 2024)
- Android NDK r23+ (r23b or r25c recommended) — Android targets
- JDK 17+ (for AAR builds)
- `cargo-ndk`, `python3`, `ninja`, `git` (Android builds compile Skia from source)

### Clone

```bash
git clone https://github.com/minigame-labs/migo.git
cd migo
```

### Build & Test (Rust)

```bash
cd engine
cargo build
cargo test
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
```

### Build Android AAR (if applicable)

```bash
# Linux / macOS
bash scripts/build-aar.sh release

# Windows
.\scripts\build-aar.ps1 release
```

For ABI selection, single-step `.so` builds, and troubleshooting (NDK
env vars, Skia source-build deps, proxy setup, WSL2 memory), see
[`docs/BUILD.md`](docs/BUILD.md).

## Coding Guidelines

### Rust

- Follow the Rust API Guidelines: https://rust-lang.github.io/api-guidelines/
- Format with `cargo fmt`
- Lint with `cargo clippy`
- Add docs for public APIs
- Keep functions small and focused

### JavaScript / TypeScript (if present)

- Keep style consistent with the existing code
- Prefer clear naming over cleverness
- Comment non-obvious logic

### Commit Messages

We prefer **Conventional Commits**: https://www.conventionalcommits.org/

Examples:

- `feat(audio): add streaming playback`
- `fix(graphics): prevent context leak on resume`
- `docs: update Android integration guide`

## Pull Request Process

### Before you open a PR

- [ ] `cargo fmt` (clean)
- [ ] `cargo clippy` (no warnings)
- [ ] `cargo test` (all pass)
- [ ] Docs updated if behavior changes
- [ ] PR is focused (one feature/fix per PR)

### PR Title Format

Use Conventional Commits style, for example:

- `feat(graphics): add WebGL2 baseline support`
- `fix(io): handle missing asset index gracefully`

### Review & Merge

A maintainer will review your PR. You may be asked to adjust the implementation, tests, or docs before merge.

## Contributor License Agreement (CLA)

Migo is licensed under **Business Source License 1.1 (BSL 1.1)** and may also be made available under additional licenses (e.g., the Change License defined in `LICENSE`) over time.  
To protect both contributors and the project, we require a CLA for code contributions.

### How to agree to the CLA

For individual contributors: add this statement in your first PR comment:

```
I have read and agree to the CLA in CLA.md.
```

For corporate contributors: please open an issue labeled `cla` (or contact the maintainers) to arrange a corporate CLA.

> If you are unsure whether you can contribute code from your employer’s time/equipment, please confirm with your employer first.

## Questions

- Use GitHub Issues for bugs and tasks: https://github.com/minigame-labs/migo/issues
- (Optional) Enable Discussions for Q&A and ideas in repo settings, then link it here.