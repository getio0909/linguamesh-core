#ifndef LINGUAMESH_HPP
#define LINGUAMESH_HPP

#include "linguamesh.h"

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace linguamesh {

/// 镜像稳定 C ABI 结果码。
enum class result_code : std::int32_t {
    ok = LM_RESULT_OK,
    invalid_argument = LM_RESULT_INVALID_ARGUMENT,
    shutdown = LM_RESULT_SHUTDOWN,
    panic = LM_RESULT_PANIC,
    protocol_incompatible = LM_RESULT_PROTOCOL_INCOMPATIBLE,
    malformed_message = LM_RESULT_MALFORMED_MESSAGE,
    busy = LM_RESULT_BUSY,
    unsupported_message = LM_RESULT_UNSUPPORTED_MESSAGE,
    internal = LM_RESULT_INTERNAL,
    resource_exhausted = LM_RESULT_RESOURCE_EXHAUSTED,
};

/// 表示核心调用返回的稳定错误。
class core_error final : public std::runtime_error {
public:
    explicit core_error(const result_code code)
        : std::runtime_error(message_for(code)), code_(code) {}

    [[nodiscard]] result_code code() const noexcept { return code_; }

private:
    static const char* message_for(const result_code code) noexcept {
        switch (code) {
        case result_code::ok:
            return "LinguaMesh call succeeded.";
        case result_code::invalid_argument:
            return "LinguaMesh rejected an invalid argument.";
        case result_code::shutdown:
            return "LinguaMesh engine is shut down.";
        case result_code::panic:
            return "LinguaMesh contained an internal panic.";
        case result_code::protocol_incompatible:
            return "LinguaMesh protocol version is incompatible.";
        case result_code::malformed_message:
            return "LinguaMesh rejected a malformed message.";
        case result_code::busy:
            return "LinguaMesh engine already has an active operation.";
        case result_code::unsupported_message:
            return "LinguaMesh does not support this message type.";
        case result_code::internal:
            return "LinguaMesh could not initialize an internal resource.";
        case result_code::resource_exhausted:
            return "LinguaMesh engine has too many outstanding buffers.";
        }
        return "LinguaMesh returned an unknown result code.";
    }

    result_code code_;
};

/// 表示客户端与已加载核心的版本不兼容。
class compatibility_error final : public std::runtime_error {
public:
    compatibility_error(
        const std::uint32_t expected_abi,
        const std::uint32_t actual_abi,
        const std::uint32_t expected_protocol,
        const std::uint32_t actual_protocol)
        : std::runtime_error("Loaded LinguaMesh core is incompatible."),
          expected_abi_(expected_abi),
          actual_abi_(actual_abi),
          expected_protocol_(expected_protocol),
          actual_protocol_(actual_protocol) {}

    [[nodiscard]] std::uint32_t expected_abi() const noexcept { return expected_abi_; }
    [[nodiscard]] std::uint32_t actual_abi() const noexcept { return actual_abi_; }
    [[nodiscard]] std::uint32_t expected_protocol() const noexcept { return expected_protocol_; }
    [[nodiscard]] std::uint32_t actual_protocol() const noexcept { return actual_protocol_; }

private:
    std::uint32_t expected_abi_;
    std::uint32_t actual_abi_;
    std::uint32_t expected_protocol_;
    std::uint32_t actual_protocol_;
};

/// 包含启动时必须验证的边界版本。
struct compatibility final {
    std::uint32_t abi_major;
    std::uint32_t protocol;
};

inline result_code to_result_code(const LmResultCode code) noexcept {
    return static_cast<result_code>(code);
}

inline void throw_on_error(const LmResultCode code) {
    if (code != LM_RESULT_OK) {
        throw core_error(to_result_code(code));
    }
}

/// 自动释放 Rust 分配的事件缓冲区。
class buffer final {
public:
    buffer() noexcept = default;
    buffer(const buffer&) = delete;
    buffer& operator=(const buffer&) = delete;

    buffer(buffer&& other) noexcept
        : owner_(std::exchange(other.owner_, nullptr)), value_(other.value_) {
        other.value_ = {};
    }

    buffer& operator=(buffer&& other) noexcept {
        if (this != &other) {
            release_noexcept();
            owner_ = std::exchange(other.owner_, nullptr);
            value_ = other.value_;
            other.value_ = {};
        }
        return *this;
    }

    ~buffer() { release_noexcept(); }

    [[nodiscard]] bool empty() const noexcept { return value_.len == 0; }
    [[nodiscard]] std::size_t size() const noexcept { return value_.len; }

    [[nodiscard]] std::span<const std::uint8_t> bytes() const noexcept {
        if (value_.data == nullptr) {
            return {};
        }
        return {value_.data, value_.len};
    }

    [[nodiscard]] std::vector<std::uint8_t> copy() const {
        const auto view = bytes();
        return {view.begin(), view.end()};
    }

private:
    friend class engine;

    explicit buffer(LmEngine* owner) noexcept : owner_(owner) {}

    [[nodiscard]] LmBuffer* output_parameter() {
        release();
        return &value_;
    }

    void release() {
        if (owner_ == nullptr) {
            if (value_.data == nullptr && value_.len == 0 && value_.capacity == 0
                && value_.allocation_id == 0) {
                return;
            }
            throw std::logic_error("LinguaMesh buffer has no owning engine.");
        }
        throw_on_error(lm_engine_buffer_free(owner_, &value_));
    }

    void release_noexcept() noexcept {
        if (owner_ != nullptr) {
            static_cast<void>(lm_engine_buffer_free(owner_, &value_));
        }
    }

    LmEngine* owner_ = nullptr;
    LmBuffer value_{};
};

/// 独占持有引擎句柄并提供同步 C++ 边界。
class engine final {
public:
    static engine create(
        const std::uint32_t expected_abi = LM_ABI_VERSION_MAJOR,
        const std::uint32_t expected_protocol = LM_PROTOCOL_VERSION) {
        const auto actual = query_compatibility();
        if (actual.abi_major != expected_abi || actual.protocol != expected_protocol) {
            throw compatibility_error(
                expected_abi,
                actual.abi_major,
                expected_protocol,
                actual.protocol);
        }
        LmEngine* handle = nullptr;
        throw_on_error(lm_engine_create(&handle));
        if (handle == nullptr) {
            throw core_error(result_code::internal);
        }
        return engine(handle);
    }

    static compatibility query_compatibility() noexcept {
        return {lm_engine_get_abi_version(), lm_engine_get_protocol_version()};
    }

    engine(const engine&) = delete;
    engine& operator=(const engine&) = delete;

    engine(engine&& other) noexcept
        : handle_(std::exchange(other.handle_, nullptr)),
          shutdown_(std::exchange(other.shutdown_, true)) {}

    engine& operator=(engine&& other) noexcept {
        if (this != &other) {
            close_noexcept();
            handle_ = std::exchange(other.handle_, nullptr);
            shutdown_ = std::exchange(other.shutdown_, true);
        }
        return *this;
    }

    ~engine() { close_noexcept(); }

    void submit(const std::span<const std::uint8_t> command) {
        require_handle();
        throw_on_error(lm_engine_submit(handle_, command.data(), command.size()));
    }

    void send_host_response(const std::span<const std::uint8_t> response) {
        require_handle();
        throw_on_error(lm_engine_send_host_response(handle_, response.data(), response.size()));
    }

    [[nodiscard]] std::vector<std::uint8_t> poll_event(const std::chrono::milliseconds timeout) {
        require_handle();
        if (timeout.count() < 0
            || static_cast<std::uint64_t>(timeout.count())
                > std::numeric_limits<std::uint32_t>::max()) {
            throw std::invalid_argument("Poll timeout is outside the supported range.");
        }
        buffer event(handle_);
        throw_on_error(lm_engine_poll_event(
            handle_,
            static_cast<std::uint32_t>(timeout.count()),
            event.output_parameter()));
        auto copied = event.copy();
        event.release();
        return copied;
    }

    void cancel() {
        require_handle();
        throw_on_error(lm_engine_cancel(handle_));
    }

    void shutdown() {
        require_handle();
        throw_on_error(lm_engine_shutdown(handle_));
        shutdown_ = true;
    }

    [[nodiscard]] bool is_open() const noexcept { return handle_ != nullptr; }

private:
    explicit engine(LmEngine* handle) noexcept : handle_(handle) {}

    void require_handle() const {
        if (handle_ == nullptr) {
            throw std::logic_error("LinguaMesh engine handle is closed.");
        }
    }

    void close_noexcept() noexcept {
        if (handle_ == nullptr) {
            return;
        }
        if (!shutdown_) {
            static_cast<void>(lm_engine_shutdown(handle_));
        }
        static_cast<void>(lm_engine_destroy(handle_));
        handle_ = nullptr;
        shutdown_ = true;
    }

    LmEngine* handle_ = nullptr;
    bool shutdown_ = false;
};

}

#endif
