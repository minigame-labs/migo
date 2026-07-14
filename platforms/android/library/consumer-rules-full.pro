# The consuming app performs a second R8 pass, so repeat the full JNI roots.
-keepclassmembers,allowoptimization class com.migo.runtime.internal.NativeBridge {
    native <methods>;
}
-keepclassmembers,allowoptimization class com.migo.runtime.internal.NativeExports {
    public static *** *(...);
}
