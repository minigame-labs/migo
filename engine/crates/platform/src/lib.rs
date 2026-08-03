#[cfg(target_os = "android")]
pub mod android;
#[cfg(any(target_os = "android", test))]
mod android_permission_gate;

// Desktop platforms only: Android measures its own window through the JVM.
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "windows"
))]
pub mod host_window;

#[cfg(any(target_os = "android", test))]
mod host_owners;

#[path = "android/jni/profile_contract.rs"]
pub(crate) mod jni_profile_contract;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub mod linux;

// OpenHarmony reports target_os = "linux" with target_env = "ohos", so it must
// be selected on the env rather than the OS -- the desktop Linux module above
// excludes it for the same reason.
#[cfg(all(target_os = "linux", target_env = "ohos"))]
pub mod ohos;

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(all(feature = "profile-full", feature = "profile-slim"))]
compile_error!("profile-full and profile-slim are mutually exclusive");
#[cfg(all(feature = "worker-snapshot", not(feature = "profile-full")))]
compile_error!("worker-snapshot requires profile-full");
#[cfg(all(
    feature = "profile-slim",
    any(
        feature = "api-sensors",
        feature = "api-media",
        feature = "api-connectivity",
        feature = "api-commerce",
        feature = "api-system"
    )
))]
compile_error!("profile-slim is exact and cannot be combined with optional API groups");
