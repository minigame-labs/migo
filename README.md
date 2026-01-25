# Migo 小游戏运行时引擎

[中文](README.md) | [English](README_EN.md)

面向移动应用的跨平台高性能小游戏运行时引擎。

Migo 让 App 厂商可以在自己的应用中运行小游戏：提供**兼容主流小游戏平台风格的 API 环境**，同时具备接近原生的性能表现。

## 特性

- **主流小游戏 API 风格** - 现有小游戏可零改动或少量改动直接运行
- **高性能** - Rust 核心引擎，优化的渲染管线
- **Canvas 2D & WebGL** - 完整支持 2D 画布与 WebGL 渲染
- **音频支持** - WebAudio 风格 API（含流式播放能力）
- **跨平台** - 支持 Android（iOS、Windows 计划中）
- **轻量级** - 体积小，启动快

## 架构

```text
+------------------------------------------------------------------------------------+
|                                      Your App                                      |
+------------------------------------------------------------------------------------+
|                                      Migo SDK                                      |
+---------------------+------------------- +------------------+----------------------+
|      Graphics       |       Audio        |        I/O       |      JS Runtime      |
|      (OpenGL)       |     (WebAudio)     |     (File/Net)   |    (deno_core/V8)    |
+---------------------+--------------------+------------------+----------------------+
|                                   Rust Core Engine                                 |
+------------------------------------------------------------------------------------+
|                         Platform Layer (Android | iOS | Windows)                   |
+------------------------------------------------------------------------------------+
```

## 快速开始

### Android 集成

> 完整示例项目请参考 [migo-android-demo](https://github.com/minigame-labs/migo-android-demo)

1. 添加 AAR 依赖到项目：

```gradle
dependencies {
    implementation files("libs/migo.aar")
}
```

2. 在 Activity 中初始化引擎：

```kotlin
class GameActivity : Activity() {
    private lateinit var gameView: MiniGameView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        gameView = MiniGameView(this)
        setContentView(gameView)

        // 加载并运行小游戏
        gameView.loadGame("path/to/game")
    }
}
```

### 从源码构建

#### 前置要求

- Rust 1.75+，需安装以下 target：
  - `aarch64-linux-android`
  - `armv7-linux-androideabi`
  - `x86_64-linux-android`
- Android NDK r23+
- JDK 11+

#### 构建命令

```bash
# 构建 Android AAR（PowerShell）
./scripts/build-aar.ps1 -BuildType release

# 构建指定架构
./scripts/build-aar.ps1 -Architectures arm64-v8a
```

## 项目结构

```text
migo/
├── engine/                 # Rust 核心引擎
│   ├── crates/
│   │   ├── core/           # 核心运行时
│   │   ├── graphics/       # 渲染（Canvas2D, WebGL）
│   │   ├── audio/          # 音频处理
│   │   ├── io/             # 文件与网络 I/O
│   │   ├── js-runtime/     # JavaScript 运行时（deno_core）
│   │   ├── shared/         # 共享类型与协议
│   │   └── platform/       # 平台相关代码
│   └── Cargo.toml
├── platforms/
│   └── android/            # Android SDK
├── scripts/                # 构建脚本
├── LICENSE                 # 许可证
├── NOTICE                  # 第三方声明
└── README.md
```

## 贡献

欢迎贡献代码！请参阅 [CONTRIBUTING.md](CONTRIBUTING.md) 了解贡献指南。

提交 Pull Request 前请确保：

1. 代码风格与现有代码保持一致
2. 为新功能添加/更新测试
3. 更新相关文档
4. 签署贡献者许可协议 (CLA)

## 致谢

Migo 基于以下优秀的开源项目构建：

- [Deno Core](https://github.com/denoland/deno_core) - JavaScript/TypeScript 运行时基础
- [V8](https://v8.dev/) - JavaScript 引擎
- [Tokio](https://tokio.rs/) - Rust 异步运行时
- [FemtoVG](https://github.com/femtovg/femtovg) - 2D 矢量图形库

完整第三方依赖列表与许可证信息请参阅 [NOTICE](NOTICE)。

## 支持

- GitHub Issues：https://github.com/minigame-labs/migo/issues
- 文档/Wiki：https://github.com/minigame-labs/migo/wiki