#include <migo/platform/android.h>
#include <migo/platform/ios.h>
#include <migo/platform/macos.h>
#include <migo/platform/openharmony.h>
#include <migo/platform/wayland.h>
#include <migo/platform/win32.h>
#include <migo/platform/winui.h>
#include <migo/platform/x11.h>

#include <stddef.h>
#include <stdint.h>

#define MIGO_CHECK_PLATFORM_PREFIX(TYPE)                                      \
    _Static_assert(offsetof(TYPE, struct_size) == 0, #TYPE " size prefix");  \
    _Static_assert(offsetof(TYPE, abi_version) == 4, #TYPE " ABI prefix");   \
    _Static_assert(offsetof(TYPE, platform_kind) == 8, #TYPE " kind prefix"); \
    _Static_assert(offsetof(TYPE, flags) == 12, #TYPE " flags prefix")

MIGO_CHECK_PLATFORM_PREFIX(MigoAndroidNativeWindowDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoWin32HwndDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoWinuiSwapChainPanelDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoMacosNsViewDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoMacosMetalLayerDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoIosUiViewDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoIosMetalLayerDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoX11WindowDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoWaylandSurfaceDescriptor);
MIGO_CHECK_PLATFORM_PREFIX(MigoOpenHarmonyNativeWindowDescriptor);

_Static_assert(sizeof(MigoPlatformSurfaceDescriptor) == 16,
               "common platform prefix size");
_Static_assert(MIGO_PLATFORM_ANDROID_NATIVE_WINDOW == UINT32_C(1),
               "Android kind");
_Static_assert(MIGO_PLATFORM_WINUI_SWAP_CHAIN_PANEL == UINT32_C(3),
               "WinUI is not HWND");
_Static_assert(MIGO_PLATFORM_MACOS_NS_VIEW != MIGO_PLATFORM_MACOS_CA_METAL_LAYER,
               "macOS target kinds remain explicit");
/*
 * The four Apple records are byte-identical, so nothing but the kind tells a
 * retain of a UIView apart from a layer whose nextDrawable the renderer calls.
 * Six pairwise inequalities, not four declarations: a copy-paste that gave iOS
 * a macOS kind would still compile against the declarations alone.
 */
_Static_assert(MIGO_PLATFORM_IOS_UI_VIEW != MIGO_PLATFORM_IOS_CA_METAL_LAYER,
               "iOS target kinds remain explicit");
_Static_assert(MIGO_PLATFORM_IOS_UI_VIEW != MIGO_PLATFORM_MACOS_NS_VIEW,
               "UIView is not NSView");
_Static_assert(MIGO_PLATFORM_IOS_UI_VIEW != MIGO_PLATFORM_MACOS_CA_METAL_LAYER,
               "UIView is not a macOS layer");
_Static_assert(MIGO_PLATFORM_IOS_CA_METAL_LAYER != MIGO_PLATFORM_MACOS_NS_VIEW,
               "iOS layer is not NSView");
_Static_assert(MIGO_PLATFORM_IOS_CA_METAL_LAYER != MIGO_PLATFORM_MACOS_CA_METAL_LAYER,
               "the two CAMetalLayer kinds stay per-OS: the Host Kit that owns "
               "the layer differs, and so does the lifecycle around it");
_Static_assert(MIGO_PLATFORM_WAYLAND_SURFACE == UINT32_C(7), "Wayland kind");

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(MigoAndroidNativeWindowDescriptor) == 24,
               "LP64 Android descriptor");
_Static_assert(sizeof(MigoWin32HwndDescriptor) == 24,
               "LP64 Win32 descriptor");
_Static_assert(sizeof(MigoWinuiSwapChainPanelDescriptor) == 24,
               "LP64 WinUI descriptor");
_Static_assert(sizeof(MigoMacosNsViewDescriptor) == 24,
               "LP64 NSView descriptor");
_Static_assert(sizeof(MigoMacosMetalLayerDescriptor) == 24,
               "LP64 CAMetalLayer descriptor");
_Static_assert(sizeof(MigoIosUiViewDescriptor) == 24,
               "LP64 UIView descriptor");
_Static_assert(sizeof(MigoIosMetalLayerDescriptor) == 24,
               "LP64 iOS CAMetalLayer descriptor");
_Static_assert(sizeof(MigoX11WindowDescriptor) == 40,
               "LP64 X11 descriptor");
_Static_assert(sizeof(MigoWaylandSurfaceDescriptor) == 32,
               "LP64 Wayland descriptor");
_Static_assert(sizeof(MigoOpenHarmonyNativeWindowDescriptor) == 24,
               "LP64 OpenHarmony descriptor");
#elif UINTPTR_MAX == UINT32_MAX
_Static_assert(sizeof(MigoAndroidNativeWindowDescriptor) == 20,
               "ILP32 Android descriptor");
_Static_assert(sizeof(MigoWin32HwndDescriptor) == 20,
               "ILP32 Win32 descriptor");
_Static_assert(sizeof(MigoWinuiSwapChainPanelDescriptor) == 20,
               "ILP32 WinUI descriptor");
_Static_assert(sizeof(MigoMacosNsViewDescriptor) == 20,
               "ILP32 NSView descriptor");
_Static_assert(sizeof(MigoMacosMetalLayerDescriptor) == 20,
               "ILP32 CAMetalLayer descriptor");
/*
 * No shipping Apple target is ILP32 -- armv7 is gone and every current slice is
 * LP64. These two still belong in the 32-bit lane: this file is compiled for
 * both widths, and an assertion that only ever gets compiled on one of them is
 * an assertion nobody has run. That is the failure recorded for the ILP32 lane
 * in this repo before -- branches written twice and verified once.
 */
_Static_assert(sizeof(MigoIosUiViewDescriptor) == 20,
               "ILP32 UIView descriptor");
_Static_assert(sizeof(MigoIosMetalLayerDescriptor) == 20,
               "ILP32 iOS CAMetalLayer descriptor");
_Static_assert(sizeof(MigoX11WindowDescriptor) == 32,
               "ILP32 X11 descriptor");
_Static_assert(sizeof(MigoWaylandSurfaceDescriptor) == 24,
               "ILP32 Wayland descriptor");
_Static_assert(sizeof(MigoOpenHarmonyNativeWindowDescriptor) == 20,
               "ILP32 OpenHarmony descriptor");
#else
#error "unsupported pointer width"
#endif

int migo_platform_c_contract(void) {
    MigoAndroidNativeWindowDescriptor android = {0};
    MigoWin32HwndDescriptor win32 = {0};
    MigoWinuiSwapChainPanelDescriptor winui = {0};
    MigoMacosNsViewDescriptor ns_view = {0};
    MigoMacosMetalLayerDescriptor metal_layer = {0};
    MigoIosUiViewDescriptor ui_view = {0};
    MigoIosMetalLayerDescriptor ios_layer = {0};
    MigoX11WindowDescriptor x11 = {0};
    MigoWaylandSurfaceDescriptor wayland = {0};
    MigoOpenHarmonyNativeWindowDescriptor openharmony = {0};

    android.platform_kind = MIGO_PLATFORM_ANDROID_NATIVE_WINDOW;
    win32.platform_kind = MIGO_PLATFORM_WIN32_HWND;
    winui.platform_kind = MIGO_PLATFORM_WINUI_SWAP_CHAIN_PANEL;
    ns_view.platform_kind = MIGO_PLATFORM_MACOS_NS_VIEW;
    metal_layer.platform_kind = MIGO_PLATFORM_MACOS_CA_METAL_LAYER;
    ui_view.platform_kind = MIGO_PLATFORM_IOS_UI_VIEW;
    ios_layer.platform_kind = MIGO_PLATFORM_IOS_CA_METAL_LAYER;
    x11.platform_kind = MIGO_PLATFORM_X11_WINDOW;
    wayland.platform_kind = MIGO_PLATFORM_WAYLAND_SURFACE;
    openharmony.platform_kind = MIGO_PLATFORM_OPENHARMONY_NATIVE_WINDOW;

    return (int)(android.platform_kind + win32.platform_kind +
                 winui.platform_kind + ns_view.platform_kind +
                 metal_layer.platform_kind + ui_view.platform_kind +
                 ios_layer.platform_kind + x11.platform_kind +
                 wayland.platform_kind + openharmony.platform_kind);
}
