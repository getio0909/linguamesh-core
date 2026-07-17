#include "linguamesh.h"

#include <assert.h>
#include <stdio.h>

int main(void) {
    assert(lm_engine_get_abi_version() == LM_ABI_VERSION_MAJOR);
    assert(lm_engine_get_protocol_version() == LM_PROTOCOL_VERSION);
    assert(LM_MAX_OUTSTANDING_BUFFERS == 64);
    assert(LM_RESULT_RESOURCE_EXHAUSTED == 9);
    LmEngine *engine = NULL;
    assert(lm_engine_create(&engine) == LM_RESULT_OK);
    assert(engine != NULL);
    LmBuffer event = {0};
    assert(lm_engine_poll_event(engine, 0, &event) == LM_RESULT_OK);
    assert(event.data == NULL);
    assert(event.len == 0);
    assert(lm_engine_shutdown(engine) == LM_RESULT_OK);
    assert(lm_engine_buffer_free(engine, &event) == LM_RESULT_OK);
    assert(lm_engine_destroy(engine) == LM_RESULT_OK);
    puts("C ABI smoke test passed.");
    return 0;
}
