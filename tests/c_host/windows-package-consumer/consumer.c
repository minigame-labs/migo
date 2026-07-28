/*
 * A third-party MSVC consumer of the Migo Windows SDK package.
 *
 * It exists to prove the package is consumable exactly the way a real host
 * consumes it: seeing only the public headers under include/migo, linking the
 * packaged import library (migo.lib), and resolving every migo_* call against
 * it -- no undefined symbol, no C++ runtime conflict.
 *
 * Unlike the Android package consumer, which cannot run without a device and a
 * window and therefore treats *linking* as the whole test, this one also runs:
 * on Windows the packaged migo.dll and its runtime DLLs load in-process, so
 * calling migo_engine_create / migo_query_capabilities / migo_engine_destroy
 * additionally proves the DLL loads and the ABI is callable. It does NOT create
 * a surface or load content -- that needs a window and is the render proof, a
 * separate step.
 */

#include <migo/migo.h>
#include <stdio.h>

int main(void) {
    MigoEngineConfig config = {0};
    config.struct_size = (uint32_t)sizeof(config);
    config.abi_version = MIGO_ABI_VERSION_CURRENT;
    /* Storage roots belong to the host: Migo writes only underneath these and
     * creates them if missing. A real host would name its app-data directory;
     * a consumer smoke test just needs three writable paths, so it uses ones
     * relative to the working directory. Leaving them NULL is a host bug and
     * the engine rightly rejects it with MIGO_ERROR_INVALID_ARGUMENT. */
    config.files_dir_utf8 = "migo-consumer-data/files";
    config.cache_dir_utf8 = "migo-consumer-data/cache";
    config.code_cache_dir_utf8 = "migo-consumer-data/code-cache";

    MigoEngine *engine = NULL;
    MigoResult result = migo_engine_create(&config, &engine);
    if (result != MIGO_OK) {
        printf("migo_engine_create failed: %d\n", (int)result);
        return (int)result;
    }

    MigoCapabilities caps = {0};
    caps.struct_size = (uint32_t)sizeof(caps);
    caps.abi_version = MIGO_ABI_VERSION_CURRENT;
    (void)migo_query_capabilities(&caps);

    (void)migo_engine_destroy(engine);
    printf("migo windows package consumer: OK "
           "(linked migo.lib, loaded migo.dll, engine create/query/destroy)\n");
    return 0;
}
