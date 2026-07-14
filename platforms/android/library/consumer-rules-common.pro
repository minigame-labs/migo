# Public SDK ABI and JNI class names must survive the consuming app's R8 pass.
-keep,allowoptimization public class com.migo.runtime.* { public protected *; }
-keep,allowoptimization public interface com.migo.runtime.* { public protected *; }
-keep,allowoptimization public enum com.migo.runtime.* { public protected *; }
-keep,allowoptimization public class com.migo.runtime.callback.** { public protected *; }
-keep,allowoptimization public interface com.migo.runtime.callback.** { public protected *; }
-keep,allowoptimization public enum com.migo.runtime.callback.** { public protected *; }
-keepnames class com.migo.runtime.internal.NativeBridge
-keepnames class com.migo.runtime.internal.NativeExports
-keepattributes *Annotation*,Signature,InnerClasses,EnclosingMethod
