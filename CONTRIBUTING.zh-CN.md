# 参与贡献 Migo

[English](CONTRIBUTING.md) | [中文](CONTRIBUTING.zh-CN.md)

感谢你对贡献 **Migo** 感兴趣！本文档说明如何报告问题、提出改动、提交 Pull Request。

## 目录

- [行为准则](#行为准则)
- [参与方式](#参与方式)
- [开发环境搭建](#开发环境搭建)
- [编码规范](#编码规范)
- [Pull Request 流程](#pull-request-流程)
- [贡献者许可协议（CLA）](#贡献者许可协议cla)
- [问题](#问题)

## 行为准则

请保持尊重与建设性。不容忍骚扰、歧视或辱骂行为。

## 参与方式

### 报告 Bug

提交 bug 报告前：

1. 先搜索现有 issue，避免重复
2. 用最新的 `main` 分支确认问题仍然存在
3. 收集关键信息（操作系统/设备、版本、日志、最小复现）

报告时请包含：

- 清晰的标题
- 复现步骤
- 预期行为与实际行为
- 日志/截图（如适用）
- 环境细节（操作系统、设备、Rust/NDK/JDK 版本）

### 提出功能建议

欢迎功能请求。请包含：

- 问题/使用场景
- 为什么重要（影响面、受益方）
- 你设想的实现方式（如果有）
- 考虑过的其他方案

### 提交代码

1. Fork 本仓库
2. 创建分支：`git checkout -b feat/your-change`（或 `fix/...`）
3. 做出改动 + 新增/更新测试
4. 确保格式化/lint/测试都通过
5. 开一个 PR

## 开发环境搭建

> 完整的、按平台区分的构建指南（Linux / macOS / Windows），包括 Skia
> 从源码构建的要求与排障，见 [`BUILD.md`](BUILD.md)。本节是精简版。

### 前置条件

- 通过 `rustup` 安装 Rust —— `engine/rust-toolchain.toml` 已钉定确切版本（edition 2024 要求 rustc ≥ 1.85）
- Android NDK r23+（推荐 r23b 或 r25c）—— Android 目标需要
- JDK 17+（AAR 构建需要）
- `python3`、`ninja`、`git`（Skia 始终从源码编译，主机构建也不例外）
- `cargo-ndk`（Android 目标需要）

### 克隆

```bash
git clone https://github.com/minigame-labs/migo.git
cd migo
```

### 构建与测试（Rust）

```bash
cd engine
cargo build
cargo test
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
```

首次构建会从源码编译 Skia（约 30–50 分钟；需要 `python3`、`ninja` 及若干
`-dev` 头文件 —— 见 [`BUILD.md`](BUILD.md)）。在精简的 Ubuntu / WSL2 主机上，
`bash scripts/dev-test-host.sh test --workspace --lib` 会替你配好这些。
（此处**不能**用 `--all-features`：`graphics` crate 的 `profile-*` 与
`*_icudtl` 特性互斥。）

### 构建 Android AAR（如适用）

```bash
# Linux / macOS
bash scripts/build-aar.sh release

# Windows
.\scripts\build-aar.ps1 release
```

关于 ABI 选择、单步 `.so` 构建、排障（NDK 环境变量、Skia 源码构建依赖、
代理设置、WSL2 内存），见 [`BUILD.md`](BUILD.md)。

## 编码规范

### Rust

- 遵循 Rust API Guidelines：https://rust-lang.github.io/api-guidelines/
- 用 `cargo fmt` 格式化
- 用 `cargo clippy` 做 lint
- 为公开 API 添加文档
- 保持函数短小、职责单一

### JavaScript / TypeScript（如有）

- 保持与现有代码风格一致
- 命名清晰优先于取巧
- 为非显而易见的逻辑加注释

### Commit 信息

我们偏好 **Conventional Commits** 风格：https://www.conventionalcommits.org/

示例：

- `feat(audio): add streaming playback`
- `fix(graphics): prevent context leak on resume`
- `docs: update Android integration guide`

## Pull Request 流程

### 开 PR 前

- [ ] `cargo fmt`（干净无改动）
- [ ] `cargo clippy`（无警告）
- [ ] `cargo test`（全部通过）
- [ ] 行为有变化时同步更新文档
- [ ] PR 聚焦（一个 PR 一个功能/修复）

### PR 标题格式

使用 Conventional Commits 风格，例如：

- `feat(graphics): add WebGL2 baseline support`
- `fix(io): handle missing asset index gracefully`

### 审核与合并

维护者会审核你的 PR。可能会要求你调整实现、测试或文档后再合并。

## 贡献者许可协议（CLA）

Migo 采用 **Business Source License 1.1（BSL 1.1）** 授权，未来也可能在此基础上
另外提供其他许可（例如 `LICENSE` 中定义的 Change License）。为了同时保护贡献者
和项目本身，代码贡献需要签署 CLA。

### 如何同意 CLA

个人贡献者：在你的第一个 PR 评论中添加这句话：

```
I have read and agree to the CLA in CLA.md.
```

企业贡献者：请开一个标记为 `cla` 的 issue（或联系维护者）来安排企业 CLA。

> 如果你不确定能否用雇主的工作时间/设备贡献代码，请先与雇主确认。

## 问题

- Bug 与任务请用 GitHub Issues：https://github.com/minigame-labs/migo/issues
