# Migo — The Native Runtime for HTML5 & Mini-games

[English](README.md) | [中文](README.zh-CN.md)

**A WebView replacement built for games — embeddable HTML5 / mini-game content without a browser.**

Migo is a source-available, high-performance, embeddable native runtime for HTML5 / Canvas2D / WebGL games. Embed it in your app to run HTML5 and mini-game content **without a WebView**: no browser, DOM, CSS, layout, or compositor overhead — faster startup, lower memory, and a runtime version you pin yourself instead of one that drifts across OEMs and OS updates.

Two adapter profiles let existing games run with **zero or minimal changes**:

- **Cross-engine HTML5 / Canvas2D / WebGL** — run Cocos / Egret / Pixi / vanilla Canvas games unmodified (`adapter` + `prelude` provide a browser-style BOM/DOM).
- **Mainstream mini-game API style** — a mini-game platform–style API environment (`wx` namespace adapter layer).

## Why Migo (vs. WebView)

| Dimension | Migo | Android System WebView |
|---|---|---|
| Version consistency | Runtime packaged and version-pinned by you; controllable across OEMs/OS versions | Auto-updates and drifts with the user's system and OEM, outside your control |
| Auditability | Source-available; sandbox boundary auditable line by line (fintech/gov/overseas compliance) | Closed source, not auditable |
| Startup / memory | No DOM/layout, V8 snapshot warm-up — small footprint, fast startup, low memory | Ships the whole Chromium, heavy resident cost |
| Cross-engine | One API across multiple engines, not locked to a single engine | — |

## Features

- **WebView replacement** — native runtime for embedded HTML5 / mini-games, no browser-layer overhead
- **Zero-change cross-engine** — Cocos / Egret / Pixi / vanilla Canvas games run unmodified
- **Source-available & auditable** — sandbox boundary auditable line by line, compliance-friendly ([BSL 1.1](LICENSE); each release converts to Apache 2.0 four years after it ships)
- **High performance** — Rust core engine with an optimized rendering pipeline; fast startup, low memory
- **Canvas 2D & WebGL** — full support for 2D canvas and WebGL rendering
- **Audio support** — WebAudio-style APIs (including streaming playback)
- **Cross-platform** — Android (iOS and Windows planned)
- **Lightweight** — small footprint and fast startup

## Architecture

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

## Quick Start

### Android Integration

> For complete sample projects, see [migo-examples](https://github.com/minigame-labs/migo-examples)

1. Add the AAR dependency:

```gradle
dependencies {
    implementation files("libs/migo.aar")
}
```

2. Initialize the engine in your Activity:

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
                // 1. Create configuration
                RuntimeConfig config = new RuntimeConfig.Builder(GameActivity.this)
                    .setDebugEnabled(true)
                    .build();

                // 2. Create session
                // "gameId" is used to isolate data directories for different games
                session = MigoRuntime.getInstance().createSession(
                    GameActivity.this,
                    holder.getSurface(),
                    config,
                    "gameId"
                );

                // 3.1 Optional: register host handlers before startGame
                // session.setAuthHandler(authHandler);
                // session.setGameLogHandler(gameLogHandler);
                // session.setSubpackageHandler(subpackageHandler);

                // 3. Start game
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

### Build from Source

#### Prerequisites

- Rust 1.85+ (Edition 2024 required), with targets:
  - `aarch64-linux-android`
  - `x86_64-linux-android`
- Android NDK r23+
- JDK 17+ (required by Android Gradle Plugin 8.x)

#### Build Commands

```bash
# Linux/macOS
./scripts/build-aar.sh release

# Windows (PowerShell)
./scripts/build-aar.ps1 -BuildType release

# Build for specific architecture
./scripts/build-aar.sh release arm64-v8a  # Linux/macOS
./scripts/build-aar.ps1 -Architectures arm64-v8a  # Windows
```

## Project Structure

```text
migo/
├── engine/                 # Rust core engine
│   ├── crates/
│   │   ├── core/           # core runtime and session lifecycle
│   │   ├── graphics/       # rendering (Canvas2D, WebGL)
│   │   ├── audio/          # audio
│   │   ├── io/             # file and network I/O
│   │   ├── runtime-v8/     # JavaScript runtime (V8 via deno_core)
│   │   ├── shared/         # shared types and protocol
│   │   ├── platform/       # platform integration
│   │   ├── capi/           # C ABI implementation
│   │   ├── capi-abi/       # C ABI layout and versioning contract
│   │   └── android-jni/    # JNI entry points (libmigo.so)
│   ├── tools/              # snapshot-gen, headless player, C host example
│   └── Cargo.toml
├── adapter/                # HTML5 -> mini-game API adapter (JavaScript)
├── include/migo/           # public C headers
├── platforms/
│   ├── android/            # Android SDK (AAR)
│   ├── linux/              # Linux host kit (Qt 6 / X11)
│   └── windows/            # Windows
├── tests/                  # conformance assets (C ABI lanes, C hosts, probes)
├── contracts/              # artifact manifest schemas
├── scripts/                # build and contract-gate scripts
├── BUILD.md                # building from source (prerequisites, flow, common errors)
├── LICENSE                 # licence (BSL 1.1)
├── LEGAL.md                # legal notice (licence / trademark / test content)
├── COMMERCIAL.md           # commercial licence: who needs one, who does not
├── NOTICE                  # third-party notices
└── README.md
```

## License

Migo is **source-available** under the [Business Source License 1.1](LICENSE) (BSL 1.1). **Each released version converts to Apache 2.0 four years after it is published.**

- **Read, audit, build, test, benchmark, modify and port** — granted to everyone, at any scale, unconditionally.
- **Ship Migo inside your own app** — free while under USD 1,000,000 annual revenue and 3,000,000 MAU.
- **Resell Migo as a standalone SDK, or run it as a hosted service** — needs a commercial license.

See [LEGAL.md](LEGAL.md) for the full statement, and [COMMERCIAL.md](COMMERCIAL.md) for commercial licensing.

"Migo" and the Migo logo are trademarks of the Migo Authors. The license grants rights in the **software**, not in the name or logo: you may fork the code, but not the name.

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md).

Before submitting a Pull Request, please ensure:

1. Code style matches the existing codebase
2. Tests are added/updated for new functionality
3. Documentation is updated where relevant
4. CLA/DCO requirements are met

## Acknowledgements

Migo is built on top of these great open-source projects:

- [Deno Core](https://github.com/denoland/deno_core) - JavaScript/TypeScript runtime foundation
- [V8](https://v8.dev/) - JavaScript engine
- [Tokio](https://tokio.rs/) - Rust async runtime
- [Skia](https://skia.org/) - 2D graphics library (Ganesh GL backend + SkParagraph text layout)

See [NOTICE](NOTICE) for the full list of dependencies and licenses.

## Support

- Issues: https://github.com/minigame-labs/migo/issues
- Wiki/Docs: https://github.com/minigame-labs/migo/wiki
