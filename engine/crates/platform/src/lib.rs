#[cfg(target_os = "android")]
pub mod android;

#[path = "android/jni/profile_contract.rs"]
pub(crate) mod jni_profile_contract;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub mod linux;
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
