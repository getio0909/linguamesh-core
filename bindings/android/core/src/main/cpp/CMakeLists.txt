cmake_minimum_required(VERSION 3.22.1)
project(linguamesh_jni LANGUAGES CXX)

if(NOT DEFINED LINGUAMESH_CORE_ROOT)
    message(FATAL_ERROR "LINGUAMESH_CORE_ROOT is required.")
endif()

add_library(linguamesh_ffi SHARED IMPORTED)
# 强制链接器记录库文件名，避免把构建机绝对路径写入 DT_NEEDED。
set_target_properties(
    linguamesh_ffi
    PROPERTIES
        IMPORTED_LOCATION "${CMAKE_CURRENT_LIST_DIR}/../jniLibs/${ANDROID_ABI}/liblinguamesh_ffi.so"
        IMPORTED_NO_SONAME TRUE
)

add_library(linguamesh_jni SHARED linguamesh_jni.cpp)
target_compile_features(linguamesh_jni PRIVATE cxx_std_20)
target_include_directories(linguamesh_jni PRIVATE "${LINGUAMESH_CORE_ROOT}/contracts/abi")
target_link_libraries(linguamesh_jni PRIVATE linguamesh_ffi)
