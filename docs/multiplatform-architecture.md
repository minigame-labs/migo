# Migo 多平台架构设计

> 状态：规范性设计与当前实现基线
>
> 校订日期：2026-07-22
>
> 适用范围：Migo engine、平台 Host Kit、公开 C ABI、构建与发布工件
>
> 决策原则：正确性与平台约束优先；在这些约束内以端到端性能为第一优化目标

本文不是“把 Android 实现套到所有平台”的移植清单，也不是未来能力宣传页。它同时回答三个问题：

1. 每个平台的最佳宿主、窗口、帧调度、图形与 JavaScript runtime 方案是什么；
2. 当前仓库已经实现了什么、还缺什么；
3. 怎样改造仓库而不为了表面统一牺牲正确性、性能或平台生态集成。

文中的状态只有三种：

- **已实现**：当前仓库存在实现和自动化契约；仍可能是未冻结的 candidate API。
- **候选**：有可复现实验或编译验证，但未完成生产链路。
- **目标**：设计方向，不能据此宣称平台支持。

## 0. 结论

1. **Migo SDK 不拥有顶层窗口。** App 或 UI toolkit 拥有窗口、View 树、事件循环和系统生命周期；Migo 接收宿主提供的原生 Surface，并在其内部运行内容。独立播放器是单独的工具，不是 SDK 的窗口依赖。
2. **统一语义和边界，不统一平台实现。** Session、Surface 生命周期、输入、错误、artifact identity 和 JS API 行为统一；Presenter、frame clock、graphics backend、audio、IME、权限和打包方式按平台实现。
3. **热路径不经过通用跨平台抽象税。** 平台/backend 在创建或 attach 时选择并固定；draw、command decode、resource lookup、submit 和 present 不做每帧服务发现、序列化、堆分配或通用 actor 跳转。
4. **Android 最低 API 固定为 26。** 更高 API 的能力通过运行时探测启用，不能反向抬高基础 artifact 的最低版本。
5. **Linux GNU 首个合同是 x86_64、glibc 2.31、GLIBCXX 3.4.28、x86-64-v1。** 这是构建 sysroot 和符号审计共同保证的发布合同，不是开发机版本。
6. **Windows 首个目标是 Windows 10 1809/build 17763、x86_64、ANGLE D3D11。** 当前只有 ABI/MSVC 和部分 crate 编译 spike，尚无生产 presenter、真实 runtime/package 或真机出帧，因此不能标为已支持。
7. **macOS 使用 Metal family；iOS App Store 默认使用 WKWebView。** macOS 可用 ANGLE Metal 承接 WebGL/GLES 语义，但最终 Canvas2D/WebGL 是否共享 ANGLE 或使用原生 Skia Metal，必须由零拷贝互操作和整帧 benchmark 决定。iOS 不把 V8 方案伪装成通用 App Store backend。
8. **OpenHarmony 使用 XComponent/NativeWindow 与 EGL/GLES。** 系统 JSVM 从 API 11 起可作为独立实验 backend；它只有在 JS conformance、snapshot/启动、调试和性能全部达标后才可替代嵌入式 V8。
9. **每个 native-V8 slice 都要单独构建 V8。** OS、arch、ABI、工具链、链接模型、CPU baseline 或 GN 参数任一不同，就是不同 component；其 snapshot 也不得复用。
10. **每个平台的每个交付 artifact 必须自描述。** manifest 必须包含 arch、最低 OS/API 或 glibc floor、CPU baseline、runtime/V8 revision、snapshot policy 与参数、工具链、输入和交付文件哈希。
11. **最低版本测试只负责兼容性，最新系统测试只负责性能。** 两条 lane 必须运行同一 package hash，并分别出报告，不能用新系统 benchmark 证明旧系统可用，也不能用旧设备性能抬高支持下限。
12. **当前 repo 不需要 Actix。** 它不解决 runtime/render 的核心问题，并会给消息热路径带来不必要的调度、分配和生命周期复杂度。新依赖只能由明确的平台能力、可测收益和维护合同驱动。
13. **Desktop Host Kit 默认按源码构建。** toolkit adapter 与 App 使用同一 Qt/GTK/编译器 ABI 构建；正式分发预编译 Host Kit 时，它才成为必须写入 arch、OS/glibc、CPU 与依赖版本身份的独立 native artifact。

## 1. 硬约束与决策顺序

### 1.1 不可交换的约束

- 不允许 native window、GPU resource、callback user data 或 Session 发生 use-after-free。
- 不允许平台线程规则被跨平台封装隐藏；UI/main thread 要求必须保留。
- 不允许因异常、dispatcher 拒绝或 teardown race 破坏后续事件投递。
- 不允许 snapshot 与 V8 archive、external reference、feature set 或 JS/Rust bootstrap 输入不匹配。
- 不允许 artifact manifest 只描述“应该交付什么”而不校验 staging tree 中的真实字节。
- 不允许为兼容一个平台而让其他平台走低性能 fallback。
- 不允许把编译通过、header 中存在 descriptor 或一次 spike 等同于 production support。

### 1.2 决策优先级

遇到冲突时按以下顺序裁决：

1. 内存安全、数据完整性、内容安全和平台政策；
2. native resource 生命周期、线程模型和 ABI 正确性；
3. 端到端 latency、稳定 frame time、内存、功耗与包体；
4. 可诊断性、可复现构建和长期维护成本；
5. 源码复用率与 API 外观一致性。

“统一代码更多”不是独立目标。只有不改变上述排序时才复用实现。

### 1.3 性能定义

性能不是单一平均 FPS。每个平台至少同时观察：

- cold/warm start 的 p50、p95；
- frame time p50、p95、p99、jank 和 missed-vsync；
- input-to-photon latency；
- steady-state RSS、GPU memory、峰值内存；
- image decode/upload、shader/program warm-up；
- Surface 重建、前后台切换和 device/context loss 恢复时间；
- 持续负载下的功耗、温升和降频；
- artifact 下载体积、安装体积与符号/调试包体积。

优化必须比较完整链路。局部 microbenchmark 更快但引入跨 GPU device copy、额外合成层或更差 p99，不能成为默认方案。

## 2. 当前仓库事实

### 2.1 支持状态

| 平台 | 当前状态 | 已有能力 | 尚不能宣称的能力 |
|---|---|---|---|
| Android | 已实现，公开 ABI 仍是 candidate | API 26；JNI AAR；C ABI static SDK+CMake；`ANativeWindow`/EGL/GLES presenter；输入、IME、gamepad；**两个 ABI 的 V8 component manifest 已验证产出**；**六份 snapshot 齐备**（full/slim/worker × aarch64 真机 + x86_64 模拟器，x86_64 已端到端证明 restore） | 所有 ABI 的 release package 与最低/最新真机双门；全量多指真机验证；`-DANDROID_STL` 矩阵 |
| Linux GNU | 已实现，公开 ABI 仍是 candidate | x86_64；X11/Wayland host-owned Surface；EGL/GLES；`libmigo.so`/`.a`；pkg-config/CMake；外部 C consumer；toolkit-neutral `SurfaceHost` 与 Qt 6 Widgets/X11 的 Bound + Managed 两种所有权形态（含输入/焦点/IME/frame request） | 用新 v2 component manifest 重建并发布的正式 artifact；发行版/驱动矩阵与性能门；Qt Wayland；Qt Quick 与 GTK 4（同一 texture/fence 前置） |
| Windows | runtime 已跑通，图形未出帧 | C/C++ header 的 MSVC x64/x86 ABI lane；`engine/crates` 全部九个 crate 的 MSVC `cargo check`；**真实 V8 链接与启动**（`migo-runtime-v8` 424/426、`migo-io` 225/225 在 MSVC 上真跑）；**从源码构建的 Windows V8**（`scripts/build-v8-windows.sh`，`use_custom_libcxx=false` 让 Skia 与 V8 共用 MSVC STL，消除二者同链时的 `std::terminate` 重复定义）；`EglProvider` 可承载 ANGLE | HWND/SwapChainPanel presenter；ANGLE 出帧；输入/音频/完整性；DLL/NuGet；真机 |
| macOS | 目标 | 公开 typed descriptor 设计 | runtime、Metal presenter、V8 component、Host Kit、package、CI |
| iOS | 目标 | 明确采用 WebKit Host Kit 的产品边界 | WKWebView backend、bridge、conformance、package、CI |
| OpenHarmony | 目标 | 公开 typed descriptor 设计与官方 NativeWindow/JSVM 可行性依据 | XComponent presenter、runtime 决策、HAR/HAP 集成、component、真机 CI |

如果表格和代码冲突，以可执行 contract 和 package verifier 为准，并立即修正文档。

### 2.2 目录结构

当前结构的方向正确，继续沿用：

```text
migo/
├── adapter/                       # JS API compatibility layer 与纯 JS 测试
├── contracts/artifact-manifest/  # versioned JSON Schema 与 V8 source lock
├── engine/
│   ├── crates/
│   │   ├── capi-abi/              # 零依赖 ABI record/validation
│   │   ├── capi/                  # C ABI 实现与平台 surface glue
│   │   ├── core/                  # Session/Host orchestration
│   │   ├── runtime-v8/            # V8/deno_core runtime 与 JS extensions
│   │   ├── graphics/              # Canvas/WebGL/Skia/GPU resource pipeline
│   │   ├── audio/                 # audio engine
│   │   ├── io/                    # network/storage/decode/VFS support
│   │   ├── platform/              # 平台 policy/presenter
│   │   │   └── src/{android,linux}/ # 按 OS 命名；不把 Linux 伪装成 generic desktop
│   │   ├── android-jni/           # Android JNI cdylib 边界
│   │   └── shared/                # 无平台所有权的共享协议与状态
│   └── tools/
│       ├── snapshot-gen/          # 链接 engine 的 snapshot 工具
│       ├── player/                # 独立 Linux player
│       └── c-host-example/        # 链接 engine 的 C host driver
├── include/migo/                  # 公共 C ABI 与 typed platform descriptors
├── platforms/                     # 平台 package/Host Kit；不是 engine core
│   ├── android/
│   ├── linux/                     # Linux Host Kit；核心仍不依赖 Qt/GTK/SDL
│   └── windows/                   # 当前仅 spike 与设计输入
├── tools/                         # 不链接 engine 的 repo/release 工具
│   └── artifact-manifest/
├── scripts/                       # 可复现 build/package/contract recipes
├── examples/                      # 外部消费方式与设备 probes
└── tests/                         # 跨语言/ABI contract tests
```

目录规则：

- `engine/crates` 只放可复用 runtime libraries；平台 delivery artifact 不能靠临时改 `crate-type` 产生。
- `engine/tools` 放需要链接 engine workspace 的可执行工具；顶层 `tools` 放独立构建/发布工具，避免把 V8、Skia、系统图形库拖入 manifest 校验器。
- `platforms/<os>` 只有在该平台存在 package、Host Kit、spike 或 CI 时创建；不预建空目录制造“已支持”错觉。
- 平台实现先在 `platform/src/<os>` 或 `graphics/src/backend/<family>` 内形成稳定边界。只有出现独立 consumer、不同依赖图或显著构建隔离收益时才拆 crate。
- `desktop` 不是平台边界：Linux、Windows 和 macOS 的窗口、线程、图形与发布合同不同，目录和类型必须使用真实 OS 名称；toolkit adapter 放在独立 Host Kit。
- 不创建一个包含所有 native window 的无类型大 union；公开 envelope 指向平台强类型 descriptor。

### 2.3 依赖方向

当前 Cargo 依赖方向如下。箭头表示“左侧依赖右侧”，不是控制流：

```text
capi-abi                         (zero dependency)
io / graphics / audio ────────► shared
runtime-v8 ───────────────────► shared + io
core ─────────────────────────► shared + io + graphics + audio + runtime-v8
platform ─────────────────────► shared + io + graphics + core
capi / android-jni / tools ───► 上述所需层
```

图表示所有权，不要求每条 Cargo edge 与图一一对应。不可违反的规则是：

- `capi-abi` 保持零依赖，可在没有 V8/Skia/系统 SDK 的环境验证布局。
- `shared` 不依赖 UI toolkit、JNI、Win32、AppKit 或 ArkUI；`shared` 与 `io` 都不得依赖 `deno_core`、`deno_error`、V8 或 `runtime-v8`。
- 基础 crate 必须直接声明自己使用的 `serde_json`、Serde derive 和 Tokio feature，不能依赖上层 crate 的 Cargo feature unification 才能单独构建。
- `runtime-v8` 不拥有 native window；`graphics` 不拥有 App lifecycle。
- 平台包可以依赖 engine，engine core 不能依赖 Gradle、WinUI、SwiftUI 或 ArkUI package 层。
- 平台 Host Kit 不通过修改 JS API 语义来迁就平台实现。

## 3. SDK 与宿主边界

### 3.1 SDK 为什么不带顶层窗口

第三方 App 已经拥有自己的 UI 树、导航、窗口、输入法、accessibility、权限和前后台生命周期。SDK 自己创建顶层窗口会造成：

- 与 App 的窗口层级、焦点、IME、无障碍和路由冲突；
- 无法自然嵌入 Qt、GTK、WinUI、AppKit、ArkUI 或 Android View；
- 多窗口、分屏、DPI、refresh rate 和 Surface 重建的所有权不清；
- 额外合成层和不可控的 present 时序。

因此 SDK 核心只接受 host-owned native target。`engine/tools/player` 可以创建窗口，因为它是一个 App，而不是 SDK 行为。

### 3.2 所有权

宿主负责：

- 顶层/子窗口、View/Control 节点和系统 event loop；
- native target 在 Migo 明确释放前保持有效；
- UI-thread API、输入、IME、clipboard、权限和生命周期桥接；
- 最佳平台 frame clock，并在被请求后回传 vsync；
- 宿主目录、内容安装来源与用户可见错误 UI。

Migo 负责：

- Session、JS runtime、内容生命周期和 capability policy；
- render worker、GPU context/resource、Canvas2D/WebGL command execution；
- 在 release 状态到达前持有自己应持有的 native reference；
- callback cancellation、generation gate 和 stale target 拒绝；
- artifact/runtime identity、自检和可诊断错误。

### 3.3 第三方桌面 App 的接入

低层稳定边界是 C ABI；平台 Host Kit 是薄适配，不是另一个 engine：

| 宿主 | 目标接入方式 | 窗口所有者 |
|---|---|---|
| C/C++/自研引擎 | headers + CMake/pkg-config/NuGet/xcframework | App |
| Qt Widgets/X11 | 独立源码 Host Kit，以宿主 parent `QWidget` 的 native child XID 接入；当前增量只负责 Surface | Qt |
| Qt Wayland/Qt Quick | 等公开 native handle 或零拷贝 texture/fence 合同后分别实现，不走 private API/overlay/readback | Qt |
| GTK | 独立 `migo-gtk` adapter，按 X11/Wayland backend 获取 display/surface | GTK |
| SDL/GLFW | 示例 adapter，复用其窗口与输入循环 | SDL/GLFW |
| WinUI 3 | Host Kit 接受 SwapChainPanel interop 或 child HWND | WinUI/App |
| AppKit/SwiftUI | Host Kit 嵌入 `NSView`/`CAMetalLayer` | AppKit/SwiftUI |

adapter 不能缓存超过合同生命周期的 native handle，也不能替 SDK 启动第二个 UI loop。toolkit 没有稳定公开 native surface 时，应提供该 toolkit 专用 Host Kit，而不是依赖私有字段或地址偏移。

### 3.4 平台原生 Host Kit 产品层

低层 C ABI 是正确性和长期兼容边界，但不能要求每个 App 重写 Surface、输入、IME、frame clock 和异步退休逻辑。每个正式支持的平台都应在 C ABI 之上交付原生 Host Kit；统一的是产品分层和生命周期语义，不是 UI class 或窗口实现：

```text
L2  optional standalone shell   Activity / Window / Player / sample App
                 │
L1  native embeddable Host Kit  View / Widget / Control / ArkUI Component
                 │
L0  stable engine boundary      C ABI + host-owned native Surface
```

- L0 永远不创建顶层窗口、UI loop 或 toolkit object。
- L1 可以创建自己内部的子 Surface/View，但它必须像普通平台控件一样由宿主布局、裁剪、显示和销毁；不得启动第二个 UI loop。
- L2 只是零样板 convenience 和验收工具。Android `MigoGameActivity`、Linux `migo-player` 即使随 SDK 发布，也不改变 L0/L1 的窗口所有权。
- toolkit 依赖只能存在于独立 Host Kit package。Qt、GTK、WinUI、AppKit、SwiftUI 或 ArkUI 不得进入 engine Cargo graph 或基础 C SDK。

Host Kit 必须提供两种 Session 所有权形态，但不要求在同一个 class 中用布尔参数切换：

| 形态 | Session 所有者 | View 消失时 | 适用场景 |
|---|---|---|---|
| Managed | convenience Host Kit | 先异步 retire Surface，再按产品策略销毁 Session | 简单页面、独立游戏容器 |
| Bound | App | 只 retire attachment；Session 可继续 paused 并绑定新 View | 导航、预热、重挂载、多窗口、自研引擎 |

`Session` 生命周期不得隐式等同于 toolkit object 生命周期。尤其在 X11/Wayland 上，native target 没有 Android `ANativeWindow_acquire` 等价物；Widget/QWindow 销毁前必须先 begin-detach，并等 release query 到 `RELEASED`。通用 wrapper 的析构函数不能阻塞 UI thread、销毁 pending observer 或静默丢失 attachment；无法完成异步清理时必须 fail fast，让 Host Kit 修正所有权而不是制造 driver use-after-free。

显示集成分成两个独立合同：

1. **Direct Surface**：Android `SurfaceView`、X11 child window、Wayland surface、child HWND、`NSView`/`CAMetalLayer`、XComponent。Migo 直接 present 到宿主放置的 native target，这是首个默认路径。
2. **Compositor texture**：Qt Quick、复杂 GTK scene graph、自研 GPU compositor。必须使用**同 GPU device** 的 native texture import/export，且 acquire 与 release **两个方向的同步都必须存在**；CPU readback、每帧 bitmap upload 或把 native child window 浮在 scene graph 上都不能作为 fallback。

   ⚠️**同步不等于"显式 fence 随 descriptor 传入"**（2026-07-22 实测订正）：两个真实消费者的公开导入 API **都收不了 fence**——GTK 4.14 `GdkDmabufTextureBuilder` 的字段里没有任何 fence/sync 参数，Qt 6.4 四个 `QSGxxxTexture::fromNative()` 亦然。Linux dmabuf 的生态标准是内核**隐式同步**，Vulkan/D3D/Metal 才用显式 semaphore/fence，且要由宿主在自己的渲染钩子里插入等待，而不是塞进纹理句柄旁边。按字面要求"显式 fence 经 descriptor 传入"会让 Qt Quick 与 GTK 4 **都不可实现**。

第二条路径需要新的平台强类型 descriptor 和同步合同，不得把 texture/fence 塞入现有 window descriptor。没有该合同的平台 Host Kit 必须明确不提供对应控件，而不是以低性能兼容层冒充实现。

音频默认由 Migo 解码、混音并打开平台输出；Host Kit 负责 audio focus/session、权限、路由和 interruption。已经拥有全局 mixer 的大型宿主以后可选择实时安全的 host audio sink，但它是独立可选能力，普通 View 集成不能因此被迫提供 PCM callback。

首批原生集成形态为：

| 平台/宿主 | 原生可嵌入层 | 独立 convenience | 当前合同 |
|---|---|---|---|
| Android | `MigoGameView` 或 App 自己的 `Surface` | `MigoGameActivity` | 已实现 candidate |
| Linux C/C++ | App 自己的 X11/Wayland target | `migo-player` | 已实现 candidate |
| Linux Qt 6 Widgets/X11 | `MigoQtX11SurfaceView`，借用 Session-scoped `SurfaceHost`（Bound） | `MigoManagedSession`，拥有 Session 与 callback table（Managed）；顶层窗口两者都由 App 提供 | 已实现 candidate：Surface 生命周期 + 输入/焦点/IME/frame request + 两种所有权形态 |
| Linux Qt 6/Wayland | toolkit adapter | 无 | Qt 6.4 环境没有满足本合同的公开 native handle API，不使用 private header |
| Qt Quick | `QQuickItem` texture consumer | 无 | 等待零拷贝 texture/fence ABI；禁止 child-window overlay workaround |
| GTK 4 | 无 Direct Surface 路径 | 无 | **与 Qt Quick 同一阻塞**：GTK 4 只给实现 `GtkNative` 的 widget（顶层/popover）`GdkSurface`，并移除了 `GtkSocket`/`GtkPlug`，宿主布局里放不进可 present 的原生子目标；证据见 `scripts/test-gtk4-surface-capability.sh` |
| Windows | child HWND / WinUI Control | sample Window | Milestone B |
| macOS | `NSView`/SwiftUI wrapper | sample controller | Milestone C |
| iOS | `UIView`/SwiftUI wrapper 内嵌 WKWebView | sample controller | Milestone E |
| OpenHarmony | ArkUI Component/XComponent wrapper | sample Page | Milestone D |

Linux Qt 首个增量只负责最难且可独立验证的 Surface 生命周期：从 Qt 的公开 X11 native interface 取得 `Display*`/XID、把 logical size 转为 physical pixels、合并同一 event-loop turn 的 resize、attach/update、非阻塞退休和 release 后关闭。具体合同是：

- App 在 Session owner/Qt GUI thread 为每个 `MigoSession` 创建一个地址稳定、不可复制也不可移动的 `SurfaceHost`；它保存该 Session 的 generation high-water mark，替换 View 必须复用它，不能从 1 重新开始；controller 和 Qt View 的跨线程控制调用必须在读取 Qt/native 状态或进入 C ABI 前返回 `MIGO_ERROR_WRONG_THREAD`。
- `MigoQtX11SurfaceView` 只借用该 controller，并强制构造时传入 App-owned `QWidget& parent`，因此不能意外成为顶层窗口；每个 View 只保护自己成功绑定的 generation，提前构造的被动 replacement 不能 resize、retire 或阻止销毁另一个 View 的 attachment。
- View、所有祖先 native widget、X11 `Display*`/XID 与 GUI event loop 必须活到 release observer 报告 `RELEASED`；析构或 `SurfaceAboutToBeDestroyed` 提前发生时 fail fast。
- adapter 使用 `WA_NativeWindow`/`WA_PaintOnScreen`，并以 `WA_DontCreateNativeAncestors` 避免把宿主整条 Widget 祖先链强制 native；它不创建 `QPainter`、CPU bitmap 或额外合成层；resize 在 UI event loop 内 latest-wins 合并；release poll 只在退休期间运行，并在异常长等待时退避且发出一次 stalled 诊断，绝不以 timeout 销毁仍 pending 的 target。
- 它不抢占 callback table：`on_request_frame` 由 App 安装，View 只提供 `requestFrame()` 供 App 在自己的回调里调用——Host Kit 抢走 callback table 就等于替 App 决定了帧策略。输入、焦点、IME 与 frame request 已交付（2026-07-22），仍未交付的是拥有 Session 的 Managed wrapper，所以它仍不是完整 `GameView`。

该 Host Kit 默认以源码/CMake target 交付，使 adapter 与宿主的 Qt minor、编译器和 C++ runtime 一致。仓库提供的 install rule 是本地/发行方构建入口，不代表 release 已新增无 manifest 的预编译二进制；一旦官方 package 分发 `.a`/`.so`，必须把它们纳入第 8 节的逐 artifact identity 和 package verifier，并记录精确兼容的 core SDK manifest SHA-256。这样 Host Kit 的 arch、OS/glibc、GLIBCXX、CPU、Qt/C++ ABI 与 core package 的 V8 revision、GN args、snapshot 参数/hash 是一条可验证关系，而不是两个可任意组合的版本号。

## 4. C ABI、Session 与 Surface 生命周期

### 4.1 ABI 规则

- C consumer 的唯一总入口是 `include/migo/migo.h`；子头可供实现内部组织，但示例、package smoke 和外部文档不得让用户拼装 include 顺序。
- `MIGO_C_ABI_HAS_RUNTIME` 是编译期 platform-capability 宏，只表示当前 target 的交付物是否存在可链接 runtime；它不是运行时 backend 探测，也不能把尚未交付的目标平台宣传成支持。
- 所有可扩展 struct 以 `struct_size`、`abi_version` 开头，调用者完整清零后填写。
- 公开 enum-like 类型使用固定宽度整数和数值宏，不使用 C enum 或 packing pragma。
- 输入 struct 按已声明 prefix 复制；旧 client 的较短 prefix 可兼容，未知较长版本 fail closed。
- 输出 struct 只写调用者声明的 prefix，不越界覆盖未来字段。
- descriptor pointer 只在调用期间借用；实现成功返回前必须复制 token，并对 ref-counted target 取得自己的引用。
- LP64、LLP64、GCC/Clang ILP32、MSVC x86 都是 layout contract；“当前只交付 64 位”不能成为不测 32 位布局的理由。
- ABI 冻结前必须同时通过 old-client/new-library、symbol allowlist、calling convention 和平台 descriptor 测试。

### 4.2 Typed Surface descriptor

公共 envelope 是 `MigoSurfaceDescriptor`，payload 按平台强类型化：

- Android：`ANativeWindow*`；
- Linux X11：`Display*` + `Window`；
- Linux Wayland：`wl_display*` + `wl_surface*`；
- Windows：child `HWND` 或 WinUI SwapChainPanel interop，二者不可混为一个 handle；
- macOS：`NSView*` 或 `CAMetalLayer*`，二者语义独立；
- OpenHarmony：`OHNativeWindow*`。

iOS v1 不提供 native Surface descriptor，因为 App Store 默认 backend 是 `WKWebView` Host Kit。将来若增加非 App Store native backend，必须使用新 platform kind，不能改变现有含义。

descriptor 解析发生在 attach/update 控制路径。它不能造成 per-frame handle conversion、字符串匹配或 backend discovery。

### 4.3 generation 与异步 detach

一个 Session 同时最多有一个 attachment。宿主为每个新 native target 选择单调递增且非零的 generation；resize、scale、color space 和 presentation mode 更新沿用当前 generation。

```text
Detached
   │ attach(generation=N)
   ▼
Attached(N) ── update(N) ──► Attached(N)
   │ begin_detach consumes attachment
   ▼
Retiring(N) ── GPU/driver retirement ──► Released(N)
   │                                      │
   │ query=PENDING                        ├─ host may destroy native target
   │                                      └─ release observer may be destroyed
   └─ Session destroy is refused
```

规范行为：

1. `migo_surface_begin_detach` 成功时消费 `MigoSurfaceAttachment*`，返回唯一的 `MigoSurfaceRelease*`；失败时不消费任何对象。
2. begin-detach 返回不代表 native resource 已安全。宿主必须保持窗口、Surface、display connection 和需要的 event loop 有效。
3. `migo_surface_release_query` 是权威 level；`on_surface_released` 只是可选 dispatcher wakeup edge。丢边、延迟或被取消都不能改变正确性。
4. release 为 `PENDING` 时，observer 不能销毁，Session 也不能销毁。
5. release 为 `RELEASED` 后，宿主可释放 native target。已 released 的 observer 不持有 Session/Surface lease，因此可在随后成功销毁 Session 后再做最终 query/destroy。
6. `migo_session_destroy` 不隐式 detach；live attachment、transition 或 pending release 都返回 `MIGO_ERROR_INVALID_STATE`，所有权留给调用者重试。
7. Surface loss 不等于 detach。attachment 仍由宿主持有，必须走同一 retirement 协议。

### 4.4 callbacks 与线程

- 一个 Session 的公开调用由宿主串行化；Engine 的独立 Session 可以并行。
- callback table 在 Session 首次运行/attach 前安装一次并复制，之后不可替换。
- 任一用户 callback 非空时必须提供 dispatcher；dispatcher 可从 engine worker 调用，必须线程安全且快速返回。
- 用户 callback 只在 dispatcher task 中运行，期间不持有 engine/session/attachment lock。
- dispatcher 拒绝 task 时，Migo 回收 task；不得在 engine thread 上 fallback 执行用户代码。
- Session destruction 取消未开始的 callback；已排队 task 后续只能释放自身存储，不能再访问 `user_data`。
- listener 的异常只能影响当前 listener；即使宿主替换的诊断 sink 自身抛异常，也不能阻断后续 listener。

## 5. 图形与帧调度架构

### 5.1 三条平面

```text
Control plane: attach / resize / lifecycle / capability / backend construction
Data plane:    decoded commands / resource ids / upload payload / input batches
Render plane:  execute / submit / present / fence / device-loss recovery
```

通用多态只允许出现在 control plane。当前 `GraphicsPlatform` 将 `EglProvider` 与 `EglSurfaceFactory` 注入 graphics；backend identity 在初始化/attach 时验证并固定。不要把这个冷路径 seam 扩散成每个 draw call 的 trait object。

### 5.2 当前 EGL family

Android 与 Linux 当前共享的是 EGL/GLES 行为合同，而不是 window 实现：

- Android provider 动态加载系统 `libEGL.so`，从持有强引用的 `ANativeWindow` 创建 EGLSurface。
- Linux provider 动态加载 `libEGL.so.1`；X11 使用宿主 `Display*`/`Window`，Wayland 使用宿主 `wl_display*`/`wl_surface*` 和运行时加载的 `libwayland-egl.so.1`。
- EGL loader 保持 1.4 必需符号下限，平台 display/window entry point 在运行时解析 EGL 1.5/EXT 版本并有受控 fallback。
- Skia、Canvas2D、WebGL 和 onscreen present 尽量处在同一 device/share-group，禁止默认路径做 CPU readback 或跨 device copy。

### 5.3 backend 选择规则

1. 每个发布 slice 编译进有限、明确的 backend family；不在每帧自动猜测。
2. 首次初始化可以按驱动能力选择该 family 内的安全变体，并记录 telemetry。
3. fallback 必须保持正确性，但不能被性能报告当成主 backend。
4. software rendering 只用于 CI、诊断或明确兼容模式。
5. 新 Vulkan/Metal/D3D backend 必须先通过像素 conformance、resource lifetime、device-loss 和整帧性能门，再替换默认。
6. 如果两个 API 之间不能证明零拷贝同步与资源共享，宁可使用成熟的单 family 路径。

### 5.4 帧调度

engine 在需要画面时发出一次 `on_request_frame`。宿主使用平台最佳 frame clock 安排一次 callback，再调用 `migo_session_notify_vsync(timestamp)`：

| 平台 | 首选 frame clock |
|---|---|
| Android | AAR Host Kit 使用 Java `android.view.Choreographer`；纯 NDK/C host 使用 `AChoreographer`；新 API 运行时探测 |
| Wayland | `wl_surface.frame`，由拥有 display dispatch 的宿主处理 |
| X11 | Present extension MSC/complete notify；不具备时才用受测的 swap-interval fallback |
| Windows | DXGI frame-latency waitable object/DirectComposition 或 WinUI compositor timing，按 presenter 类型实现 |
| macOS | `CVDisplayLink`/display link；所有 AppKit object 操作仍回 main thread |
| iOS | `CADisplayLink` 或 WKWebView 自身 compositor；两种 backend 不共用假的统一 clock |
| OpenHarmony | XComponent/ArkUI 提供的期望帧率与平台回调 |

frame clock 只负责 pacing，不在 callback 中执行输入、网络或任意长任务。

## 6. JavaScript runtime 与 V8 component

### 6.1 统一的是 JS 行为

产品合同是 observable JavaScript semantics：global、module loading、timers、Canvas2D/WebGL、audio、network、storage、lifecycle、input、错误和异步顺序。它由 `adapter` conformance 与 engine tests 定义，而不是由“所有平台都叫 V8”定义。

当前 engine 的 production native runtime 是 `runtime-v8`（`deno_core` + V8）。增加第二 runtime 时：

- extension registration、op surface 与 capability 必须生成/验证，不能手工维护两份漂移表；
- runtime 选择是 compile-time/package-time 决策，不在每个 op 上做动态分派；
- async ordering、microtask、exception、typed array、WebGL object identity 必须逐项 conformance；
- backend 特有能力只能通过显式 capability 暴露，不能静默改语义。

当前 **Platform/V8 Phase A 已完成**：platform crate 不直接依赖或组装 `deno_core::Extension`，V8 extension assembly 留在 `runtime-v8`，`shared`/`io` 也已从 V8 依赖图中解耦。完整的多 runtime backend 边界尚未完成，当前无 `JsBackend` trait；在第二个 runtime 有可运行 vertical slice 前，不为想象中的复用提前加入热路径动态分派。

### 6.2 每个平台是否都要编译 libV8

所有使用 native V8 的交付目标都要自己的 V8 component。最小 identity 是：

```text
(target triple, OS, arch, ABI, runtime floor, toolchain, linkage model,
 CPU baseline, rusty_v8 revision, upstream V8 revision, normalized GN args,
 applied patches, source/build recipe)
```

任一维度不同就不能复用 archive。特别是：

- Android Bionic archive 不能用于 Linux glibc；
- Linux `.so` 需要满足共享库/PIC/TLS 链接合同，不能拿仅适合 executable 的 archive；
- Windows MSVC `.lib` 不能与 GNU archive 混用；
- macOS universal package 内的 arm64/x86_64 是两个独立 component，`lipo` 只发生在最终 package 层；
- debug/release GN 参数、pointer compression、sandbox、i18n、startup data 和 CPU flags 都属于 identity；
- 一个来源不明但“能链接”的上游预编译 archive 可用于 spike，不能成为 Migo verified release。

计划矩阵：

| 平台 slice | runtime | V8 component |
|---|---|---|
| Android arm64-v8a | native V8 | 必须独立构建 |
| Android x86_64 | native V8 | 必须独立构建 |
| Linux GNU x86_64 | native V8 | 必须独立构建，shared-library-compatible |
| Windows MSVC x86_64 | native V8 | 必须独立构建；当前尚无 verified component |
| macOS arm64 | native V8 | 必须独立构建 |
| macOS x86_64 | native V8 | 必须独立构建 |
| OpenHarmony arm64（若选 embedded V8） | native V8 | 必须独立构建，不能复用 Android |
| OpenHarmony API 11+（若选 JSVM） | system JSVM | `v8_revision=null`，必须记录 JSVM/OS runtime floor |
| iOS App Store 默认 | WKWebView/WebKit | 不携带 V8；记录 WebKit/OS floor 与 backend policy |

### 6.3 snapshot 不是通用资源

snapshot 必须绑定：

- target triple/arch 和 CPU policy；
- runtime kind、product profile、feature set 与 hash；
- generator schema/版本和规范化参数；
- external-reference table hash；
- bootstrap inputs、Rust runtime sources、JS sources；
- `deno_core` 版本；
- 生成它的 V8 archive SHA-256；
- snapshot bytes size/SHA-256。

Android snapshot 必须在目标架构的真实 runtime 环境或等价受控 runner 上生成。不能因 host 工具方便而用 x86_64 host snapshot 填入 arm64 包。Linux 当前若采用 `snapshot_policy=none`，manifest 仍必须显式记录 `none` 和空列表，不能省略字段。

## 7. 平台最佳方案

版本只有同时写入 artifact manifest、由工具链限制并在最低版本 lane 通过后才成为支持承诺。下面“目标”版本在此之前仍是设计值。

| 平台 | 最低版本合同 | 首批 arch/CPU | 默认图形/宿主 | 状态 |
|---|---|---|---|---|
| Android | API 26 | arm64-v8a/armv8-a；x86_64/x86-64-v1 | `ANativeWindow` + EGL/GLES + Skia；Java/NDK Choreographer | 已实现 candidate |
| Linux GNU | glibc 2.31、GLIBCXX 3.4.28 | x86_64/x86-64-v1 | host X11/Wayland + system EGL/GLES | 已实现 candidate |
| Windows | Windows 10 1809/build 17763 | x86_64/x86-64-v1 | HWND/SwapChainPanel + ANGLE D3D11 | 目标；编译 spike only |
| macOS | macOS 12.0（提议） | arm64/armv8-a、x86_64/x86-64-v1 | NSView/CAMetalLayer + Metal family | 目标 |
| iOS/iPadOS | iOS 15.0（提议） | arm64 | WKWebView/WebKit Host Kit | 目标 |
| OpenHarmony | API 10 native surface；API 11 system JSVM 实验 | arm64/armv8-a | XComponent/NativeWindow + EGL/GLES | 目标 |

### 7.1 Android

- `minSdk=26` 是产品合同。编译、manifest、AAR metadata、C SDK 和设备 CI 必须一致。
- 宿主提供 `ANativeWindow*`；Migo 在 attach 成功前 `acquire` 自己的引用，每个可能重叠退休的 generation 各自持有引用。
- 默认使用系统 EGL/GLES 与 Skia GL family，避免中间 bitmap、Java Canvas 或额外 Surface 合成。
- AAR 当前使用 Java `android.view.Choreographer`；纯 NDK/C host 使用 `AChoreographer`。两者都保持 demand-driven、一次请求最多挂一个 callback。API 30/31/33 增加的预分配、frame-rate/timeline 能力只做动态探测。
- audio 优先使用 Oboe/AAudio 路径并保留经验证 fallback；API26 恰好是 AAudio floor，但具体默认仍由 latency/underrun/设备矩阵决定。
- AAR 是 Java/Kotlin Host Kit；C SDK 是 NDK static archive + CMake。二者是不同交付物，不能混淆 `libmigo.so` 的 JNI export 与公开 `migo_*` C ABI。

### 7.2 Linux

- SDK 不链接或拥有 toolkit；它接收 X11 或 Wayland token。
- X11 宿主在跨线程使用同一 `Display*` 时必须在打开连接前满足 Xlib threading 合同，并保持 display/window 到 release。
- Wayland 宿主拥有 object role 与 dispatch loop；Migo 不能私自 dispatch 宿主 queue。`wl_egl_window` 只在 presenter 内创建和退休。
- 首批发布只承诺 glibc GNU x86_64，不把 musl、Bionic 或其他 libc 归入“Linux”。它们需要独立 target/schema/package。
- 图形保持 system EGL/GLES。Mesa、NVIDIA 和主要 Wayland compositor 都要进兼容矩阵。
- audio 当前可运行基线可保留；PipeWire 原生低延迟路径只有在真实发行版的 latency、设备切换与维护成本优于现状后才升为默认，ALSA fallback 仍单独验证。

### 7.3 Windows

- 首个 presenter 分两种：Win32 child `HWND` 和 WinUI SwapChainPanel interop。二者有不同线程、COM 与 composition 生命周期，不能互相 reinterpret。
- ANGLE D3D11 是首个默认 backend：成熟、覆盖广，且现有 EGL/GLES/Skia GL 数据路径可复用。通过 `EGL_ANGLE_platform_angle` 明确请求 backend，不依赖 ANGLE 隐式默认。
- D3D12/Vulkan backend 只有在 shader warm-up、frame p99、device-loss、显存和功耗全链路胜出时替换默认；不因“更新”自动升级。
- App 拥有 message loop；SDK 不创建隐藏顶层窗口。COM apartment 和 WinUI object 访问必须留在宿主规定线程。
- 当前 spike 只验证五个 crate 的 `cargo check` 与接口可承载性。下一里程碑必须用真实 Windows V8 component 链接并启动 isolate、创建真实 HWND EGLSurface、完成像素/readback/present，再讨论支持等级。
- Windows 文件完整性不能沿用 Unix inode/flock/mode 假设；需要 `CreateFile` reparse policy、file ID、`LockFileEx` 和 ACL/read-only 策略的独立安全设计。

### 7.4 macOS

- 宿主提供 `NSView` 或 `CAMetalLayer`；所有 AppKit object 操作遵守 main-thread 规则，render command 可在专用线程编码。
- native family 是 Metal。WebGL/GLES 兼容层首选 ANGLE Metal；Canvas2D 可先使用同一 ANGLE family，避免跨 device copy。
- 原生 Skia Metal/Graphite 只有在能与 ANGLE 证明共享 device、texture、fence 且没有额外 blit 时才作为优化。否则“原生 API”标签不等于更快。
- arm64 与 x86_64 分别构建 V8/engine slice，最后组成 xcframework/universal distribution；manifest 保留每个 slice identity。
- 首个部署目标提议为 macOS 12，构建使用当前稳定 SDK并通过 availability check 使用新 API。最终 floor 以 CI 与 artifact schema 为准。

### 7.5 iOS/iPadOS

- 全球 App Store 默认 backend 是 `WKWebView`：它是平台原生可嵌入 View，并符合 WebKit/动态内容政策边界。
- Host Kit 负责 navigation policy、JS/native bridge、content install、lifecycle、audio session、input 和错误映射；不能假设 native-V8 engine 的内部线程/Surface 模型存在。
- JS API conformance 必须与 native backend 共用测试向量，但允许 WebKit 实现采用不同内部路径。
- 自带 V8/JIT 仅可作为明确的企业、研究或获批准 entitlement 的独立 distribution；不得和 App Store artifact 共用支持声明、manifest 或性能数字。
- iOS 15 是初始建议 floor，不是当前支持承诺。

### 7.6 OpenHarmony

- ArkUI `XComponent` 提供 native Surface 生命周期；Host Kit 将 `OHNativeWindow*` 交给 EGL/GLES presenter，并按 native object 引用规则持有/释放。
- API 10 可覆盖目标 native XComponent/EGL 集成；系统 JSVM C API 从 API 11 起，因此两层 floor 必须分别记录。
- 系统 JSVM 可减少 V8 包体并更贴合平台，但不能直接假设兼容 `deno_core`、Migo snapshot 或 V8 行为。先做独立 prototype：JS conformance、microtask/exception、native binding、snapshot、debugging、冷启动、RSS、frame p99。
- 如果 JSVM 未达合同，使用 OpenHarmony 自己的 V8 component；绝不复用 Android Bionic archive。
- HAR/HAP、ArkUI lifecycle、权限和输入都属于平台 Host Kit，不进入 engine core。

## 8. Artifact manifest 与可复现发布

### 8.1 当前 schema

当前仓库的版本化合同是：

- `migo-artifact-manifest/v1`：Android AAR 内单 slice identity；
- `migo-v8-component-manifest/v1`：一个 V8 component；
- `migo-artifact-package-index/v1`：多 slice package index；
- `migo-release-attestation/v1`：package/index hash 绑定；
- `migo-linux-package-manifest/v2`：Linux C SDK package；
- `migo-android-package-manifest/v2`：Android C SDK per-ABI package。

旧 package v1 schema 只用于读取历史 fixture；release generator 只能产生 v2。

### 8.2 每个平台 artifact 的必填身份

每个实际可下载/安装的 artifact 都必须写入自己的 manifest，至少包含：

```text
schema/version
product_profile/build_type/codegen_profile
target triple + OS + arch + ABI
minimum OS/API/glibc/GLIBCXX/system-runtime floor
CPU baseline + required CPU features
compiler/Rust/SDK/linker/sysroot identity
runtime backend
rusty_v8 version/revision + upstream V8 revision + normalized GN args + patches
V8 archive/binding hashes，或显式 not-applicable/null 与非 V8 backend identity
snapshot policy + normalized parameters + all input/output hashes
graphics backend family + required API
package-relative regular files: size_bytes + SHA-256
source revision + build recipe/hash + licenses
```

“version requirements 写在 README”不构成发布合同。loader/installer 可以比文档更早地拒绝不兼容 artifact，但不能比 manifest 更宽松。

### 8.3 v2 package verifier 的行为

Linux/Android C package v2 必须：

- `version` 是最长 128 字节的 ASCII SemVer 2.0.0；在把版本插入路径、CMake 或 pkg-config 内容前先验证，拒绝路径分隔符、空白、前导零和其他非 SemVer 输入；
- 只接受 release build、已知 profile/codegen profile 和精确 target/floor；Linux 记录路径无关的 sysroot recipe identity，并要求 engine package 与 V8 component 完全一致；
- 完整验证 V8 component，而不是只比较 `rusty_v8_revision`；
- 验证 package target 与 V8 target、runtime floor 一致；
- 验证 snapshot 的 V8 archive hash、参数、输入、features 和 bytes；
- 从 staging root 重新读取每个 package-relative path；
- 拒绝绝对路径、`..`、非普通文件、size 不符和 SHA-256 不符；
- 拒绝未声明的额外普通文件；Android 拒绝任意 symlink；Linux 的真实库必须精确为 `lib/libmigo.so.<manifest.version>`，且只接受并核对 `libmigo.so -> libmigo.so.1 -> libmigo.so.<manifest.version>` 这一条 soname chain；
- 在链接 engine 前重新验证 V8 component manifest 对应的实际 archive 与 Rust binding bytes；
- Android release package 在 Rust 编译前必须拒绝未 materialize 或过期的目标 ABI snapshot；不得让 `runtime-v8` 安全回退到 source JS 后仍生成声称 `snapshot_policy=embedded` 的 manifest；
- 验证必需 headers/library/metadata 存在。

manifest 生成后修改一个 staged byte 必须导致 verifier 失败。只记录 filename/size 而不读取实际文件不再允许。

### 8.4 integrity 与 authenticity 分离

SHA-256 和 canonical manifest 能证明内部一致性，不能证明发布者身份；攻击者可同时替换 package 和 sidecar。正式公开 release 还需要：

- 受保护 builder；
- package/index attestation 签名或透明日志；
- 公布 verifier、公钥轮换和撤销策略；
- SBOM、第三方许可证和 source/build recipe；
- reproducible build 差异报告，无法 bit-for-bit 时说明不可复现输入。

在签名链路落地前只能称 verified identity，不能称 trusted/signed artifact。

## 9. 测试与发布门

### 9.1 最低版本兼容性门

每个 manifest floor 至少有一条真实或官方等价环境 lane：

- 加载/链接完整 artifact，不是只编译源码；
- JS/API conformance、Canvas2D/WebGL golden、代表性内容；
- lifecycle chaos：前后台、resize、Surface 重建、window destroy race；
- context/device loss 与 resource recovery；
- input、IME、audio interruption、permission denial；
- ABI old-client/new-library、symbol/version、错误注入；
- network/TLS、storage/VFS/content integrity；
- 每个承诺 arch slice 实际启动。

该 lane 允许设置防死锁/OOM 的 ceiling，但不根据 frame time、启动耗时或功耗选择 backend，也不承担性能回归结论。

### 9.2 最新系统性能门

使用当前 stable OS/driver/SDK 和代表性低中高设备，运行同一个 package SHA-256：

- cold/warm start；
- frame p50/p95/p99、jank、input latency；
- decode/upload/shader warm-up；
- RSS/GPU memory/峰值；
- 30 分钟以上 sustained workload 的功耗、温升、降频；
- Surface resize/recreate、refresh-rate/DPI/color-space 变化；
- 与上一 release、平台 WebView/系统方案和备选 backend 做 A/B。

性能门可拒绝 release 或回退 backend，但不能据此宣告提高最低 OS。提高 floor 是独立的支持合同变更，需要使用数据、迁移期和 major/minor policy。

### 9.3 CI 分层

1. source contract：format、lint、unit、JS tests、C/C++ layout、schema、script syntax；
2. cross-target compile：Android NDK、MSVC、Apple/OpenHarmony toolchain；
3. package contract：真实 staging、exports、dependency floor、manifest/hash、外部 consumer；
4. minimum compatibility devices；
5. latest performance devices；
6. release signing/attestation/reproducibility。

没有 V8/Skia/平台 SDK 的普通 PR runner 不应伪造 release 结论。它只运行不需要这些 component 的 source contract；真实 component/package lane 在指定 builder 上执行。

Linux Qt Host Kit 属于隔离的 source contract job：只安装 Qt/X11/Xvfb，链接严格 fake C ABI，验证 controller state machine、native child widget、DPI/resize、异步 release、安装导出和非 XCB fail-closed；它不得链接或构建 V8，也不得让 Qt 进入 engine Cargo 依赖图。Xvfb 的 X11 正向路径和 `offscreen` 负向路径都必须运行，不能因开发机当前使用 Wayland 而静默 skip。

## 10. 依赖策略

### 10.1 是否引入 Actix

不引入。Actix 的主要价值是 HTTP server/actor workload，不是嵌入式游戏 runtime 的 render/input/present。将它放进 engine 会增加：

- message envelope 与调度层；
- runtime ownership 和 shutdown 复杂度；
- 跨线程唤醒、潜在分配和尾延迟；
- Android/iOS/OpenHarmony package 体积与审计面。

现有 Tokio 只应服务 async I/O、timer 和 runtime worker，并保持 feature 最小化、有界队列和明确 shutdown；render/input/present 热路径不经 Tokio task 或通用 actor。

### 10.2 新依赖准入

引入 dependency 前必须回答：

1. 它替代哪段明确能力，谁拥有 lifecycle；
2. 是否进入 shipped artifact 和热路径；
3. 支持哪些 target/floor/arch，是否带 C/C++ runtime 或系统依赖；
4. 二进制/RSS/startup/frame p99 的 A/B 数据；
5. unsafe surface、漏洞响应、license、维护活跃度；
6. 能否锁定版本、离线构建、生成 SBOM 和复现；
7. 移除或 backend 失败时的迁移方案。

可接受的未来依赖类型包括平台官方 bindings、ANGLE、签名/attestation 工具和有明确 benchmark 的 native audio backend。不得为了“架构更现代”引入 wgpu、通用窗口库、ECS 或 server framework。

## 11. 实施路线

### Milestone A：收紧当前 Android/Linux 候选发布

当前代码侧已经完成：

- C/Rust callback layout 对齐，并补齐 LP64/LLP64/ILP32 lanes；
- `on_surface_released` 作为可选 wakeup，query 仍是权威状态；
- listener exception 与诊断 sink exception 隔离；
- Linux/Android C package manifest v2；
- 完整 V8/snapshot identity 与 staged-file SHA-256 verifier；
- Windows MSVC C ABI 持续 CI。
- Linux toolkit-neutral `SurfaceHost` 与 Qt 6 Widgets/X11 Bound view（Surface 生命周期、输入、焦点、IME、frame request），以及不构建 V8 的独立 contract gate。

在指定 V8 builder 上继续完成：

1. 从 pinned source/recipe 重建 Linux 与 Android 各 slice V8 component；
2. 生成完整 `component-manifest.json`，不得为旧 archive 猜 revision/GN args；
3. 用对应 archive 重建 snapshot，并验证所有 fingerprint；
4. 构建 Linux/Android packages，运行外部 CMake/pkg-config consumer；
5. 在最低与最新系统运行同一 artifact 的兼容/性能双门；
6. 关闭 Android x86_64 snapshot/package 与多指真机验证缺口；
7. 只有上述全部通过后冻结 C ABI v1。

Linux Host Kit 的后续增量按独立能力交付，不扩张本次 surface-only 支持声明：

1. **桌面指针通道（2026-07-22 已完成）**：此前鼠标 button/move 与滚轮**没有任何入口**，而运行时已经把 `onMouseDown`/`onMouseMove`/`onMouseUp`/`onWheel` 发布给内容——这些是 wx 表面真实定义的名字（wx 小游戏在 PC 微信上跑）。JS 侧 listener 与 `_internalTrigger*` 都在，却没有任何引擎代码调用、也没有宿主能调用，内容注册后永远静默不触发。已按软键盘的形状补齐宿主通道（引擎暴露能力、宿主生产事件，两路互不合成），入口为 `migo_session_send_pointer_event` / `migo_session_send_wheel_event`。**它不是 Qt 专属前置**：Windows Milestone B 第 5 项与 macOS Milestone C 的 Host Kit 吃的是同一条通道。守卫为 `scripts/test-input-trigger-producer-contract.sh`（任何已发布的输入 listener 失去生产者即红）；权威状态见 `include/migo/README.md` 的 freeze blockers。**遗留代价**：改了嵌入 JS ⇒ Android 的 aarch64 snapshot 已 stale，需在真机重生成后 Android 宿主才能用这条通道（Linux 走源码 bootstrap，已即时生效）；
2. **Qt Widgets Bound input/frame adapter（2026-07-22 已完成）**：转换全部发生在 view 自己的事件处理器里（没有任何 event filter）、GUI 线程内、热路径零分配（以 `malloc` 计数实测，Qt 容器不走 `operator new`）；坐标为 CSS 像素（与 Qt logical 恒等，不得再乘 DPR）；`code` 取自硬件扫描码而非布局；失焦撤回未结束的按压与 preedit；帧由 `QWindow::requestUpdate()` 驱动，未请求的重绘不报帧边界。明确不交付：preedit 的光标/选区（DOM 亦不提供）、失焦时合成按键 up、hover 映射为 touch；
3. **拥有 Session 的 Managed wrapper（2026-07-22 已完成）**:`MigoManagedSession` 拥有 Session、callback table 与 view,但**不拥有 `MigoEngine`**(一个 App 可在一个 engine 上开多个 Session)。engine 回调经宿主自己的 dispatcher 投递到 GUI 线程,teardown 开始后 dispatcher **拒绝**而非排队(拒绝把任务所有权退回引擎,排队则会让它跑在已消失的 Session 上)。**三个软键盘回调一个都不装**——wx 的软键盘模型要求宿主拥有文本字段并回报全文,桌面宿主有物理键盘、内容已直接收到按键与组合态,装了不实现就是伪装支持。销毁顺序恒为 begin_detach → 等 `RELEASED` → destroy,`close()` 不阻塞 GUI 线程,仍活动时析构 fail fast。示例 App 待补;
4. **先设计同 GPU device 的零拷贝 texture/fence ABI**，然后 Qt Quick 与 GTK 4 才可能实现——两者卡在**同一个**前置条件上，此前把 GTK 4 排在 Qt Quick 之前是错的排序。GTK 4 的实测结论（2026-07-22）：子 widget 不是 `GtkNative`、没有 `GdkSurface`，`GtkSocket`/`GtkPlug` 已移除，唯一的原生 surface 是顶层窗口的；往它 present 就是被明令禁止的 child-window overlay（无视布局、裁剪与 z-order）。门禁 `scripts/test-gtk4-surface-capability.sh` 固化该结论，GTK 改变答案的那天它会红——那是解锁信号而不是回归；
6. 每个增量分别进入最低 Linux/Qt 兼容矩阵与最新系统性能矩阵。

### Milestone B：Windows production vertical slice

1. `build-v8-windows` + component manifest + isolate smoke；
2. pinned ANGLE D3D11 package 与 provenance；
3. Win32 child HWND presenter，真实 attach/resize/detach/present；
4. WinUI SwapChainPanel presenter，单独 COM/thread contract；
5. input/IME/frame clock/audio 与 Windows integrity 实现；
6. `migo.dll` export allowlist、CMake/NuGet consumer；
7. Windows 10 1809 compatibility 与最新 Windows performance lanes。

该里程碑的退出条件是外部 App 使用已打包 DLL 在真机出帧并通过 teardown/device-loss，不是 `cargo check` 变绿。

### Milestone C：macOS Metal vertical slice

1. 两个 V8 arch component；
2. NSView/CAMetalLayer Host Kit；
3. ANGLE Metal 基线；
4. 与原生 Skia Metal 方案做零拷贝/整帧 A/B；
5. xcframework/SwiftPM、codesign/notarization 与双版本 CI。

### Milestone D：OpenHarmony runtime decision

1. XComponent/EGL/GLES vertical slice；
2. JSVM 与 embedded V8 两个 prototype；
3. 同一 conformance/performance/device matrix；
4. 以数据选择默认，另一方案不自动成为 shipped fallback；
5. HAR/HAP 和 artifact schema。

### Milestone E：iOS WebKit Host Kit

1. WKWebView container 与 lifecycle/audio/input bridge；
2. JS API conformance 差异清单和可接受 capability；
3. content/security/App Review policy gate；
4. iOS 15 floor 与最新设备 performance lane；
5. xcframework/SwiftPM artifact identity。

## 12. 开源定位与发布阻塞

**已决（保留 BSL + 如实称呼）。** 根 `LICENSE` 是 BSL 1.1：带竞争用途限制，每个版本在自身发布满 4 年时转 Apache-2.0（Change Date 按版本滚动，不是单一固定日期）。Change Date 之前它是 source-available，不满足 OSI 对自由再分发和不得限制使用领域的要求。

此前本节记录的 release blocker 是"README 宣传开源、LICENSE 却是 BSL"的不一致。该项已通过**如实称呼**关闭：README/README_EN 改为"源码可审计 / source-available"，不再自称 open source；许可边界与商业许可入口分别落在 `LEGAL.md` 与 `COMMERCIAL.md`。

生产使用的分界写在 Additional Use Grant 里：嵌入自有 App 在 Small Entity 阈值内免费；作为独立 SDK 转售或提供托管运行时服务需商业许可；**非生产用途（阅读/审计/构建/测试/评测/移植）对任何规模无条件授予**——这条是"可审计"叙事能成立的前提，改许可证时不能动它。

平台 backend、build recipe、schema、verifier 和 conformance tests 保持在同一公开仓库，避免"可审计 core、不可复现平台包"。

## 13. 明确非目标

- 一个跨平台顶层窗口框架；
- 一个所有平台共享的 GPU backend 或 lowest-common-denominator renderer；
- 运行时下载不受 manifest/签名约束的 native component；
- 在一个 App 包里携带所有平台/arch 的 V8；
- 用容器、QEMU 或 compile-only 结果替代所有真机门；
- 为未来想象提前拆出大量空 crate；
- 把 Actix、通用 actor 或 server framework 放进 render/input/present；
- 在 iOS 全球 App Store artifact 中默认承诺 native V8/JIT；
- 用一次 benchmark 永久固定 backend，不再持续测量。

## 14. 官方参考

- Android NDK Native Window：<https://developer.android.com/ndk/reference/group/a-native-window>
- Android NDK Choreographer：<https://developer.android.com/ndk/reference/group/choreographer>
- Windows 版本与 SDK 分离：<https://learn.microsoft.com/en-us/windows/apps/get-started/versioning-overview>
- Windows App SDK 与既有 Win32/WPF/WinForms 集成：<https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/>
- ANGLE backend 与平台支持：<https://chromium.googlesource.com/angle/angle/+/main>
- ANGLE `EGL_ANGLE_platform_angle` 选择：<https://chromium.googlesource.com/angle/angle/+/main/doc/DevSetup.md>
- Apple `WKWebView`：<https://developer.apple.com/documentation/webkit/wkwebview/>
- Apple App Review Guidelines：<https://developer.apple.com/app-store/review/guidelines/>
- Apple `CAMetalLayer`：<https://developer.apple.com/documentation/quartzcore/cametallayer>
- OpenHarmony NativeWindow：<https://gitee.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkgraphics2d/_native_window.md>
- OpenHarmony Native XComponent：<https://gitee.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/native__interface__xcomponent_8h.md>
- OpenHarmony JSVM API（`@since 11`）：<https://gitee.com/openharmony/interface_sdk_c/tree/master/ark_runtime/jsvm>
- OSI Open Source Definition：<https://opensource.org/osd>

这些参考说明平台能力和政策边界；Migo 的实际支持合同仍由仓库中的 versioned schema、artifact manifest、package verifier 和双测试门共同定义。
