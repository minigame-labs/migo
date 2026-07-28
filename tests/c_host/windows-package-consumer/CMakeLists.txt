# A third-party MSVC consumer of the Migo Windows SDK package, built the
# idiomatic way: find_package(migo) + target_link_libraries.
#
# Build it exactly as a host would:
#   cmake -S . -B build -DCMAKE_PREFIX_PATH=<package prefix>
#   cmake --build build --config Release
#
# See build-cmake.sh for the wrapper. build.sh is the same proof without CMake
# (raw cl + migo.lib), for hosts that do not use CMake.
cmake_minimum_required(VERSION 3.16)

# C alone is enough here, unlike the Android package consumer. There migo::migo
# is a static C++ archive whose Skia/V8 symbols the consumer's link must
# resolve, so that project must enable CXX to pull in a C++ runtime. On Windows
# migo::migo is a DLL that has already absorbed V8 and links Skia internally, so
# the consumer resolves only the migo_* imports through migo.lib and needs no
# C++ runtime of its own.
project(migo_windows_consumer C)

# The package ships lib/cmake/migo/migo-config.cmake; find_package resolves it
# from CMAKE_PREFIX_PATH. The migo::migo target carries everything the consumer
# needs: the include dir, the MIGO_USE_SHARED define (so MIGO_API becomes
# __declspec(dllimport)), the C11 requirement the headers' _Static_assert needs,
# and the import library.
find_package(migo REQUIRED)

add_executable(consumer consumer.c)
target_link_libraries(consumer PRIVATE migo::migo)
