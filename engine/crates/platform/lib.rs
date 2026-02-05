#[cfg(target_os = "android")]
pub mod android;

#[cfg(not(target_os = "android"))]
pub mod desktop;
