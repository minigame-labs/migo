/*
 * The mirror image of old_client_contract.c, for the structs the library WRITES
 * rather than the ones it reads: MigoCapabilities and MigoSurfaceReleaseStatus.
 *
 * The append rule is symmetric. An input struct is copied at the caller's
 * struct_size so a short old client is accepted; an output struct is written at
 * no more than the caller's struct_size so a short old client is not overrun.
 * Both directions only work if growth is append-only, and this lane is what
 * pins that for the output side.
 *
 * As on the input side, the shapes below are declared here rather than included.
 * Neither output struct has grown yet, so today each declaration is the current
 * shape -- but declaring it independently, in a fixed field order, is what
 * catches a field *swap* between two same-type fields. MigoCapabilities has two
 * adjacent uint32_t (abi_version_min, abi_version_max): exchanging their
 * declarations leaves sizeof and the header offsets unchanged, so no size check
 * and no header pin sees it, yet the library would then report its min as its
 * max. The per-field offset asserts below are the only thing that catches it.
 *
 * The runtime half -- that the library writes no more than the caller's
 * struct_size and leaves an old client's absent tail untouched -- lives in
 * migo_capi_abi's output_prefix tests, against a grown struct with a poisoned
 * trailing field.
 */

#include <migo/migo.h>

#include <stddef.h>
#include <stdint.h>

/* MigoCapabilities as a v1 client knows it: through platform_kinds. */
typedef struct V1Capabilities {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t abi_version_min;
    uint32_t abi_version_max;
    uint64_t platform_kinds;
} V1Capabilities;

/* MigoSurfaceReleaseStatus as a v1 client knows it: through reserved0. */
typedef struct V1ReleaseStatus {
    uint32_t struct_size;
    uint32_t abi_version;
    uint64_t generation;
    uint32_t state;
    uint32_t reserved0;
} V1ReleaseStatus;

#define MIGO_OUT_SAME_OFFSET(OLD, CUR, field)                                 \
    _Static_assert(offsetof(OLD, field) == offsetof(CUR, field),              \
                   #CUR "." #field " moved; library-written fields must only append")

MIGO_OUT_SAME_OFFSET(V1Capabilities, MigoCapabilities, struct_size);
MIGO_OUT_SAME_OFFSET(V1Capabilities, MigoCapabilities, abi_version);
MIGO_OUT_SAME_OFFSET(V1Capabilities, MigoCapabilities, abi_version_min);
MIGO_OUT_SAME_OFFSET(V1Capabilities, MigoCapabilities, abi_version_max);
MIGO_OUT_SAME_OFFSET(V1Capabilities, MigoCapabilities, platform_kinds);

MIGO_OUT_SAME_OFFSET(V1ReleaseStatus, MigoSurfaceReleaseStatus, struct_size);
MIGO_OUT_SAME_OFFSET(V1ReleaseStatus, MigoSurfaceReleaseStatus, abi_version);
MIGO_OUT_SAME_OFFSET(V1ReleaseStatus, MigoSurfaceReleaseStatus, generation);
MIGO_OUT_SAME_OFFSET(V1ReleaseStatus, MigoSurfaceReleaseStatus, state);
MIGO_OUT_SAME_OFFSET(V1ReleaseStatus, MigoSurfaceReleaseStatus, reserved0);

/*
 * The v1 shape must remain no larger than the current struct: a library that
 * shrank a written struct would report fewer bytes than a v1 client's buffer
 * reserves, and the mirror rule (write min(caller, current)) would then leave
 * part of the caller's v1 fields unwritten.
 */
_Static_assert(sizeof(V1Capabilities) <= sizeof(MigoCapabilities),
               "current MigoCapabilities must still contain the v1 shape");
_Static_assert(sizeof(V1ReleaseStatus) <= sizeof(MigoSurfaceReleaseStatus),
               "current MigoSurfaceReleaseStatus must still contain the v1 shape");

#if MIGO_LP64
_Static_assert(sizeof(V1Capabilities) == 24, "v1 MigoCapabilities is 24 bytes on LP64");
_Static_assert(sizeof(V1ReleaseStatus) == 24, "v1 MigoSurfaceReleaseStatus is 24 bytes on LP64");
#endif

/*
 * What a compiled old client does to read capabilities: announce its own size
 * and hand over a buffer sized to the shape it knows. Kept as a function so the
 * compiler checks the call is well typed against the current header. The gate
 * compiles this translation unit; it does not link or run it.
 */
MigoResult migo_old_client_reads_capabilities(V1Capabilities *out);

MigoResult migo_old_client_reads_capabilities(V1Capabilities *out) {
    out->struct_size = (uint32_t)sizeof *out;
    out->abi_version = MIGO_ABI_VERSION_CURRENT;
    /* The cast is what an old client's machine code does implicitly: it passes
     * the address of storage smaller than the library's current struct. */
    return migo_query_capabilities((MigoCapabilities *)out);
}
