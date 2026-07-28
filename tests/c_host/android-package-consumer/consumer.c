/*
 * A third-party Android NDK consumer of the Migo C ABI package.
 *
 * It exists to prove the package is linkable exactly the way a real host links
 * it: through find_package(migo) with the NDK toolchain, seeing only the public
 * headers, resolving migo_* against the packaged static library. It is compiled
 * and linked into a shared object; that link succeeding -- no undefined migo_*,
 * EGL or GLES symbol, no C++ runtime conflict -- is the whole test.
 *
 * It deliberately does not run: running needs a device and a window. Linking is
 * what a packaging bug breaks, and linking is checked here without either.
 */

#include <migo/migo.h>

/* Referenced by the linker via the version script, so the whole object is kept
 * and every migo_* call below must resolve. */
int migo_consumer_probe(void);

int migo_consumer_probe(void) {
    MigoEngineConfig config = {0};
    config.struct_size = (uint32_t)sizeof(config);
    config.abi_version = MIGO_ABI_VERSION_CURRENT;

    MigoEngine *engine = NULL;
    MigoResult result = migo_engine_create(&config, &engine);
    if (result != MIGO_OK) {
        return (int)result;
    }

    /* Touch the capability query too: it is the entry point a host calls first
     * to learn what the linked library supports. */
    MigoCapabilities caps = {0};
    caps.struct_size = (uint32_t)sizeof(caps);
    caps.abi_version = MIGO_ABI_VERSION_CURRENT;
    (void)migo_query_capabilities(&caps);

    (void)migo_engine_destroy(engine);
    return 0;
}
