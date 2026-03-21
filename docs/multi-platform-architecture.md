# Migo Multi-Platform Architecture Design (Performance-First)

> Status: Design Draft v2
> Last Updated: 2026-03-21
> Covers: Android, iOS, Desktop (Windows / macOS / Linux)
> Design Principle: **Performance first, binary size second, code reuse third**

---

## 1. Design Philosophy

### 1.1 Lessons from WeChat Mini Games

WeChat's mini game engine is the most mature and performance-optimized implementation in this
space. Key architectural patterns proven at scale:

| Pattern | Description | Performance Impact |
|---------|-------------|-------------------|
| **Bare Binding** | Direct V8/JSC native function binding for WebGL/APIs, bypassing any intermediate bridge layer | Eliminates per-call overhead for thousands of draw calls/frame |
| **Direct GL Calls** | Canvas draws directly to GLSurfaceView, no Chrome-style CommandBuffer serialization | Removes serialize/deserialize cost per GL command |
| **NativeBuffer** | Zero-copy shared memory between JS and native for vertex data, texture uploads | Eliminates ArrayBuffer copy for high-frequency data transfer |
| **Native Metal (iOS)** | iOS 8.0.63+ switched from GL ES (via translation) to native Metal backend | ~20-30% GPU perf improvement, lower power consumption |
| **Per-Platform Rendering** | Android: GL ES direct. iOS: Metal native. No cross-platform GL abstraction | Each platform runs its optimal graphics API |

### 1.2 Our Performance-First Principles

1. **Zero-overhead JS binding**: All WebGL/Canvas ops use bare binding (V8 Fast API / JSC C function), no JSON serialization in hot path
2. **Platform-native rendering**: Android = GL ES (EGL native), iOS = Metal (no ANGLE), Desktop = GL ES (EGL native/ANGLE per OS)
3. **Zero-copy data transfer**: SharedArrayBuffer / NativeBuffer for vertex data, image data, audio buffers
4. **No unnecessary abstraction layers**: If an abstraction adds per-call overhead in the hot path, eliminate it
5. **Willing to rewrite**: Existing Android code can be restructured for optimal performance

---

## 2. Per-Platform Strategy

### 2.1 JS Engine

| Platform | JS Engine | JIT | Binary Cost | Rationale |
|----------|-----------|-----|-------------|-----------|
| Android | V8 (deno_core) | Full JIT | ~8 MB .so | TurboFan JIT, proven at scale, same as WeChat Android |
| iOS | System JSC | Full JIT | 0 MB | Apple JIT policy: only system JSC gets JIT. Embedded V8 = interpreter-only = 5-8x slower |
| Desktop | V8 (deno_core) | Full JIT | ~8 MB | Same as Android, no restrictions |

### 2.2 Rendering Backend

| Platform | Graphics API | Pipeline | Notes |
|----------|-------------|----------|-------|
| Android | **GL ES 2.0/3.0 (EGL native)** | EGL -> GL ES -> GPU | Direct, no translation. Same as WeChat Android |
| iOS | **Metal (native)** | Metal API -> GPU | Native Metal, NOT ANGLE. Same direction as WeChat iOS (8.0.63+) |
| Desktop Windows | GL ES (EGL ANGLE) | ANGLE -> D3D11 -> GPU | ANGLE mature on Windows, used by Chrome/Electron |
| Desktop macOS | Metal (via ANGLE or native) | ANGLE -> Metal -> GPU | Apple deprecated OpenGL; ANGLE-Metal or native Metal |
| Desktop Linux | GL ES (EGL native) | Mesa/nvidia GL -> GPU | Native GL, no translation needed |

**Key decision: iOS uses native Metal, not ANGLE.**

Why:
- WeChat switched to native Metal on iOS in 2024 (version >= 8.0.63) for measurable perf gains
- ANGLE adds a translation layer (GL ES -> Metal) with per-call overhead
- Metal is the only first-class GPU API on Apple platforms
- Metal Shader Language compiles to GPU-native ISA; ANGLE must translate GLSL -> MSL at runtime
- Metal provides features like memoryless render targets, tile shading that GL ES cannot expose

Trade-off: iOS rendering code diverges from Android. But for a game engine where rendering is the hottest path, the performance gain justifies the cost.

### 2.3 Service Implementation

| Platform | FFI Mechanism | Service Backend |
|----------|---------------|-----------------|
| Android | JNI (Java <-> Rust) | Java Manager classes -> Android SDK |
| iOS | C ABI (Swift/ObjC <-> Rust) | Swift Manager classes -> Apple frameworks |
| Desktop | None (pure Rust) | Cross-platform Rust crates directly |

---

## 3. Crate Architecture

### 3.1 Crate Dependency Graph

```
                       +------------------+
                       |     shared/      |  protocol, error, vfs, services traits
                       +--------+---------+
                                |
              +-----------------+------------------+
              |                 |                   |
     +--------v-------+ +------v--------+ +--------v--------+
     |   graphics/    | |    audio/     | |      io/        |
     | (GL ES backend)| | (audio thread)| | (file, decode)  |
     +--------+-------+ +------+--------+ +--------+--------+
              |                 |                   |
     +--------v-------+        |                   |
     | graphics-metal/|        |                   |
     | (Metal backend)|        |                   |
     +--------+-------+        |                   |
              |                 |                   |
              +-----------------+------------------+
                                |
                    +-----------v-----------+
                    |        core/          |  host, command dispatch, services
                    +-----------+-----------+
                                |
              +-----------------+------------------+
              |                                    |
     +--------v---------+              +-----------v-----------+
     | js-runtime/      |              | js-runtime-jsc/       |
     | (deno_core + V8) |              | (JSC C API)           |
     | Android + Desktop |              | iOS only              |
     +--------+---------+              +-----------+-----------+
              |                                    |
     +--------v---------+              +-----------v-----------+
     | platform/android  |              | platform/ios          |
     | platform/desktop  |              | (Swift SDK + C ABI)   |
     +-------------------+              +-----------------------+
```

### 3.2 New Crate: `graphics-metal/`

For iOS native Metal rendering:

```
engine/crates/
  graphics/           # Existing: EGL + GL ES backend (Android, Desktop)
  graphics-metal/     # NEW: Metal backend (iOS, optionally macOS Desktop)
  graphics-common/    # NEW: Shared rendering types, command protocol
```

**`graphics-common/`** contains:
- Render command protocol (RenderCmd enum, already in `shared/protocol/render_cmd.rs`)
- Canvas state types (transform, clip, paint)
- Font/text layout interfaces
- `RenderBackend` trait that both GL and Metal backends implement

**`graphics/`** (GL backend):
- EGL initialization, GL context management
- femtovg Canvas2D (GL ES)
- glow-based WebGL ops
- Used by: Android, Desktop Linux, Desktop Windows

**`graphics-metal/`** (new):
- Metal device/command queue initialization
- Metal-based Canvas2D (femtovg Metal backend or custom)
- Metal-based WebGL-compatible ops (translate WebGL semantics to Metal)
- Used by: iOS, optionally Desktop macOS

### 3.3 New Crate: `js-runtime-jsc/`

For iOS JSC binding:

```
engine/crates/
  js-runtime/         # Existing: V8/deno_core (Android + Desktop)
  js-runtime-jsc/     # NEW: JSC C API (iOS)
  js-ops/             # NEW: Shared op business logic (proc macro generated)
```

**`js-ops/`** contains the platform-independent business logic of each op, extracted via
the `#[migo_op]` proc macro (see Section 5).

---

## 4. Bare Binding Architecture

### 4.1 What is Bare Binding

Traditional approach (current Migo, most engines):
```
JS call -> deno_core op dispatch -> Rust op fn -> serialize result -> JS callback
```

Bare binding approach (WeChat pattern, optimized Migo):
```
JS call -> direct native function pointer -> Rust op fn -> direct return to JS
```

For V8: **V8 Fast API Calls**
- Arguments passed as C types directly, no `v8::Value` boxing
- Return values written directly, no `v8::ReturnValue` indirection
- deno_core `#[op2(fast)]` already leverages this

For JSC: **JSC C API function callbacks**
- `JSObjectMakeFunctionWithCallback` for simple ops
- Direct `JSValueRef` manipulation, no wrapper overhead
- `JSTypedArrayGetBytesPtr` for zero-copy buffer access

### 4.2 V8 Fast API (Android + Desktop)

deno_core's `#[op2(fast)]` already leverages V8 Fast API. Key: ensure ALL hot-path ops
(especially WebGL draw calls) are fast-compatible:

```rust
// HOT PATH: WebGL bindBuffer - called thousands of times per frame
#[op2(fast)]
pub fn op_bindBuffer(state: &mut OpState, target: u32, buffer: u32) {
    let gl = state.borrow::<GlContext>();
    unsafe { gl.bind_buffer(target, if buffer == 0 { None } else { Some(buffer) }) };
}

// HOT PATH: WebGL drawElements
#[op2(fast)]
pub fn op_drawElements(state: &mut OpState, mode: u32, count: i32, type_: u32, offset: i32) {
    let gl = state.borrow::<GlContext>();
    unsafe { gl.draw_elements(mode, count, type_, offset as i32) };
}
```

Requirements for V8 Fast API eligibility:
- Parameters: numeric types, `*const u8`/`*mut u8` for buffers, `&[u8]`
- Return: numeric types or void
- No `#[string]` (UTF-8 validation overhead), no `#[serde]` (deserialization overhead)
- No `Result` with error (fast path cannot throw; use fallback slow path)

### 4.3 JSC Bare Binding (iOS)

```rust
// Same business logic, JSC binding generated by #[migo_op]
unsafe extern "C" fn op_bindBuffer_jsc(
    ctx: JSContextRef,
    _function: JSObjectRef,
    _this: JSObjectRef,
    argc: usize,
    argv: *const JSValueRef,
    _exception: *mut JSValueRef,
) -> JSValueRef {
    let state = get_op_state_from_jsc_ctx(ctx);
    let target = JSValueToNumber(ctx, *argv.add(0), std::ptr::null_mut()) as u32;
    let buffer = JSValueToNumber(ctx, *argv.add(1), std::ptr::null_mut()) as u32;
    js_ops::webgl::op_bindBuffer_impl(state, target, buffer);
    JSValueMakeUndefined(ctx)
}
```

### 4.4 Zero-Copy Buffer Transfer

For high-frequency data ops (vertex buffers, texture uploads, audio data):

**V8 (Android/Desktop):**
```rust
#[op2(fast)]
pub fn op_bufferData(
    state: &mut OpState,
    target: u32,
    #[buffer] data: &[u8],  // Zero-copy: points directly into V8 ArrayBuffer backing store
    usage: u32,
) {
    let gl = state.borrow::<GlContext>();
    unsafe { gl.buffer_data_u8_slice(target, data, usage) };
}
```

**JSC (iOS):**
```rust
// JSTypedArrayGetBytesPtr gives direct pointer to JSC ArrayBuffer backing store
let data_ptr = JSObjectGetTypedArrayBytesPtr(ctx, argv[1], std::ptr::null_mut());
let data_len = JSObjectGetTypedArrayByteLength(ctx, argv[1], std::ptr::null_mut());
let data = std::slice::from_raw_parts(data_ptr as *const u8, data_len);
metal_buffer_data(target, data, usage);
```

No copy in either path. JS `Float32Array`/`Uint8Array` backing memory is read directly by
the native GL/Metal call.

---

## 5. `#[migo_op]` Proc Macro: Shared Business Logic

### 5.1 Problem

326 ops need V8 binding (Android/Desktop) AND JSC binding (iOS). Writing each twice is
unmaintainable.

### 5.2 Solution: Compile-Time Code Generation

A proc macro `#[migo_op]` extracts business logic and generates platform-specific bindings
at compile time. **Zero runtime overhead** - all dispatch resolved at compile time.

```rust
// In js-ops/src/webgl.rs - shared business logic
#[migo_op(fast)]
pub fn op_bindBuffer(state: &mut OpState, target: u32, buffer: u32) {
    let gl = state.borrow::<GlContext>();
    unsafe { gl.bind_buffer(target, if buffer == 0 { None } else { Some(buffer) }) };
}
```

The macro generates:

**For V8 (in `js-runtime/`):**
```rust
#[op2(fast)]
pub fn op_bindBuffer(state: &mut OpState, target: u32, buffer: u32) {
    js_ops::webgl::op_bindBuffer_impl(state, target, buffer)
}
```

**For JSC (in `js-runtime-jsc/`):**
```rust
unsafe extern "C" fn op_bindBuffer_jsc(
    ctx: JSContextRef, _fn: JSObjectRef, _this: JSObjectRef,
    argc: usize, argv: *const JSValueRef, exc: *mut JSValueRef,
) -> JSValueRef {
    let state = get_op_state_from_jsc_ctx(ctx);
    let target = jsc_to_u32(ctx, *argv.add(0));
    let buffer = jsc_to_u32(ctx, *argv.add(1));
    js_ops::webgl::op_bindBuffer_impl(state, target, buffer);
    JSValueMakeUndefined(ctx)
}
```

### 5.3 Supported Parameter Types

| `#[migo_op]` Type | V8 (deno_core) | JSC C API |
|-------------------|----------------|-----------|
| `u32`, `i32`, `f64` | V8 Fast API numeric | `JSValueToNumber` cast |
| `bool` | V8 Fast API bool | `JSValueToBoolean` |
| `&[u8]` | `#[buffer]` zero-copy | `JSObjectGetTypedArrayBytesPtr` |
| `&str` / `String` | `#[string]` | `JSStringGetUTF8CString` |
| `()` return | void fast return | `JSValueMakeUndefined` |
| `Result<T, E>` | slow path with exception | set `*exception` |

### 5.4 Implementation Priority

- Phase 1: `fast` numeric-only ops (~60% of WebGL ops)
- Phase 2: `#[string]`, `#[buffer]`, `Result` types
- Phase 3: Async ops (`#[migo_op(async)]`)

The macro lives in `engine/crates/migo-macros/` as a proc-macro crate.

---

## 6. Rendering Architecture

### 6.1 Android: GL ES Direct (Current, Optimized)

Current pipeline is already close to optimal:

```
JS WebGL call -> #[op2(fast)] bare binding -> glow GL ES call -> EGL swap buffers
```

Optimizations to apply:
1. **Audit all WebGL ops for fast-compatibility**: Refactor any hot-path op using `#[string]` or `#[serde]` to use numeric parameters
2. **Batch state changes**: Minimize EGL `makeCurrent` calls (already mostly done)
3. **GL ES 3.0 where available**: Use VAO, instancing, MRT (check `GL_VERSION`)

### 6.2 iOS: Native Metal

```
JS WebGL call -> JSC bare binding -> Metal command encoder -> Metal commit + present
```

#### 6.2.1 WebGL-to-Metal Translation Layer

A thin, purpose-built Rust layer that maps only the WebGL subset we actually use (NOT full
ANGLE which translates the entire GL ES spec):

```rust
// engine/crates/graphics-metal/src/webgl_metal.rs
pub struct MetalWebGLContext {
    device: metal::Device,
    command_queue: metal::CommandQueue,
    current_pipeline: Option<metal::RenderPipelineState>,
    // state tracking for GL -> Metal mapping
}

impl MetalWebGLContext {
    pub fn bind_buffer(&mut self, target: u32, buffer: u32) { /* Metal buffer binding */ }
    pub fn bindTexture(&mut self, target: u32, texture: u32) { /* Metal texture */ }
    pub fn draw_elements(&mut self, mode: u32, count: i32, type_: u32, offset: i32) { /* Metal draw */ }
    pub fn flush(&mut self) { /* Commit Metal command buffer */ }
}
```

Why not ANGLE:
- ANGLE translates FULL GL ES spec (thousands of functions, edge cases)
- We only need WebGL 1.0 subset (~80 functions actually used by mini games)
- Custom translation can optimize for our specific usage patterns
- Eliminates ANGLE's ~4 MB binary overhead
- No GLSL -> MSL shader compilation at runtime (pre-compile or minimal translator)

#### 6.2.2 Shader Translation

WebGL uses GLSL. Metal uses MSL. Options:

1. **SPIRV-Cross** (recommended): GLSL -> SPIR-V (glslang) -> MSL (spirv-cross)
   - Mature, used by MoltenVK and many engines
   - Offline for known shaders, load-time for dynamic shaders
   - Rust bindings via `spirv-cross` crate

2. **Naga** (alternative): wgpu's shader translator, Rust-native
   - GLSL -> Naga IR -> MSL
   - Smaller binary than SPIRV-Cross
   - Less battle-tested for WebGL GLSL

3. **Runtime caching**: Translate once, cache MSL + compiled `MTLLibrary`

#### 6.2.3 Canvas2D on Metal

femtovg currently uses OpenGL. For iOS Metal:

1. **femtovg Metal backend**: femtovg has experimental Metal support. Use if mature
2. **Custom Canvas2D on Metal**: Minimal Canvas2D renderer using Metal directly
3. **Hybrid**: Small GL ES context via ANGLE only for Canvas2D (less perf-critical), native Metal for WebGL

Recommendation: Start with femtovg Metal, fall back to hybrid if immature.

### 6.3 Desktop

```
Desktop Windows: ANGLE (D3D11 backend) - same as Chrome
Desktop Linux:   Native EGL/GL ES (Mesa)
Desktop macOS:   ANGLE (Metal backend) or native Metal matching iOS
```

Desktop uses the same `graphics/` crate as Android. Variable is EGL library path:

```rust
fn egl_lib_path() -> &'static str {
    #[cfg(target_os = "android")]  { "libEGL.so" }
    #[cfg(target_os = "linux")]    { "libEGL.so.1" }
    #[cfg(target_os = "windows")]  { "libEGL.dll" }
    #[cfg(target_os = "macos")]    { "libEGL.dylib" }
}
```

---

## 7. JS Runtime Architecture

### 7.1 V8 Runtime (Android + Desktop) - `js-runtime/`

Continues using deno_core. Key optimizations:
1. All WebGL ops marked `#[op2(fast)]` - no exceptions in draw path
2. Minimize op count in hot path: batch GL state changes where beneficial
3. V8 Fast API for buffer ops: `#[buffer]` for direct ArrayBuffer access

### 7.2 JSC Runtime (iOS) - `js-runtime-jsc/`

Built on JSC C API (`JavaScriptCore/JavaScriptCore.h`):

```rust
// engine/crates/js-runtime-jsc/src/runtime.rs
pub struct JscRuntime {
    ctx: JSGlobalContextRef,
    vm: JSContextGroupRef,
    op_state: Rc<RefCell<OpState>>,
}

impl JscRuntime {
    pub fn new() -> Self {
        let vm = unsafe { JSContextGroupCreate() };
        let ctx = unsafe { JSGlobalContextCreateInGroup(vm, std::ptr::null_mut()) };
        let mut rt = Self { ctx, vm, op_state: Rc::new(RefCell::new(OpState::new())) };
        rt.register_ops();
        rt.load_js_modules();
        rt
    }

    fn register_ops(&mut self) {
        // Register all 326 ops as global functions via migo_op generated code
        self.register_fn("__op_bindBuffer", op_bindBuffer_jsc);
        self.register_fn("__op_drawElements", op_drawElements_jsc);
        // ...
    }

    fn load_js_modules(&mut self) {
        // Load bundled JS (ESM -> IIFE at build time)
    }
}
```

### 7.3 JS Module Loading

V8 (deno_core) uses ES modules natively. JSC C API does not support ES modules.

Solution: **Build-time JS bundling for iOS**

```
Source JS modules (ESM)
  -> esbuild/rollup bundle (build step)
    -> Single IIFE bundle with op stubs
      -> Loaded via JSEvaluateScript on iOS
```

The bundler replaces `import { op_xxx } from "ext:core/ops"` with references to the
globally registered `__op_xxx` functions.

---

## 8. Platform Layer

### 8.1 Android (`platform/android/`)

Unchanged from current architecture. JNI bridge, Java Manager classes.

Optimization: Ensure JNI calls in the render path are minimized. VSync signal delivery
should use the most efficient path possible.

### 8.2 iOS (`platform/ios/`)

```
engine/crates/platform/ios/
  mod.rs              # Platform entry, service wiring
  surface.rs          # CAMetalLayer wrapper (Surface trait impl)
  services/
    mod.rs            # All iOS service implementations
  ffi/
    mod.rs            # C ABI function exports for Swift
    inbound.rs        # Swift -> Rust callbacks
    outbound.rs       # Rust -> Swift calls (via C function pointers)
```

**Surface for iOS (Metal):**
```rust
pub struct IosSurfaceWrapper {
    metal_layer: *mut c_void,  // CAMetalLayer*
    dimension: (u32, u32),
}

impl Surface for IosSurfaceWrapper {
    fn size(&self) -> (u32, u32) { self.dimension }
    fn raw_window_handle(&self) -> RawWindowHandle {
        // UiKit window handle
        todo!()
    }
    fn raw_display_handle(&self) -> RawDisplayHandle {
        RawDisplayHandle::UiKit(UiKitDisplayHandle::new())
    }
}
```

**C ABI for Swift interop:**
```rust
#[no_mangle]
pub extern "C" fn migo_create_session(config_json: *const c_char) -> i32 { ... }
#[no_mangle]
pub extern "C" fn migo_destroy_session(session_id: i32) { ... }
#[no_mangle]
pub extern "C" fn migo_update_surface(
    session_id: i32, metal_layer: *mut c_void, width: u32, height: u32,
) { ... }
#[no_mangle]
pub extern "C" fn migo_on_vsync(session_id: i32, timestamp_ns: u64) { ... }
```

### 8.3 Desktop (`platform/desktop/`)

```
engine/crates/platform/desktop/
  mod.rs              # Platform entry
  surface.rs          # winit Window wrapper (Surface trait impl)
  services/
    mod.rs            # Pure Rust service implementations
  window.rs           # winit event loop integration
```

Pure Rust services, no FFI:

| Service | Rust Crate | Notes |
|---------|-----------|-------|
| Clipboard | `arboard` | Cross-platform |
| File dialogs | `rfd` | Native file picker |
| System info | `sysinfo` | CPU, memory, OS |
| Audio | `cpal` + `rodio` | Cross-platform audio |
| Networking | `tokio` (already used) | Same as Android |
| Storage | `std::fs` | Local file-based |

---

## 9. Canvas2D Strategy

| Platform | Canvas2D Backend | Notes |
|----------|-----------------|-------|
| Android | femtovg (GL ES) | Current, working well |
| iOS | femtovg (Metal) or hybrid | Depends on femtovg Metal maturity |
| Desktop | femtovg (GL ES) | Same as Android |

If femtovg Metal backend is not production-ready, Canvas2D on iOS can use a thin ANGLE
layer (Canvas2D is less perf-critical than WebGL for games) while WebGL uses native Metal.

---

## 10. Build System

### 10.1 Feature Flags

```toml
# engine/Cargo.toml workspace features
[workspace.features]
v8-engine = []       # V8/deno_core JS runtime
jsc-engine = []      # JSC JS runtime (iOS)
gl-backend = []      # OpenGL ES rendering
metal-backend = []   # Metal rendering (iOS)
desktop = []         # Desktop platform (winit)
```

### 10.2 Build Targets

```bash
# Android (unchanged)
cargo build --target aarch64-linux-android --features v8-engine,gl-backend

# iOS
cargo build --target aarch64-apple-ios --features jsc-engine,metal-backend

# Desktop Linux
cargo build --target x86_64-unknown-linux-gnu --features v8-engine,gl-backend,desktop

# Desktop Windows
cargo build --target x86_64-pc-windows-msvc --features v8-engine,gl-backend,desktop

# Desktop macOS
cargo build --target aarch64-apple-darwin --features v8-engine,gl-backend,desktop
```

### 10.3 iOS Build Pipeline

```bash
# Build Rust static library
cargo build --target aarch64-apple-ios --release --features jsc-engine,metal-backend

# Generate Swift header from cbindgen
cbindgen --config cbindgen.toml --output MigoEngine.h

# Build Swift SDK framework
xcodebuild -project MigoSDK.xcodeproj -scheme MigoSDK -configuration Release
```

---

## 11. Binary Size

| Component | Android | iOS | Desktop |
|-----------|---------|-----|---------|
| V8 engine | ~8 MB | N/A | ~8 MB |
| JSC engine | N/A | 0 MB (system) | N/A |
| ANGLE | N/A | N/A | ~3 MB (Win/Mac) |
| Metal backend | N/A | ~0.5 MB | N/A |
| GL backend | ~0.2 MB | N/A | ~0.2 MB |
| Shader translator | N/A | ~1 MB | N/A |
| Core engine | ~3 MB | ~3 MB | ~3 MB |
| **Total** | **~11 MB** | **~4.5 MB** | **~14 MB** |

iOS is smallest: system JSC (0 cost) + native Metal (no ANGLE).

---

## 12. Implementation Phases

### Phase 1: Foundation (Weeks 1-4)

**Goal: `#[migo_op]` macro + `graphics-common` + build infrastructure**

| Task | Description |
|------|-------------|
| Create `migo-macros` crate | Proc macro crate for `#[migo_op]` |
| Implement `#[migo_op(fast)]` for numeric ops | V8 code generation (wraps `#[op2(fast)]`) |
| Create `graphics-common` crate | Extract shared rendering types from `graphics/` |
| Refactor WebGL ops for fast-compatibility | Audit all 80+ WebGL ops, remove `#[string]`/`#[serde]` from hot paths |
| Set up feature flag build system | v8-engine, jsc-engine, gl-backend, metal-backend |

### Phase 2: iOS Core (Weeks 5-10)

**Goal: JSC runtime + Metal rendering = iOS prototype running a simple game**

| Task | Description |
|------|-------------|
| Create `js-runtime-jsc` crate | JSC C API wrapper, op registration, script evaluation |
| Implement JSC code gen in `#[migo_op]` | Generate JSC binding functions from same source |
| JS module bundler for iOS | Build-time ESM -> IIFE bundling |
| Create `graphics-metal` crate | Metal device init, command buffer management |
| WebGL-to-Metal translation | Map ~80 WebGL functions to Metal equivalents |
| GLSL-to-MSL shader compilation | Integrate spirv-cross or naga |
| Canvas2D on Metal | femtovg Metal backend or ANGLE-based fallback |
| Create `platform/ios` crate | Surface wrapper, C ABI exports, basic services |
| Swift SDK skeleton | Framework project, MigoEngine Swift API |

### Phase 3: iOS Feature Complete (Weeks 11-16)

**Goal: Full API coverage on iOS**

| Task | Description |
|------|-------------|
| Implement all 326 ops via `#[migo_op]` | Migrate remaining ops from V8-only to shared |
| iOS service implementations | Camera, location, bluetooth via Swift/ObjC |
| Audio on iOS | AVAudioEngine integration |
| Touch/input on iOS | UIKit gesture handling |
| iOS performance profiling | Instruments, Metal debugger |

### Phase 4: Desktop (Weeks 17-20)

**Goal: Desktop platform with V8 + GL ES**

| Task | Description |
|------|-------------|
| Create `platform/desktop` crate | winit window, event loop, pure Rust services |
| Desktop surface wrapper | winit Window -> Surface trait |
| Desktop EGL library loading | Platform-aware library paths |
| Desktop input handling | Keyboard, mouse -> touch event translation |
| Desktop audio | cpal/rodio integration |

### Phase 5: Polish (Weeks 21-24)

| Task | Description |
|------|-------------|
| Performance benchmarking | Cross-platform comparison, bottleneck identification |
| Android optimizations | Apply bare binding learnings back to Android ops |
| CI for all platforms | Build + test for Android, iOS, Desktop |
| Documentation | API docs, integration guides |

---

## 13. Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| WebGL-to-Metal translation complexity | High | Start with minimal subset, expand iteratively. Fallback: ANGLE on iOS for edge cases |
| GLSL-to-MSL shader compilation bugs | Medium | Use battle-tested spirv-cross. Cache compiled shaders aggressively |
| JSC C API limitations (no ES modules) | Medium | Build-time bundling is proven (React Native does this) |
| femtovg Metal backend immaturity | Medium | Fallback: ANGLE for Canvas2D only, native Metal for WebGL |
| `#[migo_op]` macro complexity | Medium | Start simple (numeric-only fast ops), expand incrementally |
| 326 ops migration effort | High | Prioritize by frequency: WebGL ops first, then API ops |

---

## 14. Performance Target

Based on WeChat's published metrics and industry benchmarks:

| Metric | Current (Android) | Target (Android opt.) | Target (iOS Metal) |
|--------|-------------------|----------------------|-------------------|
| WebGL draw call overhead | ~2us/call | ~0.5us/call (fast API) | ~0.5us/call (bare JSC) |
| Buffer upload (1MB) | ~0.8ms | ~0.1ms (zero-copy) | ~0.1ms (zero-copy) |
| Shader compile | N/A (GL native) | N/A | ~5ms (MSL, cached) |
| Canvas2D path render | ~0.5ms | ~0.5ms | ~0.4ms (Metal) |
| JS execution (compute) | 1x (V8 JIT) | 1x (V8 JIT) | ~0.8-1.2x (JSC JIT) |

---

## 15. Summary

This architecture follows WeChat's proven approach: **performance comes from eliminating
abstraction layers in the hot path, not from clever abstractions.**

Key decisions:
1. **Native Metal on iOS** (not ANGLE) - matches WeChat's latest direction
2. **Bare binding on V8 and JSC** - direct native function calls, no serialization
3. **Zero-copy buffer transfer** - JS ArrayBuffer memory shared directly with GL/Metal
4. **`#[migo_op]` compile-time code gen** - share business logic, zero runtime overhead
5. **Platform-native rendering** - each platform uses its optimal graphics API
6. **Willing to diverge** - iOS rendering code differs from Android, and that is the right trade-off
