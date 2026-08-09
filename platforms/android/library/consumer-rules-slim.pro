# Core JNI roots for the consuming app's R8 pass. Keep this list in lockstep
# with proguard-slim.pro and jni/profile_contract.rs.
-keepclassmembers,allowoptimization class com.migo.runtime.internal.NativeBridge {
    public static native *** version(...);
    public static native *** getMinApiLevel(...);
    public static native *** initIcuData(...);
    public static native *** init(...);
    public static native *** shutdown(...);
    public static native *** onShow(...);
    public static native *** onHide(...);
    public static native *** onRestart(...);
    public static native *** updateSurface(...);
    public static native *** onSurfaceDestroyed(...);
    public static native *** onTouchEvent(...);
    public static native *** modMain(...);
    public static native *** executeScript(...);
    public static native *** onVsync(...);
    public static native *** setDisplayRefreshRate(...);
    public static native *** getDebugStats(...);
    public static native *** getConsoleLogs(...);
    public static native *** nativeAhbPointerFromHardwareBuffer(...);
    public static native *** onKeyboardInput(...);
    public static native *** onKeyboardConfirm(...);
    public static native *** onKeyboardComplete(...);
    public static native *** onKeyboardHeightChange(...);
    public static native *** onMemoryWarning(...);
    public static native *** onThermalStatusChanged(...);
    public static native *** onSubpackageProgress(...);
    public static native *** onSubpackageResult(...);
}
-keepclassmembers,allowoptimization class com.migo.runtime.internal.NativeExports {
    public static *** beginRuntimeRestart(...);
    public static *** completeRuntimeRestart(...);
    public static *** unzipFile(...);
    public static *** encodeGbk(...);
    public static *** decodeGbk(...);
    public static *** decodeImageAhb(...);
    public static *** decodeImageRgba(...);
    public static *** keyboardShow(...);
    public static *** keyboardHide(...);
    public static *** keyboardUpdate(...);
    public static *** subpackageDownload(...);
    public static *** onGameReady(...);
    public static *** requestVsync(...);
    public static *** onError(...);
    public static *** onExit(...);
    public static *** onHostMessage(...);
}
