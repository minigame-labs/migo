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

# Keep NativeExports - methods are called from native code via cached method IDs
# WARNING: Do not rename these methods!
-keep class com.migo.runtime.internal.NativeExports {
    static java.lang.String getCacheDirPath();
    static void openSystemBluetoothSetting(int);
    static byte[] getWindowInfoBytes(int);
    static byte[] getSystemSettingInfoBytes();
    static java.lang.String getDeviceInfoJson();
    static java.lang.String getAppAuthorizationSettingJson();
}

# ============================================================================
# RuntimeConfig (JNI Field Access)
# ============================================================================

# Keep RuntimeConfig - fields are accessed via JNI reflection
-keep class com.migo.runtime.RuntimeConfig {
    <fields>;
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
-keep public class com.migo.runtime.MigoRuntime$BuildInfo { public *; }
-keep public class com.migo.runtime.MigoRuntime$Result { public *; }
-keep public class com.migo.runtime.GameSession { public *; }
-keep public class com.migo.runtime.ErrorCode { public *; }
-keep public class com.migo.runtime.RuntimeException { public *; }
-keep public class com.migo.runtime.RuntimeConfig$Builder { public *; }

# Keep callback interface
-keep interface com.migo.runtime.callback.GameSessionListener { *; }
