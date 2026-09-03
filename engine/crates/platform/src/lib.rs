#[cfg(target_os = "android")]
pub mod android;
#[cfg(any(target_os = "android", test))]
mod android_permission_gate;

#[cfg(any(target_os = "android", test))]
#[path = "android/frame_rate.rs"]
mod android_frame_rate;

#[cfg(any(target_os = "android", test))]
mod host_owners;

#[cfg(any(target_os = "android", test))]
mod jni_method_id;
// Compiled on every host, not only Android: the profile contract's own tests
// are what keep the JNI surface and the profile features in step, and they have
// to run somewhere with a CI runner. The external-frame product is the
// exception -- it is the Apple lane, it registers no JNI methods, and compiling
// a contract with no surface to describe would be dead code that reads as
// coverage.
#[cfg(not(feature = "external-frames"))]
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
// The other half, so "which profile is this" is a total question rather than a
// mostly-true one. `active_methods` selects the JNI surface with a chain of
// `#[cfg(feature)]` attributes and the profile rule states it declaratively;
// tying the two together needs a build to have exactly one answer, not zero.
// Every dependent already forwards a profile through its own `default`, so this
// only rejects a bare `--no-default-features`, which is not a product.
#[cfg(not(any(
    feature = "profile-full",
    feature = "profile-slim",
    feature = "external-frames"
)))]
compile_error!("exactly one of profile-full, profile-slim and external-frames must be enabled");
// The external-frame product has no embedded engine, so it cannot also carry a
// profile that describes one. Rejected here rather than resolved by precedence,
// for the same reason `migo-core` rejects the pair: failing here says which
// flag to drop, and failing in a dependency-closure gate says only that V8 is
// present.
#[cfg(all(
    feature = "external-frames",
    any(feature = "profile-full", feature = "profile-slim")
))]
compile_error!(
    "external-frames cannot be combined with an embedded profile: the lane exists to \
     prove no JavaScript engine is linked"
);
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
