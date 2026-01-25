# Migo — Mini-game Runtime Engine

[中文](README.md) | [English](README_EN.md)

High-performance, cross-platform mini-game runtime engine for mobile apps.

Migo enables app developers to run mini-games inside their own apps, providing a mainstream mini-game platform–style API environment with near-native performance.

## Features

- **Mainstream mini-game API style** — Run existing mini-games with zero or minimal changes
- **High performance** — Rust core engine with an optimized rendering pipeline
- **Canvas 2D & WebGL** — Full support for 2D canvas and WebGL rendering
- **Audio support** — WebAudio-style APIs (including streaming playback)
- **Cross-platform** — Android, iOS, and Windows 
- **Lightweight** — Small footprint and fast startup

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
|                                    Rust Core Engine                                |
+------------------------------------------------------------------------------------+
|                       Platform Layer (Android / iOS / Windows)                     |
+------------------------------------------------------------------------------------+
```

## Quick Start

### Android Integration

1. Add the AAR dependency:

```gradle
dependencies {
    implementation files("libs/migo.aar")
}
```

2. Initialize the engine in your Activity:

```kotlin
class GameActivity : Activity() {
    private lateinit var gameView: MiniGameView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        gameView = MiniGameView(this)
        setContentView(gameView)

        // Load and run a mini-game
        gameView.loadGame("path/to/game")
    }
}
```

### Build from Source

#### Prerequisites

- Rust 1.75+ with the following targets:
  - `aarch64-linux-android`
  - `armv7-linux-androideabi`
  - `x86_64-linux-android`
- Android NDK r23+
- JDK 11+

#### Build Commands

```bash
# Build Android AAR (PowerShell)
./scripts/build-aar.ps1 -BuildType release

# Build for a specific ABI
./scripts/build-aar.ps1 -Architectures arm64-v8a
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
├── LICENSE                 # License
├── NOTICE                  # Third-party notices
└── README.md
```

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