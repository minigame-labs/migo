# ============================================================================
# MiniGame Host Library - Consumer ProGuard Rules
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

# Keep HostJNI - contains native method declarations
-keep class com.migo.host.internal.jni.HostJNI {
    native <methods>;
    static { *; }
}

# ============================================================================
# JNI Callbacks (Rust -> Java)
# ============================================================================

# Keep NativeExports - methods are called from native code via cached method IDs
# WARNING: Do not rename these methods!
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
# InitOption (JNI Field Access)
# ============================================================================

# Keep InitOption - fields are accessed via JNI reflection
-keep class com.migo.host.InitOption {
    <fields>;
}

# Keep LogLevel enum - ordinal() is called from native code
-keep class com.migo.host.InitOption$LogLevel {
    *;
}

# ============================================================================
# Public API
# ============================================================================

# Keep public API classes (apps may use reflection)
-keep public class com.migo.host.MiniGameSDK { public *; }
-keep public class com.migo.host.MiniGameSDK$InitResult { public *; }
-keep public class com.migo.host.HostHandle { public *; }
-keep public class com.migo.host.HostCallback { *; }
-keep public class com.migo.host.MiniGameError { public *; }
-keep public class com.migo.host.InitOption$Builder { public *; }
