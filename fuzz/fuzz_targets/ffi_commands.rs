#![no_main]

use std::ptr;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;

use libfuzzer_sys::fuzz_target;
use linguamesh_ffi::{
    LmBuffer, LmEngine, LmResultCode, lm_engine_buffer_free, lm_engine_create, lm_engine_destroy,
    lm_engine_poll_event, lm_engine_submit,
};
use linguamesh_protocol::{Envelope, PROTOCOL_VERSION, TranslateTextCommand, message_type};
use linguamesh_testkit::FakeProviderServer;
use prost::Message;

const MAX_SOURCE_BYTES: usize = 4096;
const POLL_TIMEOUT_MS: u32 = 250;
const MAX_EVENTS: usize = 16;

fn fuzz_provider_endpoint() -> &'static str {
    static ENDPOINT: OnceLock<String> = OnceLock::new();
    ENDPOINT
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel(1);
            thread::Builder::new()
                .name("ffi-fuzz-provider".to_owned())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .expect("build fuzz provider runtime");
                    runtime.block_on(async move {
                        let server = FakeProviderServer::start()
                            .await
                            .expect("start fuzz provider");
                        sender
                            .send(server.base_url())
                            .expect("publish fuzz provider endpoint");
                        std::future::pending::<()>().await;
                    });
                })
                .expect("spawn fuzz provider thread");
            receiver.recv().expect("receive fuzz provider endpoint")
        })
        .as_str()
}

fn empty_buffer() -> LmBuffer {
    LmBuffer {
        data: ptr::null_mut(),
        len: 0,
        capacity: 0,
        allocation_id: 0,
    }
}

fn is_terminal(message_type: &str) -> bool {
    matches!(
        message_type,
        message_type::COMPLETED | message_type::CANCELLED | message_type::FAILED
    )
}

fuzz_target!(|input: &[u8]| {
    let source_text = if input.is_empty() {
        "fuzz".to_owned()
    } else {
        String::from_utf8_lossy(&input[..input.len().min(MAX_SOURCE_BYTES)]).into_owned()
    };
    let command = TranslateTextCommand {
        endpoint: fuzz_provider_endpoint().to_owned(),
        model_id: "fake-translator".to_owned(),
        source_text,
        target_locale: "zh-CN".to_owned(),
        secret_ref: String::new(),
        organization: String::new(),
        project: String::new(),
        custom_headers_json: String::new(),
    };
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        operation_id: "ffi-fuzz-operation".to_owned(),
        correlation_id: "ffi-fuzz-correlation".to_owned(),
        sequence: 0,
        message_type: message_type::TRANSLATE_TEXT.to_owned(),
        payload: command.encode_to_vec(),
    }
    .encode_to_vec();

    let mut engine = ptr::null_mut::<LmEngine>();
    assert_eq!(
        unsafe { lm_engine_create(&raw mut engine) },
        LmResultCode::Ok
    );
    assert_eq!(
        unsafe { lm_engine_submit(engine, envelope.as_ptr(), envelope.len()) },
        LmResultCode::Ok
    );

    let mut terminal_count = 0;
    for _ in 0..MAX_EVENTS {
        let mut output = empty_buffer();
        assert_eq!(
            unsafe { lm_engine_poll_event(engine, POLL_TIMEOUT_MS, &raw mut output) },
            LmResultCode::Ok
        );
        if output.data.is_null() {
            continue;
        }
        let bytes = unsafe { std::slice::from_raw_parts(output.data, output.len) }.to_vec();
        assert_eq!(
            unsafe { lm_engine_buffer_free(engine, &raw mut output) },
            LmResultCode::Ok
        );
        let event = Envelope::decode(bytes.as_slice()).expect("decode fuzz provider event");
        if is_terminal(event.message_type.as_str()) {
            terminal_count += 1;
            break;
        }
    }
    assert_eq!(
        terminal_count, 1,
        "valid FFI command did not reach one terminal event"
    );
    assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
});
