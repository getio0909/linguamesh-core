#ifndef LINGUAMESH_H
#define LINGUAMESH_H

#include <stddef.h>
#include <stdint.h>

#define LM_ABI_VERSION_MAJOR UINT32_C(1)
#define LM_PROTOCOL_VERSION UINT32_C(1)
#define LM_MAX_OUTSTANDING_BUFFERS UINT32_C(64)

/* 统一静态库和动态库客户端的符号导入约定。 */
#if defined(_WIN32) && !defined(LINGUAMESH_STATIC)
#define LM_API __declspec(dllimport)
#elif defined(__GNUC__) && !defined(LINGUAMESH_STATIC)
#define LM_API __attribute__((visibility("default")))
#else
#define LM_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef struct LmEngine LmEngine;
typedef int32_t LmResultCode;

enum {
    LM_RESULT_OK = 0,
    LM_RESULT_INVALID_ARGUMENT = 1,
    LM_RESULT_SHUTDOWN = 2,
    LM_RESULT_PANIC = 3,
    LM_RESULT_PROTOCOL_INCOMPATIBLE = 4,
    LM_RESULT_MALFORMED_MESSAGE = 5,
    LM_RESULT_BUSY = 6,
    LM_RESULT_UNSUPPORTED_MESSAGE = 7,
    LM_RESULT_INTERNAL = 8,
    LM_RESULT_RESOURCE_EXHAUSTED = 9
};

/* 轮询前必须全零初始化；返回后所有字段只读且只能由所属引擎释放。 */
typedef struct LmBuffer {
    uint8_t *data;
    size_t len;
    size_t capacity;
    uint64_t allocation_id;
} LmBuffer;

LM_API LmResultCode lm_engine_create(LmEngine **output);

/* 返回 C ABI 主版本。 */
LM_API uint32_t lm_engine_get_abi_version(void);

/* 返回 Protobuf 命令和事件协议版本。 */
LM_API uint32_t lm_engine_get_protocol_version(void);

/* 保留为旧客户端的协议版本别名。 */
LM_API uint32_t lm_engine_get_version(void);

/* 返回包含核心语义、目录、ABI、协议和功能集合的版本化兼容性 Envelope。 */
LM_API LmResultCode lm_engine_get_compatibility(
    LmEngine *engine,
    LmBuffer *output
);
LM_API LmResultCode lm_engine_submit(
    LmEngine *engine,
    const uint8_t *command_data,
    size_t command_len
);

/* 缓冲区槽位耗尽时返回 RESOURCE_EXHAUSTED，保持输出为零且不消费事件。 */
LM_API LmResultCode lm_engine_poll_event(
    LmEngine *engine,
    uint32_t timeout_ms,
    LmBuffer *output
);
LM_API LmResultCode lm_engine_send_host_response(
    LmEngine *engine,
    const uint8_t *response_data,
    size_t response_len
);
LM_API LmResultCode lm_engine_cancel(LmEngine *engine);
LM_API LmResultCode lm_engine_shutdown(LmEngine *engine);

/* 仅在所有并发调用结束且活动缓冲区已释放后销毁一次句柄。 */
LM_API LmResultCode lm_engine_destroy(LmEngine *engine);

/* 每个引擎最多保留 LM_MAX_OUTSTANDING_BUFFERS 个未释放缓冲区。 */
/* 关闭后仍可释放；重复释放已清空描述符安全，错误引擎、伪造或复制描述符会被拒绝。 */
LM_API LmResultCode lm_engine_buffer_free(LmEngine *engine, LmBuffer *buffer);

#ifdef __cplusplus
}
#endif

#endif
