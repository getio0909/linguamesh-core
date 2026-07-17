#include <jni.h>

#include "linguamesh.h"

#include <cstddef>
#include <cstdint>
#include <limits>

namespace {

LmEngine* engine_from_handle(const jlong handle) noexcept {
    return reinterpret_cast<LmEngine*>(static_cast<std::uintptr_t>(handle));
}

jlong handle_from_engine(LmEngine* engine) noexcept {
    return static_cast<jlong>(reinterpret_cast<std::uintptr_t>(engine));
}

template <typename Operation>
jint with_bytes(JNIEnv* environment, jbyteArray input, Operation operation) {
    if (input == nullptr) {
        return LM_RESULT_INVALID_ARGUMENT;
    }
    const auto length = environment->GetArrayLength(input);
    if (length <= 0) {
        return LM_RESULT_INVALID_ARGUMENT;
    }
    jbyte* bytes = environment->GetByteArrayElements(input, nullptr);
    if (bytes == nullptr) {
        return LM_RESULT_INTERNAL;
    }
    const auto result = operation(
        reinterpret_cast<const std::uint8_t*>(bytes),
        static_cast<std::size_t>(length));
    environment->ReleaseByteArrayElements(input, bytes, JNI_ABORT);
    return result;
}

}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_create(
    JNIEnv* environment,
    jclass,
    jlongArray output) {
    if (output == nullptr || environment->GetArrayLength(output) < 1) {
        return LM_RESULT_INVALID_ARGUMENT;
    }
    LmEngine* engine = nullptr;
    const auto result = lm_engine_create(&engine);
    if (result != LM_RESULT_OK) {
        return result;
    }
    const jlong handle = handle_from_engine(engine);
    environment->SetLongArrayRegion(output, 0, 1, &handle);
    if (environment->ExceptionCheck() == JNI_TRUE) {
        static_cast<void>(lm_engine_destroy(engine));
        return LM_RESULT_INTERNAL;
    }
    return LM_RESULT_OK;
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_abiVersion(JNIEnv*, jclass) {
    return static_cast<jint>(lm_engine_get_abi_version());
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_protocolVersion(JNIEnv*, jclass) {
    return static_cast<jint>(lm_engine_get_protocol_version());
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_submit(
    JNIEnv* environment,
    jclass,
    const jlong handle,
    jbyteArray command) {
    return with_bytes(environment, command, [handle](const std::uint8_t* bytes, const std::size_t length) {
        return lm_engine_submit(engine_from_handle(handle), bytes, length);
    });
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_pollEvent(
    JNIEnv* environment,
    jclass,
    const jlong handle,
    const jint timeout_millis,
    jobjectArray output) {
    if (output == nullptr || environment->GetArrayLength(output) < 1) {
        return LM_RESULT_INVALID_ARGUMENT;
    }
    auto* engine = engine_from_handle(handle);
    LmBuffer buffer{};
    const auto result = lm_engine_poll_event(
        engine,
        static_cast<std::uint32_t>(timeout_millis),
        &buffer);
    if (result != LM_RESULT_OK) {
        return result;
    }
    if (buffer.len > static_cast<std::size_t>(std::numeric_limits<jsize>::max())) {
        static_cast<void>(lm_engine_buffer_free(engine, &buffer));
        return LM_RESULT_INTERNAL;
    }
    jbyteArray bytes = environment->NewByteArray(static_cast<jsize>(buffer.len));
    if (bytes == nullptr) {
        static_cast<void>(lm_engine_buffer_free(engine, &buffer));
        return LM_RESULT_INTERNAL;
    }
    if (buffer.len > 0) {
        environment->SetByteArrayRegion(
            bytes,
            0,
            static_cast<jsize>(buffer.len),
            reinterpret_cast<const jbyte*>(buffer.data));
    }
    environment->SetObjectArrayElement(output, 0, bytes);
    environment->DeleteLocalRef(bytes);
    const auto free_result = lm_engine_buffer_free(engine, &buffer);
    if (environment->ExceptionCheck() == JNI_TRUE) {
        return LM_RESULT_INTERNAL;
    }
    return free_result;
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_sendHostResponse(
    JNIEnv* environment,
    jclass,
    const jlong handle,
    jbyteArray response) {
    return with_bytes(environment, response, [handle](const std::uint8_t* bytes, const std::size_t length) {
        return lm_engine_send_host_response(engine_from_handle(handle), bytes, length);
    });
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_cancel(JNIEnv*, jclass, const jlong handle) {
    return lm_engine_cancel(engine_from_handle(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_shutdown(JNIEnv*, jclass, const jlong handle) {
    return lm_engine_shutdown(engine_from_handle(handle));
}

extern "C" JNIEXPORT jint JNICALL
Java_org_linguamesh_core_NativeBridge_destroy(JNIEnv*, jclass, const jlong handle) {
    return lm_engine_destroy(engine_from_handle(handle));
}
