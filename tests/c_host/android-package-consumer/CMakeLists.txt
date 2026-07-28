# A third-party NDK consumer of the Migo Android C ABI package.
#
# Build it exactly as a host would:
#   cmake -S . -B build \
#     -DCMAKE_TOOLCHAIN_FILE=$ANDROID_NDK/build/cmake/android.toolchain.cmake \
#     -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-26 \
#     -DANDROID_STL=c++_shared \
#     -DCMAKE_PREFIX_PATH=<package prefix>
#   cmake --build build
#
# See build.sh for the wrapper.
cmake_minimum_required(VERSION 3.16)
# C CXX, not C alone: consumer.c is pure C, but migo::migo is a C++ archive
# (Skia, V8). A C-only project does not link the C++ runtime, and the link fails
# on hundreds of std::__ndk1 symbols. Enabling CXX makes the toolchain link the
# consumer's chosen -DANDROID_STL -- which is why the package does not, and must
# not, put a C++ runtime in its own link line.
project(migo_android_consumer C CXX)

# The package ships lib/cmake/migo/migo-config.cmake; find_package resolves it
# from CMAKE_PREFIX_PATH. The migo::migo target carries the include directory
# and the platform link libraries the archive needs.
find_package(migo REQUIRED)

add_library(consumer SHARED consumer.c)

# consumer.c is C, so CMake would link with the C driver, which does not pull the
# C++ runtime -- and migo::migo is a C++ archive. Linking as C++ makes the
# toolchain link the chosen -DANDROID_STL. This is the one thing a pure-C host of
# a C++ library must know; the package cannot decide the STL for it.
set_target_properties(consumer PROPERTIES LINKER_LANGUAGE CXX)

# The migo_* entry points are the only symbol worth keeping; hide the rest so a
# consumed static archive does not re-export Migo's internals. Also roots the
# probe so --gc-sections keeps it. --gc-sections is load-bearing for the same
# reason it is in the C host example (skia-bindings' unused JPEG/PDF wrappers).
target_link_options(consumer PRIVATE
    -Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/consumer.map
    -Wl,--gc-sections
    -Wl,--no-undefined)

target_link_libraries(consumer PRIVATE migo::migo)
