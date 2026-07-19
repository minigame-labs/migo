//! Declares the link-time dependencies that this crate's graphics backend
//! creates, so consumers do not have to rediscover them.
//!
//! Skia is compiled with its GL backend enabled, which means its objects
//! reference `gl*` entry points directly. That requirement is created here, so
//! it is declared here rather than in each binary that happens to link the
//! engine. Before this existed, the player and the C example each carried their
//! own copy of the declaration and a packaged consumer had no way to learn of
//! it except by hitting undefined symbols at link time.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // Android resolves GL through its own vendor libraries and is linked by the
    // NDK toolchain; only the desktop GNU targets need this here.
    if target_os == "linux" && target_env == "gnu" {
        println!("cargo:rustc-link-lib=GL");
    }
}
