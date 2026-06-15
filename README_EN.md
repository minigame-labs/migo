# Migo — The Open Native Runtime for HTML5 & Mini-games

[中文](README.md) | [English](README_EN.md)

**A WebView replacement built for games — embeddable HTML5 / mini-game content without a browser.**

Migo is an open, high-performance, embeddable native runtime for HTML5 / Canvas2D / WebGL games. Embed it in your app to run HTML5 and mini-game content **without a WebView**: no browser, DOM, CSS, layout, or compositor overhead — faster startup, lower memory, and a runtime version you pin yourself instead of one that drifts across OEMs and OS updates.

Two adapter profiles let existing games run with **zero or minimal changes**:

- **Cross-engine HTML5 / Canvas2D / WebGL** — run Cocos / Egret / Pixi / vanilla Canvas games unmodified (`adapter` + `prelude` provide a browser-style BOM/DOM).
- **Mainstream mini-game API style** — a mini-game platform–style API environment (`wx` namespace adapter layer).

## Why Migo (vs. WebView)

| Dimension | Migo | Android System WebView |
|---|---|---|
| Version consistency | Runtime packaged and version-pinned by you; controllable across OEMs/OS versions | Auto-updates and drifts with the user's system and OEM, outside your control |
| Auditability | Open source; sandbox boundary auditable line by line (fintech/gov/overseas compliance) | Closed source, not auditable |
| Startup / memory | No DOM/layout, V8 snapshot warm-up — small footprint, fast startup, low memory | Ships the whole Chromium, heavy resident cost |
| Cross-engine | One API across multiple engines, not locked to a single engine | — |

> **Honest boundaries:** Migo and WebView both run on V8 underneath, so we **do not claim "faster JS"**; the edge is the lightweight no-browser layer, cross-OEM **version determinism**, and open auditability. Rendering is still subject to GPU/driver differences — the pitch is "fragmentation under your control," **not "pixel-identical everywhere."** On iOS, jitless restrictions mean a WKWebView backend and we don't lead with performance there; **Android is the battleground for performance and differentiation.**

## Features

- **WebView replacement** — native runtime for embedded HTML5 / mini-games, no browser-layer overhead
- **Zero-change cross-engine** — Cocos / Egret / Pixi / vanilla Canvas games run unmodified
- **Open & auditable** — sandbox boundary auditable line by line, compliance-friendly ([BSL 1.1](LICENSE), converts to Apache 2.0 on 2029-01-01)
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

> For a complete sample project, see [migo-android-demo](https://github.com/minigame-labs/migo-android-demo)

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
- JDK 11+

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
│   │   ├── core/           # Core runtime
│   │   ├── graphics/       # Rendering (Canvas2D, WebGL)
│   │   ├── audio/          # Audio processing
│   │   ├── io/             # File & network I/O
│   │   ├── js-runtime/     # JavaScript runtime (deno_core)
│   │   ├── shared/         # Shared types & protocols
│   │   └── platform/       # Platform-specific code
│   └── Cargo.toml
├── platforms/
│   └── android/            # Android SDK
├── scripts/                # Build scripts
├── LICENSE                 # License (BSL 1.1)
├── LEGAL.md                # Legal notice (license/trademarks/test content)
├── NOTICE                  # Third-party notices
└── README.md
```

## License

Migo is **source-available** under the [Business Source License 1.1](LICENSE) (BSL 1.1) and **converts to Apache 2.0 on 2029-01-01**. See [LEGAL.md](LEGAL.md) for the full statement on licensing, trademarks, and test content.

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
- [FemtoVG](https://github.com/femtovg/femtovg) - 2D vector graphics library

See [NOTICE](NOTICE) for the full list of dependencies and licenses.

## Support

- Issues: https://github.com/minigame-labs/migo/issues
- Wiki/Docs: https://github.com/minigame-labs/migo/wiki
