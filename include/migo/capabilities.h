/*
 * Migo C ABI: what the linked library supports.
 *
 * Everything else here describes what a host wants to do. This describes what
 * the library can do, which a host has no other way to establish:
 * MIGO_C_ABI_HAS_RUNTIME is a preprocessor macro and reports the platform the
 * host compiled on, not the library it linked; and until now the attachable
 * surface kinds could only be discovered by attempting an attach, after
 * building an engine, a session and a window.
 */
#ifndef MIGO_CAPABILITIES_H
#define MIGO_CAPABILITIES_H

#include <migo/types.h>

MIGO_BEGIN_DECLS

typedef struct MigoCapabilities {
    /* Set by the caller to sizeof(MigoCapabilities) before the call. */
    uint32_t struct_size;
    /* Set by the caller to the ABI version it was built against. Unlike every
     * other entry point, an unrecognised value here is answered, not rejected
     * -- see migo_query_capabilities. */
    uint32_t abi_version;

    /* The range of ABI versions this library accepts on its entry points. */
    uint32_t abi_version_min;
    uint32_t abi_version_max;

    /* Bit N is set when MIGO_PLATFORM_* value N can be attached by this build.
     * A bitmask rather than an array: the values are a small dense enum, and an
     * array would raise a lifetime question for nothing. */
    uint64_t platform_kinds;
} MigoCapabilities;

/*
 * Report what this library supports. Takes no handle: this answers what a host
 * needs to know before it commits to creating anything.
 *
 * This is the one entry point that does not reject an ABI version it does not
 * recognise. A caller asking which versions exist has by definition not
 * established that yet, and refusing the question would leave it with no way to
 * find the answer. struct_size is still validated, because it governs how many
 * bytes may be written into the caller's storage; on rejection nothing is
 * written at all, so a failed query cannot be mistaken for a successful one
 * that reported nothing.
 *
 * The reported platform_kinds is the same fact migo_session_attach_surface
 * enforces, not a second copy of it.
 */
MIGO_API MigoResult MIGO_CALL migo_query_capabilities(MigoCapabilities *out);

MIGO_END_DECLS

#endif /* MIGO_CAPABILITIES_H */
