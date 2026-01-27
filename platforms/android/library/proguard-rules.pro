# ============================================================================
# MiniGame Host Library - ProGuard Rules
# ============================================================================
# These rules are applied when building the library in release mode.
# For rules that should be inherited by consuming apps, see consumer-rules.pro

# ============================================================================
# JNI / Native Methods
# ============================================================================

# Keep all native method declarations in HostJNI
# These are registered dynamically via JNI_OnLoad and must not be renamed
-keep class com.migo.host.internal.jni.HostJNI {
    native <methods>;
    # Keep the static initializer that loads the native library
    static { *; }
}

# Keep NativeExports - called from native Rust code via cached method IDs
# Method signatures must match exactly what Rust expects
-keep class com.migo.host.internal.NativeExports {
    static void unzip(int, int, java.lang.String, java.lang.String);
    static java.lang.String getCacheDirPath();
    static void openSystemBluetoothSetting(int);
    static byte[] getWindowInfoBytes(int);
    static byte[] getSystemSettingInfoBytes();
    static java.lang.String getDeviceInfoJson();
    static java.lang.String getAppAuthorizationSettingJson();
}

# ============================================================================
# Public API Classes
# ============================================================================

# Keep all public API classes and their public/protected members
-keep public class com.migo.host.MiniGameSDK {
    public *;
}

-keep public class com.migo.host.MiniGameSDK$InitResult {
    public *;
}

-keep public class com.migo.host.HostHandle {
    public *;
}

-keep public class com.migo.host.HostCallback {
    *;
}

-keep public class com.migo.host.MiniGameError {
    public static final int *;
    public static java.lang.String getMessage(int);
    public static boolean isError(int);
}

# Keep InitOption with all fields - accessed via JNI reflection
-keep class com.migo.host.InitOption {
    <fields>;
    <methods>;
}

-keep class com.migo.host.InitOption$LogLevel {
    *;
}

-keep class com.migo.host.InitOption$Builder {
    public *;
}

# ============================================================================
# Internal Classes (Required by JNI)
# ============================================================================

# Keep HostNative - facade for JNI calls
-keep class com.migo.host.internal.jni.HostNative {
    public static *;
}

# Keep MotionEventFlattener - used for touch event serialization
-keep class com.migo.host.internal.jni.MotionEventFlattener {
    public static final int *;
    public static int flatten(android.view.MotionEvent, float, java.nio.ByteBuffer);
}

# Keep runtime classes - accessed from native code
-keep class com.migo.host.internal.runtime.HostRuntimeRegistry {
    public static *;
}

-keep class com.migo.host.internal.runtime.HostRuntime {
    <fields>;
}

# Keep AppContext - accessed from NativeExports
-keep class com.migo.host.internal.AppContext {
    public static *;
}

# Keep BluetoothSettingActivity - launched via Intent
-keep class com.migo.host.internal.bluetooth.BluetoothSettingActivity {
    <init>();
}

# ============================================================================
# Data Classes (Serialization)
# ============================================================================

# Keep window/device info classes - serialized to bytes/JSON
-keep class com.migo.host.internal.window.WindowInfo { *; }
-keep class com.migo.host.internal.window.SafeArea { *; }

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
-dontwarn com.migo.host.R
-dontwarn com.migo.host.R$*
