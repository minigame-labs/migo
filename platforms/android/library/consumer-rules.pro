# ============================================================================
# Migo Runtime SDK - Consumer ProGuard Rules
# ============================================================================
# These rules are automatically applied to apps that depend on this library.
# They ensure that critical classes are not obfuscated or removed.

# ============================================================================
# Keep EVERYTHING in the SDK package tree
# ============================================================================
# The SDK is small (~50 classes) and every class is reachable via JNI,
# callbacks, or public API. Fine-grained rules cause hard-to-debug
# ClassNotFoundException in release builds when new classes/interfaces
# are added. A single broad rule is safer and simpler.

-keep class com.migo.runtime.** { *; }
-keep interface com.migo.runtime.** { *; }
-keep enum com.migo.runtime.** { *; }

# Keep all classes with native methods (JNI requirement)
-keepclasseswithmembers class * {
    native <methods>;
}
