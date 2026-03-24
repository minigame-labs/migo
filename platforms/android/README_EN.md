# Migo Runtime SDK for Android

A lightweight, high-performance JavaScript game runtime for Android.

## Features

- 🚀 **High Performance** - Native Rust engine with OpenGL ES rendering
- 📱 **API 21+** - Supports Android 5.0 Lollipop and above
- 🔧 **Zero Dependencies** - No AndroidX, Kotlin, or third-party libraries required
- 🎮 **Game Ready** - Canvas 2D and WebGL support
- 🔒 **Sandboxed Filesystem** - Isolated file storage per game

## Installation

### Gradle

Add the AAR to your project:

```groovy
dependencies {
    implementation files('libs/migo-release.aar')
}
```

Or if using a Maven repository:

```groovy
dependencies {
    implementation 'com.migo:runtime:1.0.0'
}
```

## Quick Start

### Basic Usage

```java
import com.migo.runtime.MigoRuntime;
import com.migo.runtime.GameSession;
import com.migo.runtime.RuntimeConfig;

public class GameActivity extends Activity {
    private GameSession session;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        
        // 1. Create a SurfaceView for rendering
        SurfaceView surfaceView = new SurfaceView(this);
        setContentView(surfaceView);

        // 2. Configure the runtime
        RuntimeConfig config = new RuntimeConfig.Builder(this)
            .setDebugEnabled(BuildConfig.DEBUG)
            .setLogLevel(RuntimeConfig.LogLevel.DEBUG)
            .setTargetFps(60)
            .build();

        // 3. Set up surface callbacks
        surfaceView.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder holder) {
                // 4. Create game session (each game needs a unique gameId)
                session = MigoRuntime.getInstance()
                    .createSession(GameActivity.this, holder.getSurface(), config, "my-game-id");
                
                // 5. Start the game (loads from isolated code directory)
                // Place game code in: session.getPaths().getCodeDir()
                session.startGame("game.js");
            }

            @Override
            public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
                if (session != null) {
                    session.updateSurface(holder.getSurface());
                }
            }

            @Override
            public void surfaceDestroyed(SurfaceHolder holder) {
                if (session != null) {
                    session.close();
                    session = null;
                }
            }
        });

        // 6. Handle touch events
        surfaceView.setOnTouchListener((v, event) -> {
            if (session != null) {
                return session.dispatchTouchEvent(event);
            }
            return false;
        });
    }

    @Override
    protected void onPause() {
        super.onPause();
        if (session != null) session.pause();
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (session != null) session.resume();
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

### With Event Callbacks

```java
// Set up unified session listener
session.setListener(new GameSessionListener() {
    @Override
    public void onGameReady() {
        Log.i(TAG, "Game is ready!");
        hideLoadingScreen();
    }

    @Override
    public void onGameExit(int exitCode) {
        Log.i(TAG, "Game exited with code: " + exitCode);
        finish();
    }

    @Override
    public void onError(int errorCode, String message, boolean recoverable) {
        Log.e(TAG, "Error " + errorCode + ": " + message);
        if (!recoverable) {
            showErrorDialog(message);
        }
    }

    // Optional: override onLoadingProgress / onPaused / onResumed etc.
});
```

### Safe Session Creation (Non-throwing)

```java
MigoRuntime.Result<GameSession> result = MigoRuntime.getInstance()
    .createSessionSafe(activity, surface, config, "my-game-id");

if (result.isSuccess()) {
    session = result.getValue();
    session.startGame("game.js");  // uses paths.getCodeDir()
} else {
    int errorCode = result.getErrorCode();
    String message = result.getErrorMessage();
    Log.e(TAG, "Failed to create session: " + ErrorCode.getMessage(errorCode));
}
```

### Register Host Handlers (Auth / GameLog / Subpackage)

In the latest API, `GameSession` supports three host callback handlers.
Register them before `startGame()` whenever possible.

```java
session.setAuthHandler(new AuthHandler() {
    @Override
    public void login(int timeoutMs, LoginCallback callback) {
        callback.onFailure("not implemented");
    }

    @Override
    public void checkSession(CheckSessionCallback callback) {
        callback.onFailure("not implemented");
    }
});

session.setGameLogHandler(logJson -> {
    Log.i("GameLog", logJson);
});

session.setSubpackageHandler((request, callback) -> {
    callback.onFailure("download not implemented");
});
```

- `setAuthHandler(AuthHandler)`: backs `wx.login` / `wx.checkSession` / `wx.getUserInfo` / `wx.getPhoneNumber`
- `setGameLogHandler(GameLogHandler)`: receives game-reported logs (JSON string)
- `setSubpackageHandler(SubpackageHandler)`: handles `loadSubpackage` / `preDownloadSubpackage` downloads

## Configuration Options

```java
RuntimeConfig config = new RuntimeConfig.Builder(context)
    // Performance
    .setTargetFps(60)              // 30-120, default: 60
    
    // Debugging
    .setDebugEnabled(true)         // Enable debug features
    .setLogLevel(LogLevel.DEBUG)   // TRACE, DEBUG, INFO, WARN, ERROR, OFF
    
    // Directories
    .setCodeCacheDir(codeCacheDir) // For compiled code
    
    .build();
```

## Error Handling

The SDK uses structured error codes for all operations:

```java
// Error codes
ErrorCode.SUCCESS               //  0: Success
ErrorCode.ERR_INIT_FAILED       // -1000: Initialization failed
ErrorCode.ERR_INVALID_SURFACE   // -1001: Invalid Surface
ErrorCode.ERR_INVALID_CONFIG    // -1002: Invalid configuration
ErrorCode.ERR_NATIVE_LOAD_FAILED// -1003: Native library load failed
ErrorCode.ERR_SESSION_DESTROYED // -2000: Session destroyed
ErrorCode.ERR_CODE_DIR_NOT_FOUND// -2002: Code directory not found
ErrorCode.ERR_ENTRY_NOT_FOUND   // -2003: Entry point not found
ErrorCode.ERR_JS_EXECUTION      // -2004: JavaScript execution error
ErrorCode.ERR_INVALID_ACTIVITY  // -5004: Invalid Activity

// Get human-readable message
String message = ErrorCode.getMessage(code);
```

## API Reference

### MigoRuntime

The main entry point (singleton):

| Method | Description |
|--------|-------------|
| `getInstance()` | Get the singleton instance |
| `createSession(Activity, Surface, RuntimeConfig, String gameId)` | Create a game session (Activity-bound) |
| `createSession(Context, Surface, RuntimeConfig, String gameId)` | Create a game session (without Activity binding) |
| `createSessionSafe(Activity, Surface, RuntimeConfig, String gameId)` | Non-throwing version |
| `getVersion()` | Get SDK version |
| `getNativeVersion()` | Get native engine version |
| `isNativeLoaded()` | Check if native library loaded |
| `isDeviceSupported()` | Check device compatibility |
| `getActiveSessionCount()` | Get active session count |
| `getMinSdkVersion()` | Get minimum supported API level |

### GameSession

Represents an active game session (implements `Closeable`):

| Method | Description |
|--------|-------------|
| `startGame(String entryPoint)` | Start game (from `paths.getCodeDir()`) |
| `startGameSafe(String entryPoint)` | Non-throwing version |
| `pause()` | Pause the game |
| `resume()` | Resume the game |
| `restart()` | Restart the game |
| `updateSurface(Surface)` | Update rendering surface |
| `dispatchTouchEvent(MotionEvent)` | Handle touch input |
| `dispatchMemoryWarning(int)` | Forward memory pressure signal |
| `setListener(GameSessionListener)` | Register unified session listener |
| `setAuthHandler(AuthHandler)` | Register auth handler |
| `setGameLogHandler(GameLogHandler)` | Register game log handler |
| `setSubpackageHandler(SubpackageHandler)` | Register subpackage download handler |
| `close()` / `destroy()` | Release resources |
| `isValid()` | Check if session is valid |
| `isGameStarted()` | Check if game started |

### RuntimeConfig.Builder

Configuration builder:

| Method | Default | Description |
|--------|---------|-------------|
| `setTargetFps(int)` | 60 | Target frame rate (30-120) |
| `setDebugEnabled(boolean)` | false | Debug mode |
| `setLogLevel(LogLevel)` | WARN | Log verbosity |
| `setCodeCacheDir(String)` | cacheDir | Compiled code directory |

## ProGuard

The library includes ProGuard rules. If you need to add custom rules:

```proguard
# Keep public API
-keep public class com.migo.runtime.** { public *; }

# Keep callback interfaces
-keep interface com.migo.runtime.callback.** { *; }
```

## Requirements

- **Minimum SDK**: 21 (Android 5.0 Lollipop)
- **Target SDK**: 34 (Android 14)
- **Supported ABIs**: arm64-v8a, x86_64

## License

See the project root for license information.
