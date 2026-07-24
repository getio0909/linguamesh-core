#![no_main]

use std::ptr;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use linguamesh_ffi::{LmEngine, LmResultCode, lm_engine_create, lm_engine_submit};
use linguamesh_protocol::{Envelope, message_type};
use prost::Message;

const MAX_PROTOCOL_MESSAGE_BYTES: usize = 1024 * 1024;

static ENGINE: OnceLock<usize> = OnceLock::new();

fn fuzz_engine() -> *mut LmEngine {
    *ENGINE.get_or_init(|| {
        let mut engine = ptr::null_mut();
        let result = unsafe { lm_engine_create(&raw mut engine) };
        assert_eq!(result, LmResultCode::Ok);
        engine as usize
    }) as *mut LmEngine
}

fn is_network_command(input: &[u8]) -> bool {
    Envelope::decode(input)
        .map(|envelope| envelope.message_type == message_type::TRANSLATE_TEXT)
        .unwrap_or(false)
}

fuzz_target!(|input: &[u8]| {
    if input.len() > MAX_PROTOCOL_MESSAGE_BYTES || is_network_command(input) {
        return;
    }

    let result = unsafe { lm_engine_submit(fuzz_engine(), input.as_ptr(), input.len()) };
    assert!(
        matches!(
            result,
            LmResultCode::InvalidArgument
                | LmResultCode::ProtocolIncompatible
                | LmResultCode::MalformedMessage
                | LmResultCode::UnsupportedMessage
                | LmResultCode::Busy
        ),
        "unexpected FFI result code: {result:?}"
    );
});
