cmake_minimum_required(VERSION 3.16)
project(migo_linux_surface_host_consumer LANGUAGES CXX)

if(NOT DEFINED MIGO_PUBLIC_INCLUDE)
    message(FATAL_ERROR "MIGO_PUBLIC_INCLUDE must point at the core SDK's public headers")
endif()

add_library(migo::migo INTERFACE IMPORTED GLOBAL)
set_target_properties(migo::migo PROPERTIES
    INTERFACE_INCLUDE_DIRECTORIES "${MIGO_PUBLIC_INCLUDE}"
)

set(MIGO_LINUX_HOST_KIT_BUILD_QT6_X11 OFF CACHE BOOL "" FORCE)
if(NOT DEFINED MIGO_LINUX_HOST_KIT_ENABLE_INSTALL)
    set(MIGO_LINUX_HOST_KIT_ENABLE_INSTALL OFF CACHE BOOL "")
endif()
add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../.." host-kit)
if(TARGET Qt6::Core OR TARGET migo_qt6_x11_surface_view)
    message(FATAL_ERROR "controller-only configuration unexpectedly created Qt targets")
endif()

add_library(migo-linux-surface-host-consumer STATIC consumer.cpp)
set_target_properties(migo-linux-surface-host-consumer PROPERTIES
    CXX_STANDARD 17
    CXX_STANDARD_REQUIRED ON
    CXX_EXTENSIONS ON
)
target_link_libraries(migo-linux-surface-host-consumer PRIVATE
    migo::linux-surface-host
)
target_compile_options(migo-linux-surface-host-consumer PRIVATE
    -Wall -Wextra -Werror
)
