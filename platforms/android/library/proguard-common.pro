# Public SDK ABI. A single '*' is intentionally limited to the root package;
# internal implementation classes remain shrinkable and obfuscatable.
-keep,allowoptimization public class com.migo.runtime.* { public protected *; }
-keep,allowoptimization public interface com.migo.runtime.* { public protected *; }
-keep,allowoptimization public enum com.migo.runtime.* { public protected *; }
-keep,allowoptimization public class com.migo.runtime.callback.** { public protected *; }
-keep,allowoptimization public interface com.migo.runtime.callback.** { public protected *; }
-keep,allowoptimization public enum com.migo.runtime.callback.** { public protected *; }

# Rust resolves these class names with FindClass; members are profile-specific.
-keepnames class com.migo.runtime.internal.NativeBridge
-keepnames class com.migo.runtime.internal.NativeExports

-keepattributes SourceFile,LineNumberTable
-keepattributes *Annotation*,Signature,InnerClasses,EnclosingMethod
-dontwarn com.migo.runtime.R
-dontwarn com.migo.runtime.R$*
