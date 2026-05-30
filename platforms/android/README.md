# Migo Runtime SDK for Android

轻量级、高性能的 JavaScript 游戏运行时 Android SDK。

## 特性

- 🚀 **高性能** - 原生 Rust 引擎 + OpenGL ES 渲染
- 📱 **API 26+** - 支持 Android 8.0 及以上版本
- 🔧 **零依赖** - 无需 AndroidX、Kotlin 或其他第三方库
- 🎮 **游戏就绪** - 支持 Canvas 2D 和 WebGL
- 🔒 **沙箱隔离** - 每个游戏独立的文件系统空间

## 安装

### Gradle

将 AAR 添加到项目：

```groovy
dependencies {
    implementation files('libs/migo-release.aar')
}
```

或使用 Maven 仓库：

```groovy
dependencies {
    implementation 'com.migo:runtime:1.0.0'
}
```

## 快速开始

### 基础用法

```java
import com.migo.runtime.MigoRuntime;
import com.migo.runtime.GameSession;
import com.migo.runtime.RuntimeConfig;

public class GameActivity extends Activity {
    private GameSession session;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        
        // 1. 创建 SurfaceView 用于渲染
        SurfaceView surfaceView = new SurfaceView(this);
        setContentView(surfaceView);

        // 2. 配置运行时
        RuntimeConfig config = new RuntimeConfig.Builder(this)
            .setDebugEnabled(BuildConfig.DEBUG)
            .setLogLevel(RuntimeConfig.LogLevel.DEBUG)
            .setTargetFps(60)
            .build();

        // 3. 设置 Surface 回调
        surfaceView.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder holder) {
                // 4. 创建游戏会话（每个游戏需要唯一的 gameId）
                session = MigoRuntime.getInstance()
                    .createSession(GameActivity.this, holder.getSurface(), config, "my-game-id");
                
                // 5. 启动游戏（从隔离的 code 目录加载）
                // 游戏代码应放在: session.getPaths().getCodeDir()
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

        // 6. 处理触摸事件
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

### 使用事件回调

```java
// 设置统一事件监听器
session.setListener(new GameSessionListener() {
    @Override
    public void onGameReady() {
        Log.i(TAG, "游戏已就绪!");
        hideLoadingScreen();
    }

    @Override
    public void onGameExit(int exitCode) {
        Log.i(TAG, "游戏退出，代码: " + exitCode);
        finish();
    }

    @Override
    public void onError(int errorCode, String message, boolean recoverable) {
        Log.e(TAG, "错误 " + errorCode + ": " + message);
        if (!recoverable) {
            showErrorDialog(message);
        }
    }

    // 可选: 覆写 onLoadingProgress / onPaused / onResumed 等
});
```

### 安全的会话创建（无异常版本）

```java
MigoRuntime.Result<GameSession> result = MigoRuntime.getInstance()
    .createSessionSafe(activity, surface, config, "my-game-id");

if (result.isSuccess()) {
    session = result.getValue();
    session.startGame("game.js");  // 使用默认 code 目录
} else {
    int errorCode = result.getErrorCode();
    String message = result.getErrorMessage();
    Log.e(TAG, "创建会话失败: " + ErrorCode.getMessage(errorCode));
}
```

### 注册宿主回调 Handler（Auth / GameLog / Subpackage）

在最新 API 中，`GameSession` 支持三类宿主回调。建议在 `startGame()` 前注册。

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

- `setAuthHandler(AuthHandler)`：处理 `wx.login` / `wx.checkSession` / `wx.getUserInfo` / `wx.getPhoneNumber`
- `setGameLogHandler(GameLogHandler)`：接收小游戏上报日志（JSON 字符串）
- `setSubpackageHandler(SubpackageHandler)`：处理 `loadSubpackage` / `preDownloadSubpackage` 的下载过程

## 文件系统隔离

每个游戏都有独立的沙箱化目录，通过 `GamePaths` 管理：

### 目录结构

```
/data/data/com.your.app/
├── files/migo/games/{gameId}/     # 持久化存储
│   ├── code/                      # 游戏代码（只读）
│   └── user_data/                 # 用户数据（读写）
│
└── cache/migo/games/{gameId}/     # 缓存存储（系统可清理）
    └── tmp/                       # 临时文件（会话结束清理）
```

### JavaScript 虚拟路径

在 JS 代码中，所有路径都是虚拟的，防止游戏访问其他游戏或系统文件：

```javascript
import { env } from 'migo:env';

// 虚拟路径常量
env.USER_DATA_PATH  // "/user"  - 用户数据（持久化）
env.CACHE_PATH      // "/cache" - 缓存文件
env.CODE_PATH       // "/code"  - 游戏代码（只读）
env.TEMP_PATH       // "/tmp"   - 临时文件

// 使用示例
const fs = require('migo:fs');

// 保存游戏进度
fs.writeFileSync('/user/save.json', JSON.stringify(saveData));

// 读取配置
const config = JSON.parse(fs.readFileSync('/code/config.json'));

// 写入缓存
fs.writeFileSync('/cache/downloaded_asset.png', imageData);
```

### Java 路径管理

```java
GameSession session = runtime.createSession(activity, surface, config, "my-game-id");
GamePaths paths = session.getPaths();

// 获取真实路径
File codeDir = paths.getCodeDir();       // 放置游戏代码
File userDataDir = paths.getUserDataDir(); // 用户数据
File cacheDir = paths.getCacheDir();     // 缓存文件
File tempDir = paths.getTempDir();       // 临时文件

// 部署游戏代码
unzipGamePackage(downloadedZip, codeDir);

// 启动游戏
session.startGame("game.js");

// 获取存储使用情况
long userDataSize = paths.getUserDataSize();  // 用户数据大小（字节）
long cacheSize = paths.getCacheSize();         // 缓存大小（字节）

// 清理缓存
paths.cleanupTemp();  // 清理临时文件

// 完全删除游戏数据
paths.deleteAll();    // 卸载游戏
```

### gameId 要求

- 长度: 1-64 字符
- 允许字符: `a-z`, `A-Z`, `0-9`, `_`, `-`
- 示例: `puzzle-game`, `com_example_game`, `game123`

## 配置选项

```java
RuntimeConfig config = new RuntimeConfig.Builder(context)
    // 性能
    .setTargetFps(60)              // 30-120, 默认: 60
    
    // 调试
    .setDebugEnabled(true)         // 启用调试功能
    .setLogLevel(LogLevel.DEBUG)   // TRACE, DEBUG, INFO, WARN, ERROR, OFF
    
    // 目录
    .setCodeCacheDir(codeCacheDir) // 编译代码缓存目录
    
    .build();
```

## 错误处理

SDK 使用结构化的错误码：

```java
// 错误码
ErrorCode.SUCCESS               //  0: 成功
ErrorCode.ERR_INIT_FAILED       // -1000: 初始化失败
ErrorCode.ERR_INVALID_SURFACE   // -1001: 无效 Surface
ErrorCode.ERR_INVALID_CONFIG    // -1002: 无效配置
ErrorCode.ERR_NATIVE_LOAD_FAILED// -1003: 原生库加载失败
ErrorCode.ERR_SESSION_DESTROYED // -2000: Session 已销毁
ErrorCode.ERR_CODE_DIR_NOT_FOUND// -2002: 代码目录不存在
ErrorCode.ERR_ENTRY_NOT_FOUND   // -2003: 入口文件不存在
ErrorCode.ERR_JS_EXECUTION      // -2004: JS 执行错误
ErrorCode.ERR_INVALID_ACTIVITY  // -5004: 无效 Activity

// 获取可读的错误消息
String message = ErrorCode.getMessage(code);
```

## API 参考

### MigoRuntime

主入口（单例）：

| 方法 | 描述 |
|------|------|
| `getInstance()` | 获取单例实例 |
| `createSession(Activity, Surface, RuntimeConfig, String gameId)` | 创建游戏会话（Activity 绑定） |
| `createSession(Context, Surface, RuntimeConfig, String gameId)` | 创建游戏会话（无 Activity 绑定） |
| `createSessionSafe(Activity, Surface, RuntimeConfig, String gameId)` | 无异常版本 |
| `getVersion()` | 获取 SDK 版本 |
| `getNativeVersion()` | 获取原生引擎版本 |
| `isNativeLoaded()` | 检查原生库是否加载 |
| `isDeviceSupported()` | 检查设备兼容性 |
| `getActiveSessionCount()` | 获取当前活跃会话数 |
| `getMinSdkVersion()` | 获取最低支持 API 等级 |

### GameSession

游戏会话（实现 `Closeable`）：

| 方法 | 描述 |
|------|------|
| `startGame(String entryPoint)` | 启动游戏（从 `paths.getCodeDir()`） |
| `startGameSafe(String entryPoint)` | 无异常版本 |
| `pause()` | 暂停游戏 |
| `resume()` | 恢复游戏 |
| `restart()` | 重启游戏 |
| `updateSurface(Surface)` | 更新渲染表面 |
| `dispatchTouchEvent(MotionEvent)` | 处理触摸输入 |
| `dispatchMemoryWarning(int)` | 转发内存告警 |
| `setListener(GameSessionListener)` | 注册统一会话事件回调 |
| `setAuthHandler(AuthHandler)` | 注册鉴权回调 |
| `setGameLogHandler(GameLogHandler)` | 注册游戏日志回调 |
| `setSubpackageHandler(SubpackageHandler)` | 注册分包下载回调 |
| `close()` / `destroy()` | 释放资源 |
| `isValid()` | 检查会话是否有效 |
| `isGameStarted()` | 检查游戏是否已启动 |

### RuntimeConfig.Builder

配置构建器：

| 方法 | 默认值 | 描述 |
|------|--------|------|
| `setTargetFps(int)` | 60 | 目标帧率 (30-120) |
| `setDebugEnabled(boolean)` | false | 调试模式 |
| `setLogLevel(LogLevel)` | WARN | 日志级别 |
| `setCodeCacheDir(String)` | cacheDir | 代码缓存目录 |

## ProGuard

库已包含 ProGuard 规则。如需添加自定义规则：

```proguard
# 保留公共 API
-keep public class com.migo.runtime.** { public *; }

# 保留回调接口
-keep interface com.migo.runtime.callback.** { *; }
```

## 系统要求

- **最低 SDK**: 26 (Android 8.0 Oreo)
- **目标 SDK**: 34 (Android 14)
- **支持的 ABI**: arm64-v8a, x86_64

## 许可证

请查看项目根目录的许可证信息。
