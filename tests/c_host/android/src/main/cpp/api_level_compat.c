/*
 * android_get_device_api_level as a real symbol.
 *
 * The engine calls it as an external function (graphics/device_caps.rs), but
 * the NDK only exports it from libc at API 29 and above. Below that it is a
 * static inline, so a shared library linked against this project's API 26 stubs
 * has nothing to bind against.
 *
 * The body is the NDK's own, from
 * sysroot/usr/include/bits/get_device_api_level_inlines.h -- the same algorithm
 * the platform uses at API 29, emitted here with external linkage.
 *
 * The first attempt linked the API-level libc.a instead, and that crashed the
 * process before android_main ran: pulling bionic's static implementation into
 * a shared library that also uses the dynamic libc left __bionic_getauxval
 * dereferencing null during LSE-atomics initialisation. Static libc and dynamic
 * libc do not mix in one shared object.
 */
/* No <stdlib.h> or <android/api-level.h>: both pull in the NDK's static inline
 * of this function, and defining it here as well is a redefinition. The two
 * functions used are declared directly, exactly as the NDK's own inline header
 * does for the same reason. */
#define PROP_VALUE_MAX 92
int __system_property_get(const char* __name, char* __value);
int atoi(const char* __s);

int android_get_device_api_level(void) {
    char value[PROP_VALUE_MAX] = {0};
    if (__system_property_get("ro.build.version.sdk", value) < 1) return -1;
    int api_level = atoi(value);
    return (api_level > 0) ? api_level : -1;
}
