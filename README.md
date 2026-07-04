# Migo — 为游戏而生的开源原生运行时

[中文](README.md) | [English](README_EN.md)

**嵌入式 HTML5 / 小游戏的 WebView 替代方案。**

Migo 是一个开源、高性能、可嵌入的 HTML5 / Canvas2D / WebGL 原生游戏运行时。App 厂商把它嵌入自己的应用，就能跑 HTML5 与小游戏内容——**无需 WebView**：没有浏览器、DOM、CSS、layout 与合成层的开销，启动更快、内存更低，运行时版本由你打包钉死，不随 OEM 与系统版本漂移。

兼容两个适配 profile，现有游戏**零改动或少量改动**直接运行：

- **跨引擎 HTML5 / Canvas2D / WebGL** —— 零改动跑 Cocos / Egret / Pixi / 原生 Canvas 游戏（`adapter` + `prelude` 提供浏览器风格 BOM/DOM）。
- **主流小游戏 API 风格** —— 兼容主流小游戏平台风格的 API 环境（`wx` 命名空间适配层）。

## 为什么选 Migo（对比 WebView）

| 维度 | Migo | Android System WebView |
|---|---|---|
| 版本一致性 | 运行时由你打包、版本钉死，跨 OEM/系统可控 | 随用户系统与 OEM 自升级、自漂移，你无法控制 |
| 可审计性 | 开源、沙箱边界可逐行审计（fintech/政务/出海合规） | 闭源，无法审计 |
| 启动 / 内存 | 无 DOM/layout、V8 snapshot 预热，体积小、启动快、内存低 | 带整个 Chromium，常驻开销大 |
| 跨引擎兼容 | 同一 API 跑多引擎，不绑定单一引擎 | — |

## 特性

- **WebView 替代** - 嵌入式 HTML5 / 小游戏的原生运行时，无浏览器层开销
- **跨引擎零改动** - Cocos / Egret / Pixi / 原生 Canvas 游戏直接运行
- **开源可审计** - 沙箱边界可逐行审计，合规友好（[BSL 1.1](LICENSE)，2029-01-01 转 Apache 2.0）
- **高性能** - Rust 核心引擎，优化的渲染管线，启动快、内存低
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

```java
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import com.migo.runtime.MigoRuntime;
import com.migo.runtime.RuntimeConfig;
import com.migo.runtime.GameSession;

public class GameActivity extends Activity {
    private SurfaceView surfaceView;
    private GameSession session;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        surfaceView = new SurfaceView(this);
        setContentView(surfaceView);

        surfaceView.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder holder) {
                // 1. 创建配置
                RuntimeConfig config = new RuntimeConfig.Builder(GameActivity.this)
                    .setDebugEnabled(true)
                    .build();

                // 2. 创建会话
                // "gameId" 用于隔离不同游戏的数据目录
                session = MigoRuntime.getInstance().createSession(
                    GameActivity.this,
                    holder.getSurface(),
                    config,
                    "gameId" 
                );

                // 3.1 可选：注册宿主回调（建议在 startGame 前）
                // session.setAuthHandler(authHandler);
                // session.setGameLogHandler(gameLogHandler);
                // session.setSubpackageHandler(subpackageHandler);

                // 3. 启动游戏
                session.startGame("game.js");
            }

            @Override
            public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
                if (session != null) {
                    session.updateSurface(holder.getSurface(), width, height);
                }
            }

            @Override
            public void surfaceDestroyed(SurfaceHolder holder) {
                if (session != null) {
                    session.onSurfaceDestroyed();
                }
            }
        });
    }

    @Override
    protected void onDestroy() {
        if (session != null) {
            session.close();
            session = null;
        }
        super.onDestroy();
    }
}
```

### 从源码构建

#### 前置要求

- Rust 1.85+（需要 Edition 2024），需安装以下 target：
  - `aarch64-linux-android`
  - `x86_64-linux-android`
- Android NDK r23+
- JDK 17+（Android Gradle Plugin 8.x 要求）

#### 构建命令

```bash
# Linux/macOS
./scripts/build-aar.sh release

# Windows (PowerShell)
./scripts/build-aar.ps1 -BuildType release

# 构建指定架构
./scripts/build-aar.sh release arm64-v8a  # Linux/macOS
./scripts/build-aar.ps1 -Architectures arm64-v8a  # Windows
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
│   │   ├── platform/       # 平台相关代码
│   │   └── snapshot-gen/   # V8 快照生成（构建期工具）
│   └── Cargo.toml
├── platforms/
│   └── android/            # Android SDK
├── scripts/                # 构建脚本
├── LICENSE                 # 许可证（BSL 1.1）
├── LEGAL.md                # 法律声明（许可/商标/测试内容）
├── NOTICE                  # 第三方声明
└── README.md
```

## 许可证

Migo 采用 **source-available** 的 [Business Source License 1.1](LICENSE)（BSL 1.1），**2029-01-01 自动转为 Apache 2.0**。许可、商标与测试内容的完整说明见 [LEGAL.md](LEGAL.md)。

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
- [Skia](https://skia.org/) - 2D 图形库（Ganesh GL 后端 + SkParagraph 文本排版）

完整第三方依赖列表与许可证信息请参阅 [NOTICE](NOTICE)。

## 支持

- GitHub Issues：https://github.com/minigame-labs/migo/issues
- 文档/Wiki：https://github.com/minigame-labs/migo/wiki
