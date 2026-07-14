# Full registers every declared NativeBridge native and every public static
# NativeExports entrypoint. Optimization is allowed; shrinking/renaming is not.
-keepclassmembers,allowoptimization class com.migo.runtime.internal.NativeBridge {
    native <methods>;
}
-keepclassmembers,allowoptimization class com.migo.runtime.internal.NativeExports {
    public static *** *(...);
}
