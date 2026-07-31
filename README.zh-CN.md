# Migo — 为游戏而生的原生运行时

[English](README.md) | [中文](README.zh-CN.md)

[![CI](https://github.com/minigame-labs/migo/actions/workflows/pr-ci.yml/badge.svg)](https://github.com/minigame-labs/migo/actions/workflows/pr-ci.yml)
[![License](https://img.shields.io/badge/license-BSL%201.1-blue.svg)](LICENSE)

**为游戏而生的 WebView 替代方案。** 把 Migo 嵌进你的 App，就能原生运行 HTML5 与小游戏内容——没有浏览器、DOM、CSS 与合成层。启动更快、内存更低，运行时版本由你自己钉死，不随 OEM 与系统版本漂移。

两套适配层让现有游戏零改动或极小改动即可运行：

- **跨引擎 HTML5 / Canvas2D / WebGL** —— Cocos、Egret、Pixi 与原生 Canvas 游戏无需修改；适配层提供浏览器风格的 BOM/DOM。
- **小游戏 API 风格** —— 通过 `wx` 命名空间适配层提供小游戏平台风格的 API 环境。

## 为什么用 Migo

| | Migo | Android 系统 WebView |
|---|---|---|
| **版本一致性** | 运行时由你打包并钉死版本，跨 OEM 与系统版本完全一致 | 随用户系统自动更新，不受你控制 |
| **可审计性** | 源码可获取，沙箱边界可逐行审计 | 闭源 |
| **启动 / 内存** | 无 DOM 与 layout，V8 快照预热——体积小、启动快 | 打包整个 Chromium，常驻开销重 |
| **跨引擎** | 一套 API 覆盖多引擎 | — |

与系统 WebView 的可复现对比测试（同游戏、同设备、同 session）见 [migo-bench](https://github.com/minigame-labs/migo-bench)。

## 平台支持

| 平台 | 状态 | 已发布产物 |
|---|---|---|
| **Android**（arm64-v8a、x86_64） | 已发布 | 含 Java/Kotlin SDK 的 AAR;按 ABI 发布的 C ABI 包(头文件、静态库、CMake 包) |
| **Linux**（x86_64） | 已发布 | 静态库与共享库、pkg-config 与 CMake 包;Qt 6 / X11 host kit 在仓库内 |
| **Windows**（x86_64） | 已发布 | `migo.dll` 及其导入库、头文件、CMake 包,以及它按名加载的 ANGLE 与 V8 运行时 DLL |
| **OpenHarmony / HarmonyOS NEXT**（aarch64、x86_64） | 可构建、已门禁 | 按架构产出的 C ABI 包(头文件、静态库、CMake 包、manifest),一条命令产出并自带门禁;尚未上 releases 页面 |
| iOS、macOS | 计划中 | — |

已发布产物见 [releases 页面](https://github.com/minigame-labs/migo/releases)。每个产物都带
`.attestation.json`,记录归档的名称、大小与 sha256 —— 使用前请对照校验。

[`include/migo/`](include/migo/) 中的 C ABI 目前是 **candidate**——在 Android、Linux 与 OpenHarmony 上已有可用运行时，但尚未冻结。冻结前还剩哪些事项，以该目录下的 README 为准。

## 快速开始

**把 Migo 集成进 App** —— 各平台宿主的完整示例（含构建与运行步骤）在 [migo-examples](https://github.com/minigame-labs/migo-examples)。请从那里开始而不是本仓库：它自带可运行的游戏，并会替你解析运行时产物。

**从源码构建运行时** —— 前置条件、各平台配置与构建流程见 [BUILD.md](BUILD.md)。

```bash
# Android AAR（在 Linux/macOS 上构建）
./scripts/build-aar.sh release arm64-v8a
```

预构建的 V8 归档不入库，而是下载后对其 component manifest 校验：

```bash
bash scripts/fetch-v8-archives.sh          # Android 目标（构建默认）
bash scripts/fetch-v8-archives.sh --all    # 所有带 manifest 的目标
```

## 架构

```text
+------------------------------------------------------------------------------------+
|                                      Your App                                      |
+------------------------------------------------------------------------------------+
|                                      Migo SDK                                      |
+---------------------+--------------------+--------------------+--------------------+
|       Graphics      |       Audio        |        I/O         |     JS Runtime     |
|     (Skia / GL)     |     (WebAudio)     |     (File/Net)     |   (deno_core/V8)   |
+---------------------+--------------------+--------------------+--------------------+
|                                  Rust Core Engine                                  |
+------------------------------------------------------------------------------------+
|                     Platform Layer (Android | Linux | Windows)                     |
+------------------------------------------------------------------------------------+
```

## 仓库结构

```text
migo/
├── engine/                 # Rust 核心引擎
│   ├── crates/
│   │   ├── core/           # 核心运行时与会话生命周期
│   │   ├── graphics/       # 渲染（Canvas2D, WebGL）
│   │   ├── audio/          # 音频处理
│   │   ├── io/             # 文件与网络 I/O
│   │   ├── runtime-v8/     # JavaScript 运行时（V8，经 deno_core）
│   │   ├── shared/         # 共享类型与协议
│   │   ├── platform/       # 平台相关代码
│   │   ├── capi/           # C ABI 实现
│   │   ├── capi-abi/       # C ABI 布局与版本契约
│   │   └── android-jni/    # JNI 入口（libmigo.so）
│   ├── tools/              # 快照生成、headless player、C 宿主示例
│   └── Cargo.toml
├── adapter/                # HTML5 -> 小游戏 API 适配层（JavaScript）
├── include/migo/           # 公开 C 头文件
├── platforms/
│   ├── android/            # Android SDK（AAR）
│   ├── linux/              # Linux host kit（Qt 6 / X11）
│   ├── openharmony/        # OpenHarmony 宿主（ArkUI XComponent）
│   └── windows/            # Windows
├── tests/                  # 一致性资产（C ABI lane、C 宿主、探针内容）
├── contracts/              # 产物 manifest schema
├── scripts/                # 构建与契约门禁脚本
├── BUILD.md                # 从源码构建指南
├── LICENSE                 # 许可证（BSL 1.1）
├── LEGAL.md                # 法律声明（许可/商标/测试内容）
├── COMMERCIAL.md           # 商业许可：谁需要、谁不需要、怎么谈
└── NOTICE                  # 第三方声明
```

## 相关仓库

| 仓库 | 用途 |
|---|---|
| [migo-examples](https://github.com/minigame-labs/migo-examples) | 各平台宿主集成示例，一个平台一个目录 |
| [migo-bench](https://github.com/minigame-labs/migo-bench) | Migo 与 WebView 的可复现对比测试 |
| [migo-test-suite](https://github.com/minigame-labs/migo-test-suite) | 小游戏 API 兼容性测试套件 |

## 许可证

Migo 采用 **source-available** 的 [Business Source License 1.1](LICENSE)（BSL 1.1）。**每个发布版本在其发布满 4 年时自动转为 Apache 2.0。**

- **阅读、审计、构建、测试、评测、修改、移植** —— 任何规模、任何主体，无条件授予。
- **把 Migo 嵌进你自己的 App 上线** —— 年营收 ≤ USD 1,000,000 且月活 ≤ 3,000,000 时免费。
- **把 Migo 作为独立 SDK 转售，或提供托管运行时服务** —— 需要商业许可。

完整声明见 [LEGAL.md](LEGAL.md)，商业许可见 [COMMERCIAL.md](COMMERCIAL.md)。

"Migo" 及 Migo logo 是 Migo Authors 的商标。许可证授予的是**软件**上的权利，不含名称：你可以 fork 代码，但不能沿用名称。

## 参与贡献

欢迎贡献 —— 见 [CONTRIBUTING.md](CONTRIBUTING.md)。提交 Pull Request 前，请确认 `scripts/test-*-contract.sh` 下的契约门禁仍然通过；它们编码了普通测试覆盖不到的不变量。

## 致谢

Migo 构建于 [Deno Core](https://github.com/denoland/deno_core)、[V8](https://v8.dev/)、[Tokio](https://tokio.rs/) 与 [Skia](https://skia.org/)（Ganesh GL 后端 + SkParagraph 文本排版）之上。完整依赖与许可清单见 [NOTICE](NOTICE)。

## 支持

- Issues: https://github.com/minigame-labs/migo/issues
- 文档: https://github.com/minigame-labs/migo/wiki
