# Consumer ProGuard rules for MiniGame Host Library
# These rules will be applied to projects that use this library

# Keep native methods

-keepclasseswithmembers class * {
    native <methods>;
}

-keep class com.minigame.host.internal.jni.HostJNI { *; }

-keep class com.minigame.host.InitOption {
    *;
}