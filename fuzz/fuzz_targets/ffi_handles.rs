#![no_main]

use std::ptr;

use libfuzzer_sys::fuzz_target;
use linguamesh_ffi::{
    LmBuffer, LmEngine, LmResultCode, lm_engine_buffer_free, lm_engine_cancel, lm_engine_create,
    lm_engine_destroy, lm_engine_get_compatibility, lm_engine_poll_event, lm_engine_shutdown,
};

fn empty_buffer() -> LmBuffer {
    LmBuffer {
        data: ptr::null_mut(),
        len: 0,
        capacity: 0,
        allocation_id: 0,
    }
}

fn allowed(result: LmResultCode) -> bool {
    matches!(
        result,
        LmResultCode::Ok
            | LmResultCode::InvalidArgument
            | LmResultCode::Shutdown
            | LmResultCode::ResourceExhausted
    )
}

fuzz_target!(|input: &[u8]| {
    let mut engine = ptr::null_mut::<LmEngine>();
    assert_eq!(
        unsafe { lm_engine_create(&raw mut engine) },
        LmResultCode::Ok
    );
    let mut live = true;

    for byte in input.iter().copied().take(1024) {
        let result = match byte % 5 {
            0 => unsafe { lm_engine_cancel(engine) },
            1 => unsafe { lm_engine_shutdown(engine) },
            2 => {
                let mut output = empty_buffer();
                let result = unsafe { lm_engine_poll_event(engine, u32::from(byte), &raw mut output) };
                if !output.data.is_null() {
                    assert_eq!(
                        unsafe { lm_engine_buffer_free(engine, &raw mut output) },
                        LmResultCode::Ok
                    );
                }
                result
            }
            3 => {
                let mut output = empty_buffer();
                let result = unsafe { lm_engine_get_compatibility(engine, &raw mut output) };
                if !output.data.is_null() {
                    assert_eq!(
                        unsafe { lm_engine_buffer_free(engine, &raw mut output) },
                        LmResultCode::Ok
                    );
                }
                result
            }
            _ if live => {
                live = false;
                unsafe { lm_engine_destroy(engine) }
            }
            _ => unsafe { lm_engine_destroy(engine) },
        };
        assert!(allowed(result), "unexpected stale-handle result: {result:?}");
    }

    if live {
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
    } else {
        assert_eq!(
            unsafe { lm_engine_destroy(engine) },
            LmResultCode::InvalidArgument
        );
    }
});
