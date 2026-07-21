// Skia is built with `skia_use_egl=true` / `skia_use_gl=true` and references
// EGL entry points (e.g. `eglGetProcAddress` in GrGLMakeEGLInterface). On
// Android the NDK sysroot provides libEGL implicitly; on a glibc host the
// player executable must link it explicitly. The unversioned `libEGL.so` link
// name lives under the dev-setup-skia.sh symlink dir (`~/.local/lib`) on
// minimal Ubuntu/WSL systems that ship only `libEGL.so.1`.
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "linux" {
        if let Ok(home) = std::env::var("HOME") {
            println!("cargo:rustc-link-search=native={home}/.local/lib");
        }
        println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
        println!("cargo:rustc-link-lib=dylib=EGL");
        // Skia (skia_use_gl=true) references gl* entry points directly; desktop
        // libGL exports them. The unversioned libGL.so symlink comes from
        // scripts/dev-setup-skia.sh.
        println!("cargo:rustc-link-lib=dylib=GL");
    }
}
