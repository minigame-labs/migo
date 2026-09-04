#include <migo/platform/android.h>
#include <migo/platform/ios.h>
#include <migo/platform/macos.h>
#include <migo/platform/openharmony.h>
#include <migo/platform/wayland.h>
#include <migo/platform/win32.h>
#include <migo/platform/winui.h>
#include <migo/platform/x11.h>

#include <cstddef>
#include <type_traits>

#define MIGO_CHECK_PLATFORM_RECORD(TYPE)                                      \
    static_assert(std::is_standard_layout<TYPE>::value, #TYPE " standard layout"); \
    static_assert(std::is_trivially_copyable<TYPE>::value,                    \
                  #TYPE " trivially copyable");                              \
    static_assert(offsetof(TYPE, struct_size) == 0, #TYPE " size prefix");   \
    static_assert(offsetof(TYPE, abi_version) == 4, #TYPE " ABI prefix");    \
    static_assert(offsetof(TYPE, platform_kind) == 8, #TYPE " kind prefix"); \
    static_assert(offsetof(TYPE, flags) == 12, #TYPE " flags prefix")

MIGO_CHECK_PLATFORM_RECORD(MigoAndroidNativeWindowDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoWin32HwndDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoWinuiSwapChainPanelDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoMacosNsViewDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoMacosMetalLayerDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoIosUiViewDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoIosMetalLayerDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoX11WindowDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoWaylandSurfaceDescriptor);
MIGO_CHECK_PLATFORM_RECORD(MigoOpenHarmonyNativeWindowDescriptor);

static_assert(!std::is_same<MigoWin32HwndDescriptor,
                            MigoWinuiSwapChainPanelDescriptor>::value,
              "WinUI keeps a dedicated native contract");
static_assert(!std::is_same<MigoMacosNsViewDescriptor,
                            MigoMacosMetalLayerDescriptor>::value,
              "AppKit view and Metal layer remain explicit");
static_assert(!std::is_same<MigoIosUiViewDescriptor,
                            MigoIosMetalLayerDescriptor>::value,
              "UIKit view and Metal layer remain explicit");
static_assert(!std::is_same<MigoIosUiViewDescriptor,
                            MigoMacosNsViewDescriptor>::value,
              "UIView and NSView are different ABIs with the same layout");
static_assert(!std::is_same<MigoIosMetalLayerDescriptor,
                            MigoMacosMetalLayerDescriptor>::value,
              "the CAMetalLayer descriptors stay per-OS");

int migo_platform_cpp_contract() {
    MigoWaylandSurfaceDescriptor wayland{};
    MigoX11WindowDescriptor x11{};
    MigoIosMetalLayerDescriptor ios_layer{};
    wayland.platform_kind = MIGO_PLATFORM_WAYLAND_SURFACE;
    x11.platform_kind = MIGO_PLATFORM_X11_WINDOW;
    ios_layer.platform_kind = MIGO_PLATFORM_IOS_CA_METAL_LAYER;
    return static_cast<int>(wayland.platform_kind + x11.platform_kind +
                            ios_layer.platform_kind);
}
