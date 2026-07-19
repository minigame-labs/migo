# Migo C ABI and Surface v1 candidate

These headers are a design candidate. They make Migo's planned low-level embedding contract reviewable from C11 and C++17. On desktop Linux they are also callable: `scripts/build-linux-sdk.sh` produces `libmigo.so` and `libmigo.a` exporting exactly the `migo_*` set declared here, with pkg-config and CMake integration. Everywhere else the headers remain compile-only.

The public markers are intentional:

```c
MIGO_C_ABI_CANDIDATE  == 1     /* still a candidate everywhere */
MIGO_C_ABI_HAS_RUNTIME == 1    /* desktop Linux: a linkable runtime exists */
MIGO_C_ABI_HAS_RUNTIME == 0    /* Android and every other target */
```

A runtime existing is not the same as the ABI being frozen. Do not treat these headers as a stable SDK: the freeze blockers below are open, and the surface may still change. Android continues to use the existing Java/JNI SDK and exports no `migo_*` symbols.

## Header layout

- `migo.h` is the engine/session/Surface umbrella header.
- `types.h`, `session.h`, and `surface.h` contain only standard C types.
- `platform/*.h` contains strongly typed native target descriptors without including any platform SDK header.
- iOS has no native Surface descriptor because its planned default backend is an embeddable `WKWebView` Host Kit.

The generic descriptor is not a universal native-window union. It points to one typed Android, Win32, WinUI, macOS, X11, Wayland, or OpenHarmony descriptor, preserving the platform's best native integration model.

## Structure initialization and extension

Every caller-owned extensible structure starts with `struct_size` and `abi_version`. Callers zero-initialize the complete record, then set both fields explicitly:

```c
MigoSurfaceDescriptor descriptor = {0};
descriptor.struct_size = (uint32_t)sizeof(descriptor);
descriptor.abi_version = MIGO_ABI_VERSION_1;
```

A future implementation copies every retained structure, rejects an unsupported version or undersized required prefix, and ignores unknown trailing fields. Every reserved field must remain zero when supplied by a caller and must be written as zero by an implementation; this keeps the bytes available for a compatible future meaning. Descriptor pointers are borrowed only for the duration of the API call. Public enum-like values use fixed-width integer typedefs and numeric macros; the headers deliberately use neither C enums nor packing pragmas.

`MigoSurfaceDescriptor.platform_descriptor_size` deliberately duplicates the typed payload's `struct_size`. A receiver compares both before reading the payload, so the envelope cannot claim a larger readable range than the payload reports for itself.

## Surface generation and completion

The host chooses a non-zero generation that increases monotonically within a Session. Only one attachment is active at a time. Resize/DPI/color/presentation updates repeat the generation; a stale generation is rejected. Replacing a native target always means synchronous detach followed by attach with a newer generation.

`MigoSurfaceAttachment*` is a unique handle; callers must not create independently owned aliases. Detach is a cold-path completion boundary. After `migo_surface_detach` returns `MIGO_OK`, the handle has been consumed and released, its pointer is invalid, no future GPU call or present may reference that generation, and the host may destroy its borrowed native resources. A non-success result leaves ownership with the caller for retry or Session destruction. Destroying the Session consumes any remaining live attachment and invalidates all caller-held attachment pointers. This avoids retaining one tombstone object for every Surface recreation.

A future implementation can marshal detach to Migo-owned render/platform workers but cannot require an SDK-owned window or event loop, and it must not wait for another turn of the host dispatcher. When required platform-thread affinity is not satisfied, detach returns `MIGO_ERROR_WRONG_THREAD` before retiring the generation or changing handle ownership. Consequently a callback running on a single-threaded UI dispatcher can detach reentrantly without blocking on its own queue.

## Native target ownership

| Target | Lifetime rule |
|---|---|
| Android `ANativeWindow*` | Migo acquires a strong reference before attach succeeds and releases it before detach returns. |
| OpenHarmony `OHNativeWindow*` | Migo takes/releases a native-object reference around the attachment. |
| macOS `NSView*` / `CAMetalLayer*` | Migo retains/releases the Objective-C object; the two target kinds remain distinct. |
| WinUI native SwapChainPanel interface | Migo takes/releases its own COM reference; this is not modeled as an HWND. |
| Win32 child `HWND` | Host-owned and valid through detach; Migo neither destroys it nor owns the message loop. |
| X11 `Display*` + `Window` | Host-owned and valid through detach; Migo does not close/destroy them. |
| Wayland `wl_display*` + `wl_surface*` | Host-owned and valid through detach; the host owns the role and dispatch loop. |

## Dispatcher, callbacks, and destruction

A callback record is copied according to `struct_size`; it is never borrowed. Callback configuration can be installed only once per Session and must be installed before the first Surface attach or transition to `MIGO_LIFECYCLE_RUNNING`; later calls return `MIGO_ERROR_INVALID_STATE`. This eliminates replacement races for queued callback function pointers and `user_data`. Any non-null user callback requires a non-null dispatcher. The dispatcher can be entered from an engine worker, must be thread-safe, and returns promptly. Returning `MIGO_OK` accepts exactly one task invocation; a rejection leaves ownership with Migo.

User callbacks execute only inside the dispatched task, with no Migo engine/session/attachment lock held. They may re-enter lifecycle, visibility, focus, detach, or destroy. Session destruction cancels queued user callbacks. A queued internal task may later run only to release its own storage; it cannot touch `user_data` after destruction. Reentrant destruction invalidates the Session immediately and permits only the current callback stack to unwind.

Successful `migo_session_destroy` and `migo_engine_destroy` calls consume and release their respective handles; those pointers are invalid afterward. All child Sessions must be destroyed before their Engine.

## Performance boundary

Descriptors are parsed and converted once during attach/update control operations. This contract adds no per-frame virtual dispatch, allocation, serialization, native-handle conversion, or callback hop. Presenter selection is fixed after attach; future platform backends remain free to use their best zero-copy graphics path.

## ABI v1 freeze blockers

The candidate cannot be declared stable until all of the following exist:

- performance-oriented batched pointer/touch, keyboard/text/IME, and gamepad contracts —
  **pointer/touch done** (`migo_session_send_touch`: batched, one copy at the boundary, no
  allocation, sharing the engine path Android already drives); keyboard/text/IME and gamepad
  **open**;
- asynchronous request IDs, cancellation races, and late-completion rules — **open**;
- capability and supported-structure/version queries — **open**;
- Android and Linux implementations using this same contract — **Linux done, Android open**; the ABI crate is desktop-only today;
- export lists, symbol/version tests, old-client/new-library tests, and per-target ILP32/LP64 layout lanes — **export list and symbol/version tests done** for `linux-x86_64` (`scripts/test-linux-sdk-contract.sh`); old-client/new-library lanes **open**;
- Android/Linux compatibility and performance gates with no material regression — **Linux compatibility gate done**, the rest open.

The Linux artifact contract that is in place is described by
`dist/migo-linux-x86_64/share/migo/linux-x86_64-manifest.json`: target triple, CPU
baseline, glibc/GLIBCXX floor, sysroot, dynamic dependencies, and the exact V8
revision and GN arguments the archive was built from.

Compile the current consumer contract with:

```sh
bash scripts/test-c-abi-surface-candidate.sh
```

This test compiles only small C/C++ fixtures. It does not build or link Migo, Android native code, or V8.

CI also compiles the same layout contract with the Android API 26 ARMv7 toolchain to exercise ARM ILP32 alignment. That lane is layout-only and does not publish or claim support for an `armeabi-v7a` runtime artifact; official Android artifacts remain `arm64-v8a` and `x86_64` unless a separate product decision adds ARMv7.
