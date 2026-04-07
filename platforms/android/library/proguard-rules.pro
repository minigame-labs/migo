# ============================================================================
# Migo Runtime SDK - ProGuard Rules
# ============================================================================
# These rules are applied when building the library in release mode.
# For rules that should be inherited by consuming apps, see consumer-rules.pro

# ============================================================================
# Keep EVERYTHING in the SDK package tree
# ============================================================================
# The SDK is small (~50 classes) and every class is reachable via JNI,
# callbacks, or public API. Fine-grained rules are fragile — new classes
# (e.g., inner interfaces, callback types) get stripped silently.

-keep class com.migo.runtime.** { *; }
-keep interface com.migo.runtime.** { *; }
-keep enum com.migo.runtime.** { *; }

# Keep all classes with native methods (JNI requirement)
-keepclasseswithmembers class * {
    native <methods>;
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
