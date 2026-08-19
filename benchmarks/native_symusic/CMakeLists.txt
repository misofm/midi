cmake_minimum_required(VERSION 3.20)

project(miso_native_symusic_bench LANGUAGES CXX)

include(CheckIPOSupported)

set(SYMUSIC_SOURCE_DIR "" CACHE PATH "Checkout of the pinned Symusic source tree")
set(MISO_SYMUSIC_PIN "43ff25277abbc72dbd8d00fb5a9a14ec37fb7906" CACHE STRING "Symusic v0.6.0 source commit")

if(NOT EXISTS "${SYMUSIC_SOURCE_DIR}/CMakeLists.txt")
    message(FATAL_ERROR
        "Set SYMUSIC_SOURCE_DIR to a populated Symusic checkout. Run "
        "benchmarks/native_symusic/fetch_symusic.sh first."
    )
endif()

execute_process(
    COMMAND git -C "${SYMUSIC_SOURCE_DIR}" rev-parse HEAD
    RESULT_VARIABLE symusic_git_result
    OUTPUT_VARIABLE symusic_git_head
    OUTPUT_STRIP_TRAILING_WHITESPACE
)
if(NOT symusic_git_result EQUAL 0 OR NOT symusic_git_head STREQUAL MISO_SYMUSIC_PIN)
    message(FATAL_ERROR "Symusic source HEAD must be ${MISO_SYMUSIC_PIN}; got ${symusic_git_head}")
endif()

if(NOT CMAKE_BUILD_TYPE STREQUAL "Release")
    message(FATAL_ERROR "Native comparison requires -DCMAKE_BUILD_TYPE=Release")
endif()

# This benchmark is a C++ consumer of Symusic's public library, not its Python
# extension. Keep the competitor build configuration explicit and record it in
# the executable's report.
set(BUILD_SYMUSIC_PY OFF CACHE BOOL "" FORCE)
set(BUILD_SYMUSIC_EXAMPLE OFF CACHE BOOL "" FORCE)
set(BUILD_SYMUSIC_TEST OFF CACHE BOOL "" FORCE)
set(ENABLE_LTO ON CACHE BOOL "" FORCE)

check_ipo_supported(RESULT MISO_SYMUSIC_IPO_SUPPORTED OUTPUT miso_symusic_ipo_error LANGUAGES CXX)
if(NOT MISO_SYMUSIC_IPO_SUPPORTED)
    message(FATAL_ERROR "Release comparison requires C++ IPO/LTO: ${miso_symusic_ipo_error}")
endif()

add_subdirectory("${SYMUSIC_SOURCE_DIR}" "${CMAKE_BINARY_DIR}/symusic" EXCLUDE_FROM_ALL)
get_property(MISO_SYMUSIC_LIBRARY_IPO_ENABLED TARGET symusic PROPERTY INTERPROCEDURAL_OPTIMIZATION)
if(NOT MISO_SYMUSIC_LIBRARY_IPO_ENABLED)
    message(FATAL_ERROR "Pinned Symusic target did not enable IPO with ENABLE_LTO=ON")
endif()

add_executable(miso_native_symusic_bench src/main.cpp)
target_compile_features(miso_native_symusic_bench PRIVATE cxx_std_20)
target_compile_definitions(
    miso_native_symusic_bench
    PRIVATE
        MISO_SYMUSIC_PIN="${MISO_SYMUSIC_PIN}"
        MISO_SYMUSIC_BUILD_TYPE="${CMAKE_BUILD_TYPE}"
        MISO_SYMUSIC_LTO_REQUESTED=$<BOOL:${ENABLE_LTO}>
        MISO_SYMUSIC_IPO_ENABLED=1
        MISO_SYMUSIC_LIBRARY_IPO_ENABLED=1
        MISO_SYMUSIC_CMAKE_VERSION="${CMAKE_VERSION}"
        MISO_SYMUSIC_CMAKE_COMPILER_ID="${CMAKE_CXX_COMPILER_ID}"
        MISO_SYMUSIC_CMAKE_COMPILER_VERSION="${CMAKE_CXX_COMPILER_VERSION}"
        MISO_SYMUSIC_CMAKE_CXX_FLAGS="${CMAKE_CXX_FLAGS} ${CMAKE_CXX_FLAGS_RELEASE}"
        MISO_SYMUSIC_CMAKE_GENERATOR="${CMAKE_GENERATOR}"
)
target_include_directories(
    miso_native_symusic_bench
    PRIVATE
        "${SYMUSIC_SOURCE_DIR}/include"
        "${SYMUSIC_SOURCE_DIR}/3rdparty"
        "${SYMUSIC_SOURCE_DIR}/3rdparty/pdqsort"
        "${SYMUSIC_SOURCE_DIR}/3rdparty/pyvec"
        "${SYMUSIC_SOURCE_DIR}/3rdparty/zpp_bits"
)
target_link_libraries(miso_native_symusic_bench PRIVATE symusic)
set_property(TARGET miso_native_symusic_bench PROPERTY INTERPROCEDURAL_OPTIMIZATION TRUE)

# Symusic's third-party tree disables BUILD_TESTING globally. Keep this focused
# harness check independent so source/build validation never silently reports
# an empty CTest suite.
enable_testing()
add_test(NAME native_symusic_self_test COMMAND miso_native_symusic_bench --self-test)
