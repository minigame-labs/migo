# Windows

Windows 平台支持处于**可行性调查（spike）**阶段，尚无可用产物。

构建与探测脚本在 `platforms/windows/spike/`，从 WSL 侧驱动 Windows 工具链。

## spike/ 下的脚本

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

## 本阶段不包含

打包、WinUI 集成、`EglProvider` 实现、`MigoWin32HwndDescriptor` 实现。
形态待 spike 结论倒逼。
