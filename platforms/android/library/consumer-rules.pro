# ============================================================================
# Migo Runtime SDK - Consumer ProGuard Rules
# ============================================================================
# These rules are automatically applied to apps that depend on this library.
# They ensure that critical classes are not obfuscated or removed.

# ============================================================================
# Native Methods
# ============================================================================

# Keep all classes with native methods (JNI requirement)
-keepclasseswithmembers class * {
    native <methods>;
}

# Keep NativeBridge - contains native method declarations
-keep class com.migo.runtime.internal.NativeBridge {
    native <methods>;
    static { *; }
}

# ============================================================================
# JNI Callbacks (Rust -> Java)
# ============================================================================

# Keep NativeExports - ALL methods are called from native code via cached method IDs
# WARNING: Do not rename or remove ANY methods!
-keep class com.migo.runtime.internal.NativeExports {
    *;
}

# Keep internal classes accessed from native code or NativeExports
-keep class com.migo.runtime.internal.NativeMethods {
    *;
}

-keep class com.migo.runtime.internal.RuntimeRegistry {
    *;
}

-keep class com.migo.runtime.internal.RuntimeContext {
    *;
}

-keep class com.migo.runtime.internal.AppContext {
    *;
}

# ============================================================================
# RuntimeConfig (JNI Field Access)
# ============================================================================

# Keep RuntimeConfig - fields and methods are accessed via JNI reflection
-keep class com.migo.runtime.RuntimeConfig {
    <fields>;
    <methods>;
}

# Keep DebugOverlayView - may be used by host apps
-keep public class com.migo.runtime.DebugOverlayView {
    public *;
}

# Keep GamePaths - used by host apps
-keep public class com.migo.runtime.GamePaths {
    public *;
}

# Keep LogLevel enum - ordinal() is called from native code
-keep class com.migo.runtime.RuntimeConfig$LogLevel {
    *;
}

# Keep RenderBackend enum
-keep class com.migo.runtime.RuntimeConfig$RenderBackend {
    *;
}

# ============================================================================
# Public API
# ============================================================================

# Keep public API classes (apps may use reflection)
-keep public class com.migo.runtime.MigoRuntime { public *; }
-keep public class com.migo.runtime.BuildInfo { public *; }
-keep public class com.migo.runtime.MigoRuntime$Result { public *; }
-keep public class com.migo.runtime.GameSession { public *; }
-keep public class com.migo.runtime.ErrorCode { public *; }
-keep public class com.migo.runtime.RuntimeException { public *; }
-keep public class com.migo.runtime.MigoException { *; }
-keep public class com.migo.runtime.RuntimeConfig$Builder { public *; }

# Keep callback interface
-keep interface com.migo.runtime.callback.GameSessionListener { *; }

# Keep new public lifecycle API
-keep public class com.migo.runtime.SessionState { *; }
-keep public interface com.migo.runtime.OnStateChangeListener { *; }

# Keep new public API classes
-keep public class com.migo.runtime.MigoGameView { public *; }
-keep public class com.migo.runtime.MigoGameActivity { public *; protected *; }

# Keep ResultProxyActivity - started via Intent
-keep class com.migo.runtime.internal.ResultProxyActivity { *; }

# Keep PerformanceSnapshot - public API for performance monitoring
-keep public class com.migo.runtime.PerformanceSnapshot { *; }
