#!/usr/bin/env bash
# Local source contract for the OpenHarmony N-API host.  The OpenHarmony SDK is
# not required: these checks protect the lifecycle invariants that are easiest
# to lose in a platform-only build.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/platforms/openharmony/entry/src/main/cpp/napi_init.cpp"
CMAKE="$ROOT/platforms/openharmony/entry/src/main/cpp/CMakeLists.txt"
PROFILE="$ROOT/platforms/openharmony/entry/build-profile.json5"
PAGE="$ROOT/platforms/openharmony/entry/src/main/ets/pages/Index.ets"
ABILITY="$ROOT/platforms/openharmony/entry/src/main/ets/entryability/EntryAbility.ets"

python3 - "$SOURCE" "$CMAKE" "$PROFILE" "$PAGE" "$ABILITY" <<'PY'
from pathlib import Path
import re
import sys

source_path, cmake_path, profile_path, page_path, ability_path = map(Path, sys.argv[1:])
source = source_path.read_text(encoding="utf-8")
cmake = cmake_path.read_text(encoding="utf-8")
profile = profile_path.read_text(encoding="utf-8")
page = page_path.read_text(encoding="utf-8")
ability = ability_path.read_text(encoding="utf-8")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"OpenHarmony host contract failed: {message}")


def function_body(name: str) -> str:
    match = re.search(r"\b" + re.escape(name) + r"\s*\([^)]*\)\s*\{", source)
    require(match is not None, f"missing {name} definition")
    start = match.end() - 1
    depth = 0
    for index in range(start, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start + 1:index]
    raise SystemExit(f"OpenHarmony host contract failed: unterminated {name}")


detach = function_body("detach_surface")
start = function_body("Start")
stop = function_body("Stop")
foreground = function_body("SetForeground")
foreground_flow = foreground + function_body("apply_foreground_state")

require("#include <mutex>" in source, "Host lifecycle state is not synchronized")
require("std::mutex" in source, "Host lifecycle state is not synchronized")
require("MigoSurfaceRelease *release" in source, "release observer is not retained")
require("release_callback_seen" in source, "callback-before-store is not recorded")
require("release_generation" in source, "release generation is not retained")
require("on_surface_released" in source, "surface release callback is not installed")
require("migo_surface_release_query" in source, "release callback does not query authority")
require("migo_surface_release_destroy" in source, "completed release observer is not destroyed")
require("for (;;)" not in detach and "while (true)" not in detach,
        "detach_surface still busy-waits on the UI callback")
require("release_callback_seen" in detach or "try_finalize_surface_release" in source,
        "detach path does not handle callback-before-observer publication")

require("StartState" in source, "Start has no one-shot lifecycle state")
require("MIGO_ERROR_INVALID_STATE" in start, "duplicate Start is not rejected")
require("migo_session_destroy" in start or "rollback_start" in start,
        "Start failure does not roll back Session")
require("migo_engine_destroy" in start or "rollback_start" in start,
        "Start failure does not roll back Engine")
require("callbacks.user_data" in start, "callbacks do not identify the synchronized Host")
require("callbacks.on_surface_released" in start,
        "release wakeup callback is not registered")
require("std::isfinite" in start and "g_host.scale_factor" in start,
        "Start does not validate and retain the device scale factor")
require("densityPixels" in page and
        "migohost.start(filesDir, cacheDir, CONTENT_ID, scaleFactor)" in page,
        "ArkTS does not pass the current display density to the native host")
require("3.0f" not in source,
        "host still hard-codes a device-specific scale factor")
require("callbacks.dispatch = dispatch_to_arkts" in start,
        "callbacks are not marshalled onto the ArkTS event loop")
require("napi_create_threadsafe_function" in source and
        "napi_call_threadsafe_function" in source and
        "napi_tsfn_nonblocking" in source,
        "host dispatcher is not a bounded non-blocking N-API TSFN")
require("dispatch_inline" not in source,
        "engine-worker inline callback dispatcher remains")
require("attach_pending_surface_if_ready" in source,
        "release completion does not reattach a waiting Surface")
surface_changed = function_body("OnSurfaceChangedCB")
require("migo_surface_update" in surface_changed,
        "Surface size changes are not forwarded through the public C ABI")
surface_destroyed = function_body("OnSurfaceDestroyedCB")
require("pending_component == component" in surface_destroyed and
        "pending_window == window" in surface_destroyed and
        "pending_component = nullptr" in surface_destroyed and
        "pending_window = nullptr" in surface_destroyed,
        "destroyed pre-start Surface remains queued for a later attach")
require("MigoResult detach_surface" in source,
        "surface detach cannot report pending/failure state to Stop")
require("finish_stop_if_ready" in source,
        "Stop has no release-aware teardown completion path")
require("migo_session_destroy" in stop or "finish_stop_if_ready" in stop,
        "Stop never drives Session teardown")
require("migo_engine_destroy" in source,
        "Stop never drives the final Engine thread barrier")
require(source.find("migo_session_destroy", source.find("finish_stop_if_ready")) <
        source.find("migo_engine_destroy", source.find("finish_stop_if_ready")),
        "Stop does not destroy Session before Engine")
require("MIGO_ERROR_WOULD_BLOCK" in stop,
        "asynchronous Stop does not expose its retryable pending state")
require('{"stop", nullptr, Stop' in source,
        "native module does not export Stop")

require("migo_session_set_lifecycle" in foreground_flow and
        "migo_session_set_visibility" in foreground_flow and
        "migo_session_set_focus" in foreground_flow,
        "foreground API does not drive all three Session state signals")
require('{"setForeground", nullptr, SetForeground' in source,
        "native module does not export foreground state")
require("migohost.setForeground(foreground)" in ability and
        "this.setHostForeground(true)" in ability and
        "this.setHostForeground(false)" in ability,
        "Ability foreground/background events do not reach the Session")
require("migohost.stop()" in page and ".onDestroy(" in page,
        "XComponent destruction does not initiate native teardown")
require("(void)component;\n    (void)component;" not in source,
        "duplicate component suppression remains")

require(re.search(r"config\.flags\s*=\s*0\s*;", source) is not None,
        "production engine flags are not explicitly zero")
require("MIGO_OHOS_ALLOW_UNSIGNED_CONTENT" in source,
        "unsigned-content opt-in is not compile-time gated")
unsigned_assignments = re.findall(
    r"config\.flags\s*=\s*MIGO_ENGINE_FLAG_ALLOW_UNSIGNED_CONTENT\s*;", source
)
require(len(unsigned_assignments) == 1 and "#if" in source,
        "unsigned content is enabled outside an explicit debug define")

require("MIGO_OHOS_ALLOW_UNSIGNED_CONTENT" in cmake,
        "CMake has no explicit unsigned debug option")
require("target_compile_definitions" in cmake,
        "CMake does not wire the unsigned debug define")
require("CMAKE_BUILD_TYPE" in cmake and "Debug" in cmake,
        "CMake does not reject the unsigned define outside Debug")

debug = re.search(r'"name"\s*:\s*"debug"(?P<body>.*?)(?=\n\s*\},)', profile, re.S)
release = re.search(r'"name"\s*:\s*"release"(?P<body>.*?)(?=\n\s*\},)', profile, re.S)
require(debug is not None and "MIGO_OHOS_ALLOW_UNSIGNED_CONTENT=1" in debug.group("body"),
        "debug build profile does not explicitly opt into unsigned content")
require(release is not None and "MIGO_OHOS_ALLOW_UNSIGNED_CONTENT=1" not in release.group("body"),
        "release build profile enables unsigned content")

print("OpenHarmony host lifecycle/flags contract: PASS")
PY
