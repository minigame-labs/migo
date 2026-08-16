# Windows

Windows 平台已发布：`migo.dll`（x86_64、arm64）连同 `migo.lib` 导入库、公开头文件、
CMake 包，以及它按名加载的 `rusty_v8.dll` 与 ANGLE 运行时 DLL（`libEGL.dll`、
`libGLESv2.dll`、`d3dcompiler_47.dll`）。两个架构都由 `release.yml` 在原生 Windows
runner 上构建——x86_64 用 `release-windows`（`windows-latest`），arm64 用
`release-windows-arm64`（`windows-11-arm`）——不做交叉编译，各自跑在自己架构的
runner 上。C ABI 挂载的是 Win32 子 `HWND`（`engine/crates/capi/src/platform/windows.rs`），
`migo_query_capabilities` 会报告 `MIGO_PLATFORM_WIN32_HWND` 为可用的挂载类型。

从源码构建见 [`BUILD.md`](../../BUILD.md) 的 "5. Windows SDK" 一节；
契约门禁是 `scripts/test-windows-sdk-contract.sh`。

本阶段仍不包含：打包体验之外的 WinUI 集成、`EglProvider` 实现、更完整的
Win32 host kit（目前只到 C ABI 层，没有 Qt/X11 host-kit 那样的现成宿主封装）。

## `spike/` 下的脚本 —— 本项目自己的 WSL 开发机工作流

这些脚本不是"探测阶段的遗留物"，是这台开发机（源码在 WSL、工具链在 Windows）
现在仍在用的构建/验证工具，`BUILD.md` 的 "Windows SDK" 一节也在引用它们
（`sync-worktree.sh` 是 `scripts/build-windows-sdk.sh` 的 WSL 前置步骤）。

源码在 WSL，构建在 Windows。`cargo check` 无法在 `\\wsl.localhost\...` 上工作
（六分钟零输出且不创建 target 目录），因此 `sync-worktree.sh` 把当前 HEAD 复制
到 Windows 本地盘，`probe-layer.sh` 在那里驱动一次 `cargo check`。

    bash platforms/windows/spike/sync-worktree.sh
    bash platforms/windows/spike/probe-layer.sh migo-capi-abi

两者都从 WSL 侧运行。`probe-layer.sh` 会先比对两侧 HEAD，不一致直接以 91 退出——
探测一棵陈旧的树并报告"通过"，比报告失败有害得多。

## 工具链前置条件

`engine/rust-toolchain.toml` 把工具链钉在 **1.95.0**，并声明两个 **Android** target。
Windows 上那两份 std 是死重量，但不装它们钉定的工具链就解析不了，所以首次准备
Windows 侧环境需要：

    rustup toolchain install 1.95.0-x86_64-pc-windows-msvc --component rustfmt --component clippy
    rustup target add --toolchain 1.95.0-x86_64-pc-windows-msvc aarch64-linux-android x86_64-linux-android

用 `+stable` 绕过钉定版本是错的：那条钉子是可复现构建链的一部分。

若 rustup 的下载中途被打断，它会留下一个半装的工具链目录，之后每次调用都以
`os error 145（目录不是空的）`失败而不自愈。修法是删掉
`%USERPROFILE%\.rustup\toolchains\<那个工具链>` 与 `tmp/`、`downloads/` 后重装。
