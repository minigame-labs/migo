#ifndef MIGO_PLATFORM_WINUI_H_
#define MIGO_PLATFORM_WINUI_H_

#include <migo/surface.h>

/*
 * swap_chain_panel_native is an ISwapChainPanelNative-compatible COM pointer,
 * not an HWND. A future implementation takes its own COM reference before
 * attach returns and releases it during detach.
 */
typedef struct MigoWinuiSwapChainPanelDescriptor {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoPlatformKind platform_kind;
    MigoPlatformDescriptorFlags flags;
    void *swap_chain_panel_native;
} MigoWinuiSwapChainPanelDescriptor;

#endif /* MIGO_PLATFORM_WINUI_H_ */
