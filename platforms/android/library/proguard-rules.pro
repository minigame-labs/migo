# ============================================================================
# Migo Runtime SDK - ProGuard Rules
# ============================================================================
# These rules are applied when building the library in release mode.
# For rules that should be inherited by consuming apps, see consumer-rules.pro

# ============================================================================
# JNI / Native Methods
# ============================================================================

# Keep all native method declarations in NativeBridge
# These are registered dynamically via JNI_OnLoad and must not be renamed
-keep class com.migo.runtime.internal.NativeBridge {
    *;
}

# Keep NativeExports - called from native Rust code via cached method IDs
-keep class com.migo.runtime.internal.NativeExports {
    *;
}

# ============================================================================
# Public API Classes
# ============================================================================

# Keep all public API classes and their public/protected members
-keep public class com.migo.runtime.MigoRuntime {
    public *;
}

-keep public class com.migo.runtime.MigoRuntime$BuildInfo {
    public *;
}

-keep public class com.migo.runtime.MigoRuntime$Result {
    public *;
}

-keep public class com.migo.runtime.GameSession {
    public *;
}

-keep public class com.migo.runtime.ErrorCode {
    public static final int *;
    public static java.lang.String getMessage(int);
    public static boolean isSuccess(int);
    public static boolean isInitError(int);
    public static boolean isRuntimeError(int);
    public static boolean isNetworkError(int);
}

-keep public class com.migo.runtime.RuntimeException {
    public *;
}

# Keep RuntimeConfig with all fields - accessed via JNI reflection
-keep class com.migo.runtime.RuntimeConfig {
    <fields>;
    <methods>;
}

-keep class com.migo.runtime.RuntimeConfig$LogLevel {
    *;
}

-keep class com.migo.runtime.RuntimeConfig$RenderBackend {
    *;
}

-keep class com.migo.runtime.RuntimeConfig$Builder {
    public *;
}

# ============================================================================
# Callback Interfaces
# ============================================================================

-keep interface com.migo.runtime.callback.OnGameEventListener {
    *;
}

-keep interface com.migo.runtime.callback.OnErrorListener {
    *;
}

-keep interface com.migo.runtime.callback.OnLifecycleListener {
    *;
}

# ============================================================================
# Internal Classes (Required by JNI)
# ============================================================================

# Keep NativeMethods - facade for JNI calls
-keep class com.migo.runtime.internal.NativeMethods {
    public static *;
}

# Keep TouchEventHandler - used for touch event serialization
-keep class com.migo.runtime.internal.TouchEventHandler {
    public static final int *;
    public void dispatch(int, android.view.MotionEvent);
}

# Keep runtime classes - accessed from native code
-keep class com.migo.runtime.internal.RuntimeRegistry {
    public static *;
}

-keep class com.migo.runtime.internal.RuntimeContext {
    <fields>;
    public *;
}

# Keep AppContext - accessed from NativeExports
-keep class com.migo.runtime.internal.AppContext {
    public static *;
}

# ============================================================================
# Debugging
# ============================================================================

# Keep line numbers for better crash reports
-keepattributes SourceFile,LineNumberTable

# Keep annotations for reflection and lint
-keepattributes *Annotation*,Signature,InnerClasses,EnclosingMethod

# ============================================================================
# Warnings
# ============================================================================

# Suppress warnings for generated R class references
-dontwarn com.migo.runtime.R
-dontwarn com.migo.runtime.R$*
