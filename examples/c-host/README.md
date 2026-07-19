# Embedding migo from C

One example, two platforms. Both hosts do the same thing — create an engine and
a session, attach the window they own, load content, and forward input — and
both see nothing but the headers under `include/migo`. If either ever needs
something outside those headers, the ABI is incomplete, and finding that out is
the reason these exist.

| | |
|---|---|
| `linux/` | An X11 host. Built with plain `cc` and `pkg-config`, or with CMake through `find_package(migo)`. |
| `android/` | A NativeActivity host. No Java at all: the manifest declares `android:hasCode="false"` and names this library's `ANativeActivity_onCreate`. |
| `touch-probe/` | Content shared by both, for verifying that input arrives. |

The Android module lives here rather than under `platforms/android/` because it
is a *consumer* of what that tree ships, not part of the product. Gradle picks
it up through `projectDir` in `platforms/android/settings.gradle`.

## Linux

```sh
bash scripts/build-linux-sdk.sh              # stages dist/migo-linux-x86_64
bash examples/c-host/linux/build-with-pkgconfig.sh
./examples/c-host/linux/c-host <files-dir> <content-id> <seconds>
```

## Android

```sh
bash scripts/build-android-c-host.sh         # cross-compiles capi, builds the APK
adb install -r examples/c-host/android/build/outputs/apk/debug/*.apk
```

Content goes where the engine resolves it, under the app's own files directory:
`<files>/migo/games/<content-id>/code/{game.json,game.js}`.

## What each platform proves that the other cannot

The Linux host maps one mouse to one touch point, so it cannot exercise the
multi-pointer path at all. The Android host carries every pointer from
`AMotionEvent_getPointerCount`, which is the only place `MIGO_TOUCH_MAX_POINTS`
and the per-pointer `CHANGED`/`REMOVED` flags are tested.

Android is also the only place the lifecycle contract runs for real: nothing on
a desktop takes the application away, so `migo_session_set_visibility` and the
detach/attach cycle only meet genuine pause, resume and window loss here.
