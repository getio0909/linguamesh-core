#ifndef LINGUAMESH_H
#define LINGUAMESH_H

#include <stddef.h>
#include <stdint.h>

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
    LM_RESULT_MALFORMED_MESSAGE = 5
};

typedef struct LmBuffer {
    uint8_t *data;
    size_t len;
    size_t capacity;
} LmBuffer;

LmResultCode lm_engine_create(LmEngine **output);
uint32_t lm_engine_get_version(void);
LmResultCode lm_engine_submit(
    LmEngine *engine,
    const uint8_t *command_data,
    size_t command_len
);
LmResultCode lm_engine_poll_event(
    LmEngine *engine,
    uint32_t timeout_ms,
    LmBuffer *output
);
LmResultCode lm_engine_send_host_response(
    LmEngine *engine,
    const uint8_t *response_data,
    size_t response_len
);
LmResultCode lm_engine_cancel(LmEngine *engine);
LmResultCode lm_engine_shutdown(LmEngine *engine);
LmResultCode lm_engine_destroy(LmEngine *engine);
LmResultCode lm_buffer_free(LmBuffer *buffer);

#ifdef __cplusplus
}
#endif

#endif
