#![no_main]

use std::ptr;

use libfuzzer_sys::fuzz_target;
use linguamesh_ffi::{
    LmBuffer, LmEngine, LmResultCode, lm_engine_buffer_free, lm_engine_cancel, lm_engine_create,
    lm_engine_destroy, lm_engine_file_lease_create_temporary_path, lm_engine_file_lease_destroy,
    lm_engine_file_lease_expire, lm_engine_file_lease_is_active, lm_engine_file_lease_revoke,
    lm_engine_get_compatibility, lm_engine_poll_event, lm_engine_send_host_response,
    lm_engine_shutdown,
};

const MAX_SEQUENCE_BYTES: usize = 4096;
const TEMPORARY_PATH: &[u8] = b"/tmp/linguamesh-fuzz-sequence";

fn empty_buffer() -> LmBuffer {
    LmBuffer {
        data: ptr::null_mut(),
        len: 0,
        capacity: 0,
        allocation_id: 0,
    }
}

fn allowed_control_result(result: LmResultCode) -> bool {
    matches!(
        result,
        LmResultCode::Ok
            | LmResultCode::InvalidArgument
            | LmResultCode::Shutdown
            | LmResultCode::ResourceExhausted
            | LmResultCode::ProtocolIncompatible
            | LmResultCode::MalformedMessage
            | LmResultCode::UnsupportedMessage
            | LmResultCode::Busy
            | LmResultCode::Internal
    )
}

fuzz_target!(|input: &[u8]| {
    let mut engine = ptr::null_mut::<LmEngine>();
    assert_eq!(
        unsafe { lm_engine_create(&raw mut engine) },
        LmResultCode::Ok
    );

    let mut leases = Vec::new();
    for chunk in input[..input.len().min(MAX_SEQUENCE_BYTES)].chunks(2) {
        let opcode = chunk[0] % 10;
        let argument = chunk.get(1).copied().unwrap_or_default();
        let lease_id = leases
            .get(usize::from(argument) % leases.len().max(1))
            .copied()
            .unwrap_or(u64::from(argument));

        let result = match opcode {
            0 => unsafe { lm_engine_cancel(engine) },
            1 => unsafe { lm_engine_shutdown(engine) },
            2 => {
                let mut output = empty_buffer();
                let result = unsafe {
                    lm_engine_poll_event(engine, u32::from(argument % 2), &raw mut output)
                };
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
            4 => {
                let mut client_owned = [argument];
                let mut forged = LmBuffer {
                    data: client_owned.as_mut_ptr(),
                    len: client_owned.len(),
                    capacity: client_owned.len(),
                    allocation_id: u64::from(argument),
                };
                unsafe { lm_engine_buffer_free(engine, &raw mut forged) }
            }
            5 => {
                let mut created = 0_u64;
                let result = unsafe {
                    lm_engine_file_lease_create_temporary_path(
                        engine,
                        TEMPORARY_PATH.as_ptr(),
                        TEMPORARY_PATH.len(),
                        &raw mut created,
                    )
                };
                if result == LmResultCode::Ok {
                    leases.push(created);
                }
                result
            }
            6 => {
                let mut active = 0_u8;
                unsafe { lm_engine_file_lease_is_active(engine, lease_id, &raw mut active) }
            }
            7 => unsafe { lm_engine_file_lease_expire(engine, lease_id) },
            8 => unsafe { lm_engine_file_lease_revoke(engine, lease_id) },
            _ => {
                if argument == 0 {
                    unsafe { lm_engine_file_lease_destroy(engine, lease_id) }
                } else {
                    let response = [argument];
                    unsafe {
                        lm_engine_send_host_response(engine, response.as_ptr(), response.len())
                    }
                }
            }
        };
        assert!(
            allowed_control_result(result),
            "unexpected FFI result code: {result:?}"
        );
    }

    for lease_id in leases {
        let result = unsafe { lm_engine_file_lease_destroy(engine, lease_id) };
        assert!(matches!(
            result,
            LmResultCode::Ok | LmResultCode::InvalidArgument
        ));
    }
    assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
});
