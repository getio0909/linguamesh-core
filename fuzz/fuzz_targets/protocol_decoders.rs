#![no_main]

use libfuzzer_sys::fuzz_target;
use linguamesh_protocol::{
    CompatibilitySnapshot, Envelope, FailureEvent, HostSecretResponse, SecretRequiredEvent,
    TextDeltaEvent, TranslateTextCommand, message_type,
};
use prost::Message;

fn decode_known_payload(message_type: &str, payload: &[u8]) {
    match message_type {
        message_type::TRANSLATE_TEXT => {
            let _ = TranslateTextCommand::decode(payload);
        }
        message_type::TEXT_DELTA => {
            let _ = TextDeltaEvent::decode(payload);
        }
        message_type::FAILED => {
            let _ = FailureEvent::decode(payload);
        }
        message_type::SECRET_REQUIRED => {
            let _ = SecretRequiredEvent::decode(payload);
        }
        message_type::HOST_SECRET_RESPONSE => {
            let _ = HostSecretResponse::decode(payload);
        }
        message_type::COMPATIBILITY => {
            let _ = CompatibilitySnapshot::decode(payload);
        }
        message_type::STARTED | message_type::COMPLETED | message_type::CANCELLED => {}
        _ => {}
    }
}

fuzz_target!(|input: &[u8]| {
    if input.is_empty() || input.len() > 1024 * 1024 {
        return;
    }

    let Ok(envelope) = Envelope::decode(input) else {
        return;
    };
    if envelope.validate_version().is_err() {
        return;
    }
    decode_known_payload(&envelope.message_type, envelope.payload.as_slice());
});
